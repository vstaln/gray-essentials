use std::path::Path;

use serde_json::{Value, json};

use crate::host::Host;
use crate::prompt;
use crate::settings;

pub fn session_start(host: &Host, params: &Value) {
    let rules = if settings::on("issue_links") {
        prompt::cwd(params)
            .and_then(|cwd| remote_base(&cwd))
            .map(|base| json!([{"pattern": "#([0-9]+)", "url": format!("{base}/issues/$1")}]))
            .unwrap_or_else(|| json!([]))
    } else {
        json!([])
    };
    host.link_rules(rules);
}

fn remote_base(cwd: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["remote", "get-url", "origin"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    normalize_remote(&String::from_utf8_lossy(&out.stdout))
}

fn normalize_remote(remote: &str) -> Option<String> {
    let r = remote.trim().trim_end_matches('/');
    let r = r.strip_suffix(".git").unwrap_or(r);
    let (host, path) = if let Some(rest) = r.strip_prefix("git@") {
        rest.split_once(':')?
    } else {
        let rest = r.split_once("://").map(|(_, rest)| rest).unwrap_or(r);
        if rest == r && !rest.contains('/') {
            return None;
        }
        let rest = rest.split_once('@').map(|(_, rest)| rest).unwrap_or(rest);
        rest.split_once('/')?
    };
    if host.is_empty() || path.is_empty() || !path.contains('/') {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}
