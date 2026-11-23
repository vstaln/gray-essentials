use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Map, Value, json};

use super::util;

const SERVICE: &str = "gray-secrets";

pub fn lookup(name: &str) -> Option<String> {
    let out = Command::new("secret-tool")
        .args(["lookup", "service", SERVICE, "name", name])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

pub fn store(name: &str, value: &str) -> Result<(), String> {
    let mut child = Command::new("secret-tool")
        .args([
            "store",
            &format!("--label=gray-secrets {name}"),
            "service",
            SERVICE,
            "name",
            name,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run secret-tool: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(value.as_bytes())
            .map_err(|e| format!("could not pass the value to secret-tool: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("secret-tool failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "secret-tool store failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut index = index();
    index.insert(name.to_string(), json!({"created": util::now_iso()}));
    save_index(&index);
    Ok(())
}

pub fn clear(name: &str) -> Result<(), String> {
    let status = Command::new("secret-tool")
        .args(["clear", "service", SERVICE, "name", name])
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("could not run secret-tool: {e}"))?;
    let mut index = index();
    let known = index.remove(name).is_some();
    save_index(&index);
    if status.success() || known {
        Ok(())
    } else {
        Err(format!("no stored secret named {name}"))
    }
}

pub fn index() -> Map<String, Value> {
    std::fs::read_to_string(util::state_dir().join("index.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn save_index(index: &Map<String, Value>) {
    let dir = util::state_dir();
    if util::ensure_private_dir(&dir).is_err() {
        return;
    }
    let tmp = dir.join(format!("index.json.tmp-{}", std::process::id()));
    let body = serde_json::to_vec_pretty(index).unwrap_or_default();
    if util::write_private(&tmp, &body).is_ok()
        && std::fs::rename(&tmp, dir.join("index.json")).is_err()
    {
        let _ = std::fs::remove_file(&tmp);
    }
}

pub fn mark_used(names: &[String]) {
    let mut index = index();
    let now = util::now_iso();
    let mut changed = false;
    for name in names {
        if let Some(entry) = index.get_mut(name).and_then(Value::as_object_mut) {
            entry.insert("last_used".into(), json!(now));
            changed = true;
        }
    }
    if changed {
        save_index(&index);
    }
}

pub fn table(session: &[String]) -> String {
    let index = index();
    let mut rows = vec![format!(
        "{:<24} {:<10} {:<21} {}",
        "NAME", "LIFETIME", "CREATED", "LAST USED"
    )];
    for (name, meta) in &index {
        let field = |k: &str| {
            meta.get(k)
                .and_then(Value::as_str)
                .unwrap_or("-")
                .to_string()
        };
        rows.push(format!(
            "{name:<24} {:<10} {:<21} {}",
            "permanent",
            field("created"),
            field("last_used")
        ));
    }
    for name in session.iter().filter(|n| !index.contains_key(*n)) {
        rows.push(format!("{name:<24} {:<10} {:<21} -", "session", "-"));
    }
    if rows.len() == 1 {
        return "no secrets stored".to_string();
    }
    rows.join("\n")
}
