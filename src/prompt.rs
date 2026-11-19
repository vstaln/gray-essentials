use serde_json::Value;

use crate::host::Host;

pub const TRANSCRIPT_LAST: usize = 20;

pub fn session_id(params: &Value) -> String {
    params["session"]["id"].as_str().unwrap_or("").to_string()
}

pub fn cwd(params: &Value) -> Option<std::path::PathBuf> {
    params["session"]["cwd"]
        .as_str()
        .filter(|c| !c.trim().is_empty())
        .map(std::path::PathBuf::from)
}

pub fn interactive(params: &Value) -> bool {
    params
        .get("interactive")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

pub fn messages(host: &Host, last: usize) -> Vec<Value> {
    for attempt in 0..3 {
        match host.transcript(last) {
            Ok(messages) => return messages,
            Err(_) if attempt < 2 => {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            Err(_) => break,
        }
    }
    Vec::new()
}

pub fn text(message: &Value) -> String {
    message
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub fn last_text(messages: &[Value], role: &str) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|m| m.get("role").and_then(Value::as_str) == Some(role))
        .map(text)
        .find(|t| !t.trim().is_empty())
}

pub fn first_text(messages: &[Value], role: &str) -> Option<String> {
    messages
        .iter()
        .filter(|m| m.get("role").and_then(Value::as_str) == Some(role))
        .map(text)
        .find(|t| !t.trim().is_empty())
}

pub fn tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.trim().chars().collect();
    chars[chars.len().saturating_sub(n)..].iter().collect()
}

pub fn head(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}
