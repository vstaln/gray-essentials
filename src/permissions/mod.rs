mod core;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::host::Host;
pub use core::ApprovalGate;
use core::{
    APPROVAL_QUESTION_ID, LABEL_DENY, MODE_AUTO, MODE_FULL, Outcome, Scope, approval_question,
    call_label, decide_from_answers, host_is_headless, run_permissions_command, save_rule,
};

pub const COMMANDS: &[&str] = core::COMMANDS;

const ASK_TTL: Duration = Duration::from_secs(300);
const ASK_TTL_HEADLESS: Duration = Duration::from_secs(30);
const ASK_PROBE_TTL: Duration = Duration::from_secs(5);

const HOST_UNKNOWN: u8 = 0;
const HOST_INTERACTIVE: u8 = 1;
const HOST_HEADLESS: u8 = 2;
static HOST: AtomicU8 = AtomicU8::new(HOST_UNKNOWN);

pub fn new_gate() -> ApprovalGate {
    let mode = ["GRAY_PERMISSION", "GRAY_PERMISSIONS"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| MODE_AUTO.to_string());
    ApprovalGate::new(&mode)
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

pub fn command(gate: &ApprovalGate, argv: &[String]) -> Value {
    json!({ "text": run_permissions_command(gate, argv) })
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
    match gate.check_cached(name, &args, &cwd) {
        Ok(true) => return json!({"decision": "allow"}),
        Err(reason) => return json!({"decision": "deny", "reason": reason}),
        Ok(false) => {}
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
            gate.set_mode(MODE_FULL);
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
