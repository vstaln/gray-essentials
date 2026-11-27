mod core;
mod family;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::host::Host;
use crate::settings;
pub use core::ApprovalGate;
use core::{
    APPROVAL_QUESTION_ID, LABEL_DENY, MODE_FULL, Outcome, Scope, approval_question, call_label,
    decide_from_answers, host_is_headless, normalize_mode, permission_modes,
    run_permissions_command, save_rule,
};

pub const COMMANDS: &[&str] = &["/permissions", "/perms", "/access", "/confirm", "/gate"];

const DANGER_QUESTION_ID: &str = "confirm-dangerous";

const RISKY_SHELL_PATTERNS: &[&str] = &[
    r"\brm\s+(-[a-zA-Z]*r|--recursive)",
    r"\bsudo\b",
    r"\b(chmod|chown)\b[^|&;]*\b777\b",
    r"\bdd\b\s",
    r"\bmkfs[.\w-]*\b",
    r"\bshred\b",
    r">\s*/dev/",
    r"\bkill\s+-9\s+1\b",
    r"\bgit\s+push\b[^|&;]*(--force\b|-f\b)",
    r"\bgit\s+reset\b[^|&;]*--hard\b",
];

fn state_home() -> PathBuf {
    std::env::var_os("GRAY_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".gray")))
        .unwrap_or_else(|| PathBuf::from("."))
}

const ASK_TTL: Duration = Duration::from_secs(300);
const ASK_TTL_HEADLESS: Duration = Duration::from_secs(30);
const ASK_PROBE_TTL: Duration = Duration::from_secs(5);

const HOST_UNKNOWN: u8 = 0;
const HOST_INTERACTIVE: u8 = 1;
const HOST_HEADLESS: u8 = 2;
static HOST: AtomicU8 = AtomicU8::new(HOST_UNKNOWN);
static MODE_PINNED: AtomicBool = AtomicBool::new(false);

fn env_mode() -> Option<String> {
    ["GRAY_PERMISSION", "GRAY_PERMISSIONS"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.trim().is_empty()))
}

fn sync_persisted_mode(gate: &ApprovalGate) {
    if !MODE_PINNED.load(Ordering::Relaxed) {
        gate.set_mode(&settings::choice("permissions_mode"));
    }
}

pub fn new_gate() -> ApprovalGate {
    if let Some(mode) = env_mode() {
        MODE_PINNED.store(true, Ordering::Relaxed);
        return ApprovalGate::new(&mode);
    }
    ApprovalGate::new(&settings::choice("permissions_mode"))
}

pub fn note_host_event(params: &Value) {
    if params.get("type").and_then(Value::as_str) != Some("session_start") {
        return;
    }
    let interactive = params
        .get("interactive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    HOST.store(
        if interactive {
            HOST_INTERACTIVE
        } else {
            HOST_HEADLESS
        },
        Ordering::Relaxed,
    );
}

