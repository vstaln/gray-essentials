use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::host::{self, Host};
use crate::paths;

pub fn command(host: &Host, args: &[String]) -> Value {
    let target = args.join(" ");
    let target = target.trim();
    let Some(dir) = resolve(target) else {
        let shown = absolute(target)
            .map(|p| paths::tilde(&p))
            .unwrap_or_else(|| target.to_string());
        return json!({"text": format!("not a directory: {shown}")});
    };
    if cwd().is_some_and(|here| here == dir) {
        return json!({"text": format!("already in {}", paths::tilde(&dir))});
    }
    if let Some(project) = cwd().as_deref().and_then(worktree)
        && !dir.starts_with(&project)
    {
        match confirm(host, &project, &dir) {
            Ok(true) => {}
            Ok(false) => return json!({"text": "change directory cancelled"}),
            Err(e) => {
                return json!({
                    "text": format!("change directory needs confirmation: {e}")
                });
            }
        }
    }
    match host.call(
        "host/restart",
        json!({"resume": true, "cwd": dir}),
        host::HOST_TTL,
    ) {
        Ok(_) => json!({
            "text": format!(
                "changing directory to {} — restarting gray (same conversation)…",
                paths::tilde(&dir)
            )
        }),
        Err(e) => json!({"text": format!("could not change directory: {e}")}),
    }
}

fn cwd() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    Some(std::fs::canonicalize(&cwd).unwrap_or(cwd))
}

fn absolute(target: &str) -> Option<PathBuf> {
    let path = if target.is_empty() {
        paths::home()
    } else if let Some(rest) = target.strip_prefix('~') {
        let rest = rest.trim_start_matches(&['/', '\\'][..]);
        paths::home().join(rest)
    } else {
        PathBuf::from(target)
    };
    Some(if path.is_absolute() {
        path
    } else {
        cwd()?.join(path)
    })
}

fn resolve(target: &str) -> Option<PathBuf> {
    let path = std::fs::canonicalize(absolute(target)?).ok()?;
    path.is_dir().then_some(path)
}

fn worktree(cwd: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let top = String::from_utf8(out.stdout).ok()?;
    let top = PathBuf::from(top.trim());
    let top = std::fs::canonicalize(&top).unwrap_or(top);
    top.is_dir().then_some(top)
}

fn confirm(host: &Host, project: &Path, target: &Path) -> Result<bool, String> {
    let questions = json!([{
        "id": "confirm",
        "header": "Change directory",
        "question": "Target is outside this project:",
        "detail": format!(
            "{}\ncurrent project: {}",
            paths::tilde(target),
            paths::tilde(project)
        ),
        "options": [
            {"label": "Change", "description": "restart gray in the new directory"},
            {"label": "Cancel", "description": "stay where you are"}
        ],
        "no_other": true
    }]);
    let answer = host.ask(questions)?;
    Ok(host::answer_labels(&answer, "confirm")
        .is_some_and(|labels| labels.iter().any(|l| l == "Change")))
}
