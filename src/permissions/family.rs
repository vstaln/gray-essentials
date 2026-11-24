use std::path::PathBuf;

use serde_json::{Value, json};

use super::core::MODE_FULL;

fn state_dir() -> PathBuf {
    super::state_home().join("confirm")
}

fn enabled() -> bool {
    !state_dir().join("disabled").exists()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConfirmConfig {
    timeout_secs: u64,
    default_allow: bool,
}

pub(super) fn load_config() -> ConfirmConfig {
    let mut cfg = ConfirmConfig {
        timeout_secs: 0,
        default_allow: false,
    };
    if let Ok(raw) = std::fs::read_to_string(state_dir().join("config.json"))
        && let Ok(v) = serde_json::from_str::<Value>(&raw)
    {
        if let Some(n) = v.get("timeout_secs").and_then(Value::as_u64) {
            cfg.timeout_secs = n;
        }
        if v.get("default").and_then(Value::as_str) == Some("allow") {
            cfg.default_allow = true;
        }
    }
    if let Ok(s) = std::env::var("GRAY_CONFIRM_TIMEOUT_SECS")
        && let Ok(n) = s.trim().parse::<u64>()
    {
        cfg.timeout_secs = n;
    }
    cfg
}

impl ConfirmConfig {
    pub(super) fn timeout_secs(&self) -> u64 {
        self.timeout_secs
    }

    pub(super) fn default_allow(&self) -> bool {
        self.default_allow
    }
}

pub(super) fn patterns() -> (Vec<regex::Regex>, Vec<String>) {
    let mut out = Vec::new();
    for p in super::RISKY_SHELL_PATTERNS {
        if let Ok(r) = regex::Regex::new(p) {
            out.push(r);
        }
    }
    let mut bad = Vec::new();
    if let Ok(raw) = std::fs::read_to_string(state_dir().join("patterns.json"))
        && let Ok(v) = serde_json::from_str::<Value>(&raw)
        && let Some(list) = v.get("patterns").and_then(Value::as_array)
    {
        for item in list {
            let Some(s) = item.as_str() else { continue };
            match regex::Regex::new(s) {
                Ok(r) => out.push(r),
                Err(_) => bad.push(s.to_string()),
            }
        }
    }
    (out, bad)
}

pub(super) fn risky(command: &str) -> Option<String> {
    if !enabled() {
        return None;
    }
    COMPILED
        .iter()
        .find(|p| p.is_match(command))
        .map(|p| p.as_str().to_string())
}

static COMPILED: std::sync::LazyLock<Vec<regex::Regex>> = std::sync::LazyLock::new(|| patterns().0);

pub(super) fn set_enabled(on: bool) -> Result<(), String> {
    let dir = state_dir();
    let flag = dir.join("disabled");
    let res = if on {
        std::fs::remove_file(&flag).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(e)
            }
        })
    } else {
        std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&flag, b""))
    };
    res.map_err(|e| format!("couldn't flip state: {e}"))
}

pub(super) fn summary() -> String {
    let (patterns, bad) = patterns();
    let cfg = load_config();
    let mut out = format!(
        "dangerous commands: {} patterns · {} · timeout {}s (0 = host default, times out {})",
        patterns.len(),
        if enabled() { "on" } else { "off" },
        cfg.timeout_secs,
        if cfg.default_allow { "allow" } else { "deny" },
    );
    for b in bad {
        out.push_str(&format!("\n  (skipped, invalid regex) {b}"));
    }
    out
}

pub(super) fn list() -> String {
    let (patterns, bad) = patterns();
    let mut lines = vec![format!("{} patterns active:", patterns.len())];
    for p in &patterns {
        lines.push(format!("  {p}"));
    }
    for b in bad {
        lines.push(format!("  (skipped, invalid regex) {b}"));
    }
    lines.join("\n")
}

pub(super) fn bypasses(mode: &str) -> bool {
    mode == MODE_FULL
}