fn ask_ttl(host: &Host) -> Option<Duration> {
    match HOST.load(Ordering::Relaxed) {
        HOST_INTERACTIVE => Some(ASK_TTL),
        HOST_HEADLESS => Some(ASK_TTL_HEADLESS),
        _ => {
            let probe = host.call(
                "host/ask",
                json!({ "questions": [], "blocking": false }),
                ASK_PROBE_TTL,
            );
            let answered = match probe {
                Ok(_) => true,
                Err(ref e) => !e.contains("host did not answer"),
            };
            answered.then(|| {
                let _ = HOST.compare_exchange(
                    HOST_UNKNOWN,
                    HOST_INTERACTIVE,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
                ASK_TTL
            })
        }
    }
}

pub fn command(name: &str, gate: &ApprovalGate, argv: &[String]) -> Value {
    if name == "/confirm" {
        return json!({ "text": family::confirm_command(argv) });
    }
    if name == "/gate" {
        return json!({ "text": family::gate_command(argv) });
    }
    if argv.is_empty() {
        sync_persisted_mode(gate);
        let items: Vec<Value> = permission_modes()
            .into_iter()
            .map(|(id, label, desc)| json!({ "id": id, "label": label, "description": desc }))
            .collect();
        return json!({ "picker": {
            "title": "Permissions (all sessions)",
            "items": items,
            "selected_id": gate.mode(),
        }});
    }
    let chosen = match argv {
        [flag, .., pick] if flag == "--pick" => Some(pick.as_str()),
        [mode, ..] => Some(mode.as_str()),
        [] => None,
    };
    if let Some(clean) = chosen.and_then(normalize_mode) {
        MODE_PINNED.store(false, Ordering::Relaxed);
        gate.set_mode(clean);
        if let Err(e) = settings::set_choice("permissions_mode", clean) {
            eprintln!("permissions: could not persist mode ({e})");
            return json!({ "text": format!("permissions: {clean} (this session only: {e})") });
        }
        return json!({ "text": format!("permissions: {clean} (saved)") });
    }
    json!({ "text": run_permissions_command(gate, argv) })
}

fn risky_command(gate: &ApprovalGate, tool: &str, args: &Value) -> Option<String> {
    if tool != "bash" || family::bypasses(&gate.mode()) {
        return None;
    }
    let command = args.get("command").and_then(Value::as_str)?;
    family::risky(command)
}

fn confirm_risky(host: &Host, tool: &str, args: &Value, matched: &str) -> Value {
    let cfg = family::load_config();
    let command = args.get("command").and_then(Value::as_str).unwrap_or("");
    let ttl = if cfg.timeout_secs() == 0 {
        ASK_TTL
    } else {
        Duration::from_secs(cfg.timeout_secs())
    };
    let reply = host.call(
        "host/ask",
        json!({
            "questions": [family::dangerous_question(command, matched)],
            "blocking": true,
        }),
        ttl,
    );
    match reply
        .ok()
        .as_ref()
        .and_then(family::dangerous_answer_allows)
    {
        Some(true) => json!({"decision": "allow"}),
        Some(false) => json!({
            "decision": "deny",
            "reason": "dangerous command declined by user — do NOT retry via a workaround; ask the user instead"
        }),
        None if cfg.default_allow() => json!({"decision": "allow"}),
        None => json!({
            "decision": "deny",
            "reason": format!("no confirmation channel or timed out ({tool}: dangerous commands require confirmation)")
        }),
    }
}

pub fn tool_before(host: &Host, gate: &ApprovalGate, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("args").cloned().unwrap_or_else(|| json!({}));
    let cwd = params
        .get("session")
        .and_then(|s| s.get("cwd"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if name.is_empty() {
        return json!({"decision": "deny", "reason": "empty tool name"});
    }
    if let Some(reason) = family::denied(name) {
        return json!({"decision": "deny", "reason": reason});
    }
    sync_persisted_mode(gate);
    let risky = risky_command(gate, name, &args);
    match gate.check_cached(name, &args, &cwd) {
        Ok(true) if risky.is_none() => return json!({"decision": "allow"}),
        Err(reason) => return json!({"decision": "deny", "reason": reason}),
        _ => {}
    }
    if let Some(matched) = risky {
        return confirm_risky(host, name, &args, &matched);
    }
    static HEADLESS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *HEADLESS.get_or_init(host_is_headless) {
        return json!({"decision": "allow"});
    }
    let Some(ttl) = ask_ttl(host) else {
        return json!({"decision": "deny", "reason": "approval prompt unavailable"});
    };
    let label = call_label(name, &args);
    let q = approval_question(name, &label, &cwd);
    let picked: Vec<String> = match host.call(
        "host/ask",
        json!({ "questions": [q], "blocking": true }),
        ttl,
    ) {
        Ok(result) => result
            .get("answers")
            .and_then(|a| a.get(APPROVAL_QUESTION_ID))
            .and_then(|e| e.get("answers"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        Err(err) => {
            eprintln!("permissions: host/ask failed: {err}");
            if err.contains("ask service not installed") {
                return json!({"decision": "allow"});
            }
            if err.contains("host did not answer") {
                return json!({"decision": "deny", "reason": "approval timed out"});
            }
            return json!({"decision": "deny", "reason": "approval prompt unavailable"});
        }
    };
    match decide_from_answers(&picked) {
        Outcome::Allow => json!({"decision": "allow"}),
        Outcome::AllowSession => {
            gate.remember(name, &args, &cwd);
            json!({"decision": "allow"})
        }
        o @ (Outcome::AllowProject | Outcome::AllowAlways) => {
            let scope = if matches!(o, Outcome::AllowProject) {
                Scope::Project
            } else {
                Scope::User
            };
            if let Err(e) = save_rule(scope, name, &args, &cwd) {
                eprintln!(
                    "permissions: could not persist rule ({e}); remembered for session instead"
                );
                gate.remember(name, &args, &cwd);
            }
            json!({"decision": "allow"})
        }
        Outcome::Bypass => {
            MODE_PINNED.store(true, Ordering::Relaxed);
            gate.set_mode(MODE_FULL);
            json!({"decision": "allow"})
        }
        Outcome::BypassAlways => {
            MODE_PINNED.store(false, Ordering::Relaxed);
            gate.set_mode(MODE_FULL);
            if let Err(e) = settings::set_choice("permissions_mode", MODE_FULL) {
                eprintln!("permissions: could not persist bypass mode ({e})");
                MODE_PINNED.store(true, Ordering::Relaxed);
            }
            json!({"decision": "allow"})
        }
        Outcome::Edit(text) => {
            if name != "bash" {
                return json!({
                    "decision": "deny",
                    "reason": format!("edited arguments unsupported for {name}"),
                });
            }
            let mut rewritten = args.clone();
            rewritten["command"] = json!(text);
            json!({"decision": "modify", "args": rewritten})
        }
        Outcome::DenyWith(note) => json!({
            "decision": "deny",
            "reason": format!("{name} declined by user: {note}"),
        }),
        Outcome::Reject => json!({
            "decision": "cancel",
            "reason": format!("{name} declined by user ({LABEL_DENY}) — turn stopped"),
        }),
    }
}
