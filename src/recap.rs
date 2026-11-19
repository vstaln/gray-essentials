use std::path::PathBuf;

use serde_json::Value;

use crate::host::Host;
use crate::prompt;
use crate::settings;

const SYSTEM: &str = "The user stepped away from this coding session and just came back. \
In one or two short sentences, remind them what they were doing and what comes next. \
Plain text only, no preamble.";
const EXCERPT_CHARS: usize = 1500;
const MAX_TOKENS: u32 = 160;
const RECAP_AFTER_SECS: u64 = 60;

fn last_path(session: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    let name: String = session
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = if name.is_empty() {
        "none".to_string()
    } else {
        name
    };
    Some(base.join("gray-qol").join("recap").join(name))
}

fn recap_after() -> u64 {
    std::env::var("QOL_RECAP_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(RECAP_AFTER_SECS)
}

pub fn user_return(host: &Host, params: &Value) {
    let away = params.get("away_secs").and_then(Value::as_u64).unwrap_or(0);
    if away < recap_after() || !settings::on("away_recap") {
        return;
    }
    let session = prompt::session_id(params);
    let messages = prompt::messages(host, prompt::TRANSCRIPT_LAST);
    let Some(user) = prompt::last_text(&messages, "user") else {
        return;
    };
    let reply = prompt::last_text(&messages, "assistant").unwrap_or_default();
    let key = serde_json::json!([user, reply]).to_string();
    let path = last_path(&session);
    if let Some(path) = &path {
        if std::fs::read_to_string(path).is_ok_and(|prev| prev == key) {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, &key);
    }
    let body = format!(
        "Last request:\n{}\n\nLast reply:\n{}\n\nRecap:",
        prompt::tail(&user, EXCERPT_CHARS),
        prompt::tail(&reply, EXCERPT_CHARS)
    );
    if let Some(text) = host.complete(SYSTEM, &body, MAX_TOKENS) {
        host.say_card(text.trim(), "Recap");
    }
}
