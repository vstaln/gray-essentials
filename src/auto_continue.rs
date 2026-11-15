use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::host::Host;
use crate::settings;

const CONTINUE_AFTER_LIMIT: &str =
    "Your usage limit has reset. Continue the task where you left off.";
const CONTINUE_GRACE: u64 = 30;

static TURN_GEN: AtomicU64 = AtomicU64::new(0);

pub fn turn_end(host: &Arc<Host>, params: &Value) {
    let armed = TURN_GEN.fetch_add(1, Ordering::Relaxed);
    let error = params.get("error").cloned().unwrap_or(Value::Null);
    if error.get("kind").and_then(Value::as_str) != Some("usage_limit") {
        return;
    }
    if !settings::on("auto_continue") {
        return;
    }
    let Some(resets_at) = error.get("resets_at").and_then(Value::as_u64) else {
        return;
    };
    let wait = resets_at.saturating_sub(crate::paths::now()) + CONTINUE_GRACE;
    host.say(&format!(
        "\u{b7} resuming in {} \u{b7} usage limit reset",
        span(wait)
    ));
    let host = Arc::clone(host);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(wait));
        if TURN_GEN.load(Ordering::Relaxed) != armed {
            return;
        }
        host.inject(CONTINUE_AFTER_LIMIT);
    });
}

fn span(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, secs % 3600 / 60)
    } else {
        format!("{}m", secs.div_ceil(60))
    }
}
