use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::host::Host;
use crate::paths;
use crate::prompt;
use crate::settings;

const PRUNE_STAMP: &str = ".last-prune";
const SIDECARS: [&str; 4] = ["dismissed", "title", "autotitle", "prefix"];

fn root() -> PathBuf {
    paths::gray_home().join("sessions")
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn session_start(host: &Host, params: &Value) {
    if !prompt::interactive(params) {
        return;
    }
    cleanup(host);
    hint(host, params);
}

fn cleanup(host: &Host) {
    let Some(days) = settings::cleanup_days() else {
        return;
    };
    let root = root();
    let stamp = root.join(PRUNE_STAMP);
    let fresh = std::fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < Duration::from_secs(86_400));
    if fresh {
        return;
    }
    let _ = std::fs::write(&stamp, b"");
    let cutoff = paths::now().saturating_sub(u64::from(days) * 86_400);
    for session in host.sessions() {
        if session.get("running").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(id) = session.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !valid_id(id) {
            continue;
        }
        let last = session
            .get("last_active")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if last == 0 || last >= cutoff {
            continue;
        }
        remove(&root, id);
    }
}

fn remove(root: &Path, id: &str) {
    let _ = std::fs::remove_file(root.join(format!("{id}.jsonl")));
    for ext in SIDECARS {
        let _ = std::fs::remove_file(root.join(format!("{id}.{ext}")));
    }
}

fn interrupted_here(host: &Host, cwd: &Path) -> Vec<String> {
    let here = canonical(cwd);
    host.sessions()
        .into_iter()
        .filter(|s| {
            s.get("running").and_then(Value::as_bool) != Some(true)
                && s.get("interrupted").and_then(Value::as_bool) == Some(true)
        })
        .filter(|s| {
            s.get("cwd")
                .and_then(Value::as_str)
                .map(|c| canonical(Path::new(c)) == here)
                .unwrap_or(false)
        })
        .filter_map(|s| s.get("id").and_then(Value::as_str).map(str::to_string))
        .filter(|id| valid_id(id))
        .collect()
}

fn hint(host: &Host, params: &Value) {
    let Some(cwd) = prompt::cwd(params) else {
        return;
    };
    let n = interrupted_here(host, &cwd).len();
    if n == 0 {
        return;
    }
    let noun = if n == 1 { "session" } else { "sessions" };
    host.notice(
        &format!(
            "\u{26a0} {n} interrupted {noun} in {} \u{b7} /sessions to pick up",
            paths::tilde(&cwd)
        ),
        "/qol sessions dismiss to clear",
        "home",
        "qol.interrupted",
    );
}

pub fn dismiss_command(host: &Host, params: &Value) -> Value {
    let Some(cwd) = prompt::cwd(params) else {
        return json!({"text": "no session directory"});
    };
    let root = root();
    let mut dismissed = 0;
    for id in interrupted_here(host, &cwd) {
        if let Some(at) = last_message_at(&root, &id)
            && std::fs::write(root.join(format!("{id}.dismissed")), at.to_string()).is_ok()
        {
            dismissed += 1;
        }
    }
    let plural = if dismissed == 1 { "" } else { "s" };
    json!({"text": format!("dismissed {dismissed} interrupted session{plural}")})
}

fn last_message_at(root: &Path, id: &str) -> Option<u64> {
    let path = root.join(format!("{id}.jsonl"));
    let text = std::fs::read_to_string(&path).ok();
    let from_entries = text.and_then(|text| {
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| v.get("timestamp").and_then(Value::as_u64))
            .max()
    });
    from_entries.or_else(|| {
        std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
    })
}
