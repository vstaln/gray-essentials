use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::host::Host;
use crate::prompt;
use crate::settings;

const STREAK_IDLE_RESET: Duration = Duration::from_secs(15 * 60);
const MESSAGE_MAX_CHARS: usize = 100;

struct Streak {
    start: Option<Instant>,
    last_end: Option<Instant>,
    reminded: bool,
}

static STREAK: Mutex<Streak> = Mutex::new(Streak {
    start: None,
    last_end: None,
    reminded: false,
});

pub fn turn_end(host: &Host, params: &Value) {
    if !prompt::interactive(params) {
        return;
    }
    let failed = params.get("error").is_some_and(|e| !e.is_null());
    break_reminder(host);
    if !settings::on("notify_on_finish") {
        return;
    }
    let body = match failed {
        true => "turn failed".to_string(),
        false => turn_message(host),
    };
    host.notify("gray", &body);
}

fn break_reminder(host: &Host) {
    let Some(minutes) = settings::break_minutes() else {
        return;
    };
    let now = Instant::now();
    let remind = {
        let mut st = STREAK.lock().unwrap_or_else(|e| e.into_inner());
        let broken = st
            .last_end
            .is_some_and(|end| now.saturating_duration_since(end) > STREAK_IDLE_RESET);
        if st.start.is_none() || broken {
            st.start = Some(now);
            st.reminded = false;
        }
        st.last_end = Some(now);
        let streak_minutes = st
            .start
            .map(|start| now.saturating_duration_since(start).as_secs() / 60)
            .unwrap_or(0);
        let remind = !st.reminded && streak_minutes >= u64::from(minutes);
        if remind {
            st.reminded = true;
        }
        remind.then_some(minutes)
    };
    if let Some(minutes) = remind {
        host.notify(
            "gray",
            &format!("You've been working for {minutes} min. Time for a short break?"),
        );
    }
}

fn turn_message(host: &Host) -> String {
    let messages = prompt::messages(host, 1);
    let line = prompt::last_text(&messages, "assistant")
        .and_then(|text| {
            text.lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(str::to_string)
        })
        .map(|l| {
            l.chars()
                .filter(|c| !c.is_control())
                .take(MESSAGE_MAX_CHARS)
                .collect::<String>()
        })
        .filter(|l| !l.is_empty());
    line.unwrap_or_else(|| "turn finished".to_string())
}
