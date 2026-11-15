use serde_json::{Value, json};

use crate::paths;

pub const NAME: &str = "quality-of-life";

enum Kind {
    Bool(bool),
    Choice(&'static [&'static str], &'static str),
}

struct Row {
    key: &'static str,
    label: &'static str,
    kind: Kind,
}

const CLEANUP: &[&str] = &["never", "7 days", "30 days", "90 days", "365 days"];
const BREAK: &[&str] = &["never", "30 min", "60 min", "90 min", "120 min"];

const ROWS: &[Row] = &[
    Row {
        key: "away_recap",
        label: "Recap when you come back after 5+ min",
        kind: Kind::Bool(true),
    },
    Row {
        key: "prompt_suggestions",
        label: "Suggest the next prompt",
        kind: Kind::Bool(false),
    },
    Row {
        key: "auto_titles",
        label: "Name sessions automatically",
        kind: Kind::Bool(true),
    },
    Row {
        key: "issue_links",
        label: "Link #123 to the repo's issues and PRs",
        kind: Kind::Bool(true),
    },
    Row {
        key: "session_cleanup",
        label: "Delete sessions idle longer than",
        kind: Kind::Choice(CLEANUP, "never"),
    },
    Row {
        key: "auto_continue",
        label: "Continue after usage limit resets",
        kind: Kind::Bool(true),
    },
    Row {
        key: "notify_on_finish",
        label: "Notify when a turn finishes while you're away",
        kind: Kind::Bool(true),
    },
    Row {
        key: "break_reminder",
        label: "Break reminder after",
        kind: Kind::Choice(BREAK, "never"),
    },
];

pub fn manifest_rows() -> Value {
    Value::Array(
        ROWS.iter()
            .map(|r| match &r.kind {
                Kind::Bool(default) => {
                    json!({"key": r.key, "label": r.label, "type": "bool", "default": default})
                }
                Kind::Choice(choices, default) => json!({
                    "key": r.key,
                    "label": r.label,
                    "type": "choice",
                    "choices": choices,
                    "default": default,
                }),
            })
            .collect(),
    )
}

fn stored(key: &str) -> Option<Value> {
    let path = paths::gray_home()
        .join("plugin-settings")
        .join(format!("{NAME}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get(key).cloned()
}

fn declare(key: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.key == key)
}

pub fn on(key: &str) -> bool {
    if let Some(Value::Bool(b)) = stored(key) {
        return b;
    }
    match declare(key).map(|r| &r.kind) {
        Some(Kind::Bool(default)) => *default,
        _ => false,
    }
}

pub fn choice(key: &str) -> String {
    if let Some(Value::String(s)) = stored(key) {
        return s;
    }
    match declare(key).map(|r| &r.kind) {
        Some(Kind::Choice(_, default)) => (*default).to_string(),
        _ => String::new(),
    }
}

fn leading_number(s: &str) -> Option<u32> {
    let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|n| *n > 0)
}

pub fn cleanup_days() -> Option<u32> {
    leading_number(&choice("session_cleanup"))
}

pub fn break_minutes() -> Option<u32> {
    leading_number(&choice("break_reminder"))
}
