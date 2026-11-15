use serde_json::Value;

use crate::host::Host;
use crate::prompt;
use crate::settings;

const SYSTEM: &str = "Predict the user's most likely next message to this coding assistant. \
Reply with only that message, at most 12 words, no quotes.";
const EXCERPT_CHARS: usize = 800;
const MAX_TOKENS: u32 = 320;
const MAX_WORDS: usize = 12;

pub fn turn_end(host: &Host, params: &Value) {
    if !prompt::interactive(params) || !settings::on("prompt_suggestions") {
        return;
    }
    let messages = prompt::messages(host, prompt::TRANSCRIPT_LAST);
    let Some(reply) = prompt::last_text(&messages, "assistant") else {
        return;
    };
    let user = prompt::last_text(&messages, "user").unwrap_or_default();
    let body = format!(
        "Last request:\n{}\n\nLast reply:\n{}\n\nNext message:",
        prompt::tail(&user, EXCERPT_CHARS),
        prompt::tail(&reply, EXCERPT_CHARS)
    );
    let Some(raw) = host.complete(SYSTEM, &body, MAX_TOKENS) else {
        return;
    };
    if let Some(text) = clean_reply(&raw) {
        host.ghost(&text);
    }
}

fn clean_reply(raw: &str) -> Option<String> {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())?
        .trim_matches(|c| matches!(c, '"' | '\'' | '`' | '\u{201c}' | '\u{201d}'))
        .trim()
        .to_string();
    if line.is_empty() {
        return None;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let capped = words
        .iter()
        .take(MAX_WORDS)
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    (!capped.trim().is_empty()).then_some(capped)
}
