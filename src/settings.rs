use serde_json::{Value, json};

use crate::paths;

pub const NAME: &str = "extras";
const LEGACY_NAME: &str = "quality-of-life";

enum Kind {
    Bool(bool),
    Choice(&'static [&'static str], &'static str),
}

struct Row {
    key: &'static str,
    label: &'static str,
    short: &'static str,
    kind: Kind,
}

const CLEANUP: &[&str] = &["never", "7 days", "30 days", "90 days", "365 days"];
const BREAK: &[&str] = &["never", "30 min", "60 min", "90 min", "120 min"];
const PERMISSION_MODES: &[&str] = &["auto", "full", "read-only"];

const ROWS: &[Row] = &[
    Row {
        key: "away_recap",
        label: "Recap when you come back after 1+ min",
        short: "Recap on return",
        kind: Kind::Bool(true),
    },
    Row {
        key: "prompt_suggestions",
        label: "Suggest the next prompt",
        short: "Prompt suggestions",
        kind: Kind::Bool(false),
    },
    Row {
        key: "auto_titles",
        label: "Name sessions automatically",
        short: "Auto titles",
        kind: Kind::Bool(true),
    },
    Row {
        key: "issue_links",
        label: "Link #123 to the repo's issues and PRs",
        short: "Issue links",
        kind: Kind::Bool(true),
    },
    Row {
        key: "session_cleanup",
        label: "Delete sessions idle longer than",
        short: "Prune idle sessions",
        kind: Kind::Choice(CLEANUP, "never"),
    },
    Row {
        key: "auto_continue",
        label: "Continue after usage limit resets",
        short: "Continue after limit",
        kind: Kind::Bool(true),
    },
    Row {
        key: "usage_warnings",
        label: "Warn at 75%, 90% and 95% of a usage limit",
        short: "Usage warnings",
        kind: Kind::Bool(true),
    },
    Row {
        key: "notify_on_finish",
        label: "Notify when a turn finishes while you're away",
        short: "Notify when done",
        kind: Kind::Bool(true),
    },
    Row {
        key: "break_reminder",
        label: "Break reminder after",
        short: "Break reminder",
        kind: Kind::Choice(BREAK, "never"),
    },
    Row {
        key: "vim_mode",
        label: "Vim keybindings in the input box",
        short: "Vim mode",
        kind: Kind::Bool(false),
    },
    Row {
        key: "permissions_mode",
        label: "Tool approval mode (full = always bypass)",
        short: "Permissions mode",
        kind: Kind::Choice(PERMISSION_MODES, "auto"),
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

fn store_path() -> std::path::PathBuf {
    paths::gray_home()
        .join("plugin-settings")
        .join(format!("{NAME}.json"))
}

fn stored_all() -> serde_json::Map<String, Value> {
    let dir = paths::gray_home().join("plugin-settings");
    std::fs::read_to_string(dir.join(format!("{NAME}.json")))
        .or_else(|_| std::fs::read_to_string(dir.join(format!("{LEGACY_NAME}.json"))))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn stored(key: &str) -> Option<Value> {
    stored_all().get(key).cloned()
}

pub struct PanelRow {
    pub key: &'static str,
    pub label: &'static str,
    pub value: String,
}

pub fn panel_rows() -> Vec<PanelRow> {
    ROWS.iter()
        .map(|r| PanelRow {
            key: r.key,
            label: r.short,
            value: match &r.kind {
                Kind::Bool(_) if on(r.key) => "\u{2713} on".to_string(),
                Kind::Bool(_) => "off".to_string(),
                Kind::Choice(..) => match choice(r.key) {
                    c if c == "never" => "off".to_string(),
                    c => c,
                },
            },
        })
        .collect()
}

pub fn step(key: &str) -> std::io::Result<()> {
    let Some(row) = declare(key) else {
        return Err(std::io::Error::other(format!("unknown setting {key:?}")));
    };
    let next = match &row.kind {
        Kind::Bool(_) => Value::Bool(!on(key)),
        Kind::Choice(choices, _) => {
            let cur = choice(key);
            let i = choices.iter().position(|c| *c == cur).map_or(0, |i| i + 1);
            Value::String(choices[i % choices.len()].to_string())
        }
    };
    write_value(key, next)
}

pub fn set_choice(key: &str, value: &str) -> std::io::Result<()> {
    let Some(row) = declare(key) else {
        return Err(std::io::Error::other(format!("unknown setting {key:?}")));
    };
    let Kind::Choice(choices, _) = &row.kind else {
        return Err(std::io::Error::other(format!(
            "setting {key:?} is not a choice"
        )));
    };
    if !choices.contains(&value) {
        return Err(std::io::Error::other(format!(
            "invalid value {value:?} for {key:?}"
        )));
    }
    write_value(key, Value::String(value.to_string()))
}

fn write_value(key: &str, value: Value) -> std::io::Result<()> {
    let mut values = stored_all();
    values.insert(key.to_string(), value);
    let text =
        serde_json::to_string_pretty(&Value::Object(values)).map_err(std::io::Error::other)?;
    let path = store_path();
    paths::write_atomic(&path, &text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
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
