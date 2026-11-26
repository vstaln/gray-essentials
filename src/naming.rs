use serde_json::{Value, json};

use crate::host::Host;
use crate::prompt;

pub const TOOL_NAME: &str = "session_rename";

const MAX_CHARS: usize = 48;
const HASH_LEN: usize = 4;
const MAX_WORDS: usize = 4;
const BUDGET: usize = MAX_CHARS - HASH_LEN - 1;
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "at", "by", "for", "from", "in", "into", "is", "it", "of", "on", "or", "the",
    "to", "with",
];

pub fn tool_def() -> Value {
    json!({
        "name": TOOL_NAME,
        "label": "Session rename",
        "preview": "{name}",
        "description": "Rename this session to fit its current work, 2-4 words, the way you would name a subagent. The stable hash suffix is kept.",
        "parameters": {
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The session's topic in 2-4 words, e.g. \"acp usage cache\"."
                }
            },
            "required": ["name"]
        }
    })
}

pub fn build(topic: &str, session: &str) -> String {
    let mut words = split_words(topic);
    let kept: Vec<String> = words
        .iter()
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .cloned()
        .collect();
    if !kept.is_empty() {
        words = kept;
    }
    let mut slug = String::new();
    for word in words.iter().take(MAX_WORDS) {
        let next = if slug.is_empty() {
            word.clone()
        } else {
            format!("{slug}-{word}")
        };
        if next.len() > BUDGET {
            break;
        }
        slug = next;
    }
    if slug.is_empty() {
        slug = words
            .first()
            .map(|w| w.chars().take(BUDGET).collect())
            .unwrap_or_else(|| "session".to_string());
    }
    format!("{slug}-{}", hash(session))
}

pub fn hash(session: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in session.as_bytes() {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")[..HASH_LEN].to_string()
}

fn split_words(topic: &str) -> Vec<String> {
    topic
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn tool_call(host: &Host, params: &Value) -> Value {
    let topic = params
        .get("args")
        .and_then(|a| a.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if split_words(topic).is_empty() {
        return json!({
            "content": "session_rename: a name of 2-4 words is required",
            "is_error": true,
        });
    }
    let name = build(topic, &prompt::session_id(params));
    match host.session_name(&name) {
        Ok(v) if v.get("ok").and_then(Value::as_bool) == Some(false) => json!({
            "content": "session_rename: the host did not accept the name",
            "is_error": true,
        }),
        Ok(_) => json!({"content": name}),
        Err(err) => json!({
            "content": format!("session_rename: {err}"),
            "is_error": true,
        }),
    }
}
