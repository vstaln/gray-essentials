use std::sync::Mutex;

use serde_json::Value;

use crate::host::Host;
use crate::prompt;
use crate::settings;

const SYSTEM: &str = "You name chat sessions. Reply with a short title of about six words \
that says what the session is about. Plain text only: no quotes, no trailing period, \
no preamble.";
const EXCERPT_CHARS: usize = 600;
const MAX_TOKENS: u32 = 64;
const MAX_WORDS: usize = 8;
const MAX_TITLE_CHARS: usize = 80;
const BACKGROUND_MARKER: &str = "[Background task notification]";

static CLAIMED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn turn_end(host: &Host, params: &Value) {
    if !prompt::interactive(params) || !settings::on("auto_titles") {
        return;
    }
    if origin_tag().is_some() {
        return;
    }
    let session = prompt::session_id(params);
    if claimed(&session) {
        return;
    }
    let messages = prompt::messages(host, prompt::TRANSCRIPT_LAST);
    let Some(first_user) = prompt::first_text(&messages, "user") else {
        return;
    };
    let Some(reply) = prompt::first_text(&messages, "assistant") else {
        return;
    };
    if opener_tag(&first_user).is_some() {
        return;
    }
    mark(&session);
    let body = format!(
        "First message:\n{}\n\nFirst reply:\n{}\n\nTitle for this session:",
        prompt::head(&first_user, EXCERPT_CHARS),
        prompt::head(&reply, EXCERPT_CHARS)
    );
    let Some(raw) = host.complete(SYSTEM, &body, MAX_TOKENS) else {
        return;
    };
    if let Some(title) = title_from_reply(&raw) {
        host.session_title(&title);
    }
}

fn claimed(session: &str) -> bool {
    CLAIMED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|s| s == session)
}

fn mark(session: &str) {
    CLAIMED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(session.to_string());
}

fn origin_tag() -> Option<String> {
    if let Ok(origin) = std::env::var("GRAY_SESSION_ORIGIN")
        && !origin.trim().is_empty()
    {
        return Some(origin.trim().to_ascii_lowercase());
    }
    (std::env::var("GRAY_SUBAGENTS_ACTIVE").as_deref() == Ok("1")).then(|| "subagent".to_string())
}

fn opener_tag(first: &str) -> Option<&'static str> {
    let first = first.trim_start();
    let head: String = first.chars().take(160).collect();
    if first.starts_with(BACKGROUND_MARKER) {
        Some("background")
    } else if head.contains("subagent of a parent Gray") {
        Some("subagent")
    } else if head.starts_with("You are") && head.contains(" worker") {
        Some("worker")
    } else {
        None
    }
}

fn title_from_reply(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = if line.to_ascii_lowercase().starts_with("title:") {
        &line[6..]
    } else {
        line
    };
    let line = line.trim_matches(|c: char| c.is_whitespace() || "\"'`*#_".contains(c));
    let line = line.trim_end_matches(|c: char| ".!?:;".contains(c));
    let words: Vec<&str> = line.split_whitespace().take(MAX_WORDS).collect();
    clean_title(&words.join(" "))
}

fn clean_title(raw: &str) -> Option<String> {
    let one: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = one.chars().take(MAX_TITLE_CHARS).collect();
    (!t.is_empty()).then_some(t)
}
