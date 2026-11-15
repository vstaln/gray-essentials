use std::sync::Mutex;

use serde_json::Value;

use crate::host::Host;
use crate::prompt;
use crate::settings;

const SYSTEM: &str = "The user stepped away from this coding session and just came back. \
In one or two short sentences, remind them what they were doing and what comes next. \
Plain text only, no preamble.";
const EXCERPT_CHARS: usize = 1500;
const MAX_TOKENS: u32 = 160;
const RECAP_AFTER_SECS: u64 = 180;

static LAST: Mutex<Option<(String, usize, String)>> = Mutex::new(None);

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
    let (count, tail_text) = prompt::signature(&messages);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last
            .as_ref()
            .is_some_and(|(s, c, t)| *s == session && *c == count && *t == tail_text)
        {
            return;
        }
        *last = Some((session, count, tail_text));
    }
    let Some(user) = prompt::last_text(&messages, "user") else {
        return;
    };
    let reply = prompt::last_text(&messages, "assistant").unwrap_or_default();
    let body = format!(
        "Last request:\n{}\n\nLast reply:\n{}\n\nRecap:",
        prompt::tail(&user, EXCERPT_CHARS),
        prompt::tail(&reply, EXCERPT_CHARS)
    );
    if let Some(text) = host.complete(SYSTEM, &body, MAX_TOKENS) {
        host.say_card(text.trim(), "Recap");
    }
}
