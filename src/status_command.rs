use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::host::Host;
use crate::paths;

const DEFAULT_INTERVAL_MS: u64 = 5_000;
const MIN_INTERVAL_MS: u64 = 500;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const KEY: &str = "command";

static FACTS: Mutex<Option<Value>> = Mutex::new(None);
static HOST: OnceLock<Arc<Host>> = OnceLock::new();

pub fn on_event(host: &Arc<Host>, params: &Value) {
    remember(params);
    HOST.get_or_init(|| Arc::clone(host));
    start();
}

fn remember(params: &Value) {
    let session = &params["session"];
    let usage = &params["usage"];
    let turn = json!({
        "cwd": session["cwd"],
        "workspace": { "current_dir": session["cwd"] },
        "model": session["model"],
        "session_id": session["id"],
        "usage": usage,
    });
    if let Ok(mut slot) = FACTS.lock() {
        let merged = match slot.take() {
            Some(mut prev) => {
                merge(&mut prev, &turn);
                prev
            }
            None => turn,
        };
        *slot = Some(merged);
    }
}

fn merge(base: &mut Value, next: &Value) {
    let (Some(base), Some(next)) = (base.as_object_mut(), next.as_object()) else {
        return;
    };
    for (k, v) in next {
        if v.is_null() || v.as_str() == Some("") {
            continue;
        }
        if v.is_object() {
            let slot = base.entry(k.clone()).or_insert_with(|| json!({}));
            merge(slot, v);
        } else {
            base.insert(k.clone(), v.clone());
        }
    }
}

fn start() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("qol-statusline".into())
            .spawn(move || {
                loop {
                    let Some(host) = HOST.get() else {
                        std::thread::sleep(Duration::from_millis(MIN_INTERVAL_MS));
                        continue;
                    };
                    let interval = interval_ms();
                    let shown = config_command().and_then(|cmd| {
                        run(&cmd, &facts()).and_then(|out| {
                            let line = first_line(&out);
                            (!line.is_empty()).then_some(line)
                        })
                    });
                    match shown {
                        Some(line) => {
                            if last() != Some(line.clone()) {
                                host.status(KEY, &line);
                                set_last(Some(line));
                            }
                        }
                        None => {
                            if last().is_some() {
                                host.status(KEY, "");
                                set_last(None);
                            }
                        }
                    }
                    std::thread::sleep(Duration::from_millis(interval));
                }
            });
    });
}

fn last() -> Option<String> {
    let raw = std::fs::read(paths::qol_dir().join("status-line")).ok()?;
    let text = String::from_utf8_lossy(&raw).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn set_last(line: Option<String>) {
    let path = paths::qol_dir().join("status-line");
    match line {
        Some(text) => {
            let _ = paths::write_atomic(&path, &text);
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn facts() -> Value {
    FACTS
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or(Value::Null)
}

fn config() -> Value {
    std::fs::read(paths::gray_home().join("config.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or(Value::Null)
}

fn config_command() -> Option<String> {
    let cfg = config();
    cfg["status_line"]["command"]
        .as_str()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string)
}

fn interval_ms() -> u64 {
    config()["status_line"]["interval_ms"]
        .as_u64()
        .unwrap_or(DEFAULT_INTERVAL_MS)
        .max(MIN_INTERVAL_MS)
}

fn expand_tilde(cmd: &str) -> String {
    let rest = match cmd.strip_prefix("~/") {
        Some(rest) => rest,
        None => return cmd.to_string(),
    };
    format!("{}/{rest}", paths::home().display())
}

fn run(cmd: &str, facts: &Value) -> Option<String> {
    let cmd = expand_tilde(cmd);
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(facts.to_string().as_bytes());
    }
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Err(_) => return None,
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out)
}

fn first_line(s: &str) -> String {
    crate::update::strip_ansi(s)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}