pub(super) fn deny_file() -> PathBuf {
    super::state_home().join("tool-gate").join("deny.txt")
}

pub(super) fn deny_list() -> Vec<String> {
    match std::fs::read_to_string(deny_file()) {
        Ok(s) => s
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect(),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("permissions: can't read {}: {e}", deny_file().display());
            }
            Vec::new()
        }
    }
}

pub(super) fn write_deny_list(list: &[String]) -> std::io::Result<()> {
    let dir = super::state_home().join("tool-gate");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        deny_file(),
        list.join("\n") + if list.is_empty() { "" } else { "\n" },
    )
}

fn glob_match(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let s: Vec<char> = s.chars().collect();
    let (mut pi, mut si) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub(super) fn denied(tool: &str) -> Option<String> {
    for pat in deny_list() {
        if glob_match(&pat, tool) {
            return Some(format!(
                "{tool} denied by tool-gate (/gate allow {pat} to re-enable)"
            ));
        }
    }
    None
}

pub(super) fn gate_command(argv: &[String]) -> String {
    let words: Vec<&str> = argv.iter().map(String::as_str).collect();
    match words.first().copied() {
        Some("deny") => match words.get(1) {
            Some(pat) => {
                let mut list = deny_list();
                if list.iter().any(|p| p == pat) {
                    format!("{pat} already denied")
                } else {
                    list.push(pat.to_string());
                    match write_deny_list(&list) {
                        Ok(()) => format!("denied {pat}"),
                        Err(e) => format!("couldn't write {}: {e}", deny_file().display()),
                    }
                }
            }
            None => "usage: /gate deny <tool-or-glob>".into(),
        },
        Some("allow") => match words.get(1) {
            Some(pat) => {
                let mut list = deny_list();
                let before = list.len();
                list.retain(|p| p != pat);
                if list.len() == before {
                    format!("{pat} wasn't denied")
                } else {
                    match write_deny_list(&list) {
                        Ok(()) => format!("allowed {pat}"),
                        Err(e) => format!("couldn't write {}: {e}", deny_file().display()),
                    }
                }
            }
            None => "usage: /gate allow <tool-or-glob>".into(),
        },
        Some("list") => {
            let list = deny_list();
            if list.is_empty() {
                "no denied tools".into()
            } else {
                format!("denied: {}", list.join(", "))
            }
        }
        Some("clear") => match write_deny_list(&[]) {
            Ok(()) => "deny list cleared".into(),
            Err(e) => format!("couldn't write {}: {e}", deny_file().display()),
        },
        _ => format!(
            "persisted tool deny-list in {}. \
             /gate deny <pat> | allow <pat> | list | clear (globs: *, ?)",
            deny_file().display()
        ),
    }
}

pub(super) fn confirm_command(argv: &[String]) -> String {
    match argv.first().map(String::as_str) {
        Some("list") => list(),
        Some("on") | Some("off") => {
            let on = argv[0] == "on";
            match set_enabled(on) {
                Ok(()) => format!("confirm gate {}", if on { "on" } else { "off" }),
                Err(e) => e,
            }
        }
        _ => summary(),
    }
}

pub(super) fn dangerous_question(command: &str, matched: &str) -> Value {
    json!({
        "id": super::DANGER_QUESTION_ID,
        "header": "Dangerous command",
        "question": format!("⚠️ Dangerous command (matched `{matched}`):\n\n  {command}\n\nRun it?"),
        "options": [
            {"label": "Yes", "description": "run the command once"},
            {"label": "No", "description": "block it"},
        ],
    })
}

pub(super) fn dangerous_answer_allows(result: &Value) -> Option<bool> {
    if result.get("error").is_some() {
        return None;
    }
    let picked: Vec<&str> = result
        .get("answers")
        .and_then(|a| a.get(super::DANGER_QUESTION_ID))
        .and_then(|e| e.get("answers"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    match picked.first() {
        Some(&"Yes") => Some(true),
        Some(_) => Some(false),
        None => None,
    }
}
