use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use serde_json::{Value, json};

pub const ASK_TTL: Duration = Duration::from_secs(300);
pub const HOST_TTL: Duration = Duration::from_secs(25);
pub const INSTALL_TTL: Duration = Duration::from_secs(600);
pub const MODEL_TTL: Duration = Duration::from_secs(120);

pub struct Host {
    out: Mutex<std::io::Stdout>,
    pending: Mutex<HashMap<String, mpsc::Sender<Value>>>,
    counter: AtomicU64,
    closed: std::sync::atomic::AtomicBool,
}

impl Host {
    pub fn new() -> Self {
        Self {
            out: Mutex::new(std::io::stdout()),
            pending: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            closed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    pub fn write(&self, v: &Value) {
        let mut o = self.out.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
    }

    pub fn deliver(&self, id: &str, msg: Value) {
        let tx = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        if let Some(tx) = tx {
            let _ = tx.send(msg);
        }
    }

    pub fn call(&self, method: &str, params: Value, ttl: Duration) -> Result<Value, String> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(format!("{method}: host did not answer"));
        }
        let n = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("qol{n}");
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), tx);
        self.write(&json!({"id": id, "method": method, "params": params}));
        let msg = rx.recv_timeout(ttl).map_err(|_| {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            format!("{method}: host did not answer")
        })?;
        if let Some(err) = msg.get("error").filter(|e| !e.is_null()) {
            return Err(err
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| err.to_string()));
        }
        let result = msg.get("result").cloned().unwrap_or(Value::Null);
        if let Some(err) = result.get("error").and_then(Value::as_str) {
            return Err(err.to_string());
        }
        Ok(result)
    }

    pub fn say(&self, text: &str) {
        let _ = self.call("host/say", json!({"text": text}), HOST_TTL);
    }

    pub fn notice(&self, text: &str, action: &str, slot: &str, id: &str) -> bool {
        self.call(
            "host/notice",
            json!({"text": text, "action": action, "slot": slot, "id": id}),
            HOST_TTL,
        )
        .ok()
        .and_then(|v| v.get("shown").and_then(Value::as_bool))
        .unwrap_or(false)
    }

    pub fn say_card(&self, text: &str, card: &str) {
        let _ = self.call("host/say", json!({"text": text, "card": card}), HOST_TTL);
    }

    pub fn status(&self, key: &str, text: &str) {
        let _ = self.call("host/status", json!({"key": key, "text": text}), HOST_TTL);
    }

    pub fn transcript(&self, last: usize) -> Result<Vec<Value>, String> {
        self.call("host/transcript", json!({"last": last}), HOST_TTL)
            .map(|v| {
                v.get("messages")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
    }

    pub fn complete(&self, system: &str, prompt: &str, max_tokens: u32) -> Option<String> {
        let v = self
            .call(
                "host/complete",
                json!({"system": system, "prompt": prompt, "max_tokens": max_tokens}),
                MODEL_TTL,
            )
            .ok()?;
        let text = v.get("text").and_then(Value::as_str)?;
        (!text.trim().is_empty()).then(|| text.to_string())
    }

    pub fn session_title(&self, title: &str) -> bool {
        ok_bool(self.call("host/session_title", json!({"title": title}), HOST_TTL))
    }

    pub fn session_name(&self, name: &str) -> Result<Value, String> {
        self.call("host/session_name", json!({"name": name}), HOST_TTL)
    }

    pub fn inject(&self, text: &str) -> bool {
        self.call("host/inject", json!({"text": text}), HOST_TTL)
            .map(|v| v.get("ok").and_then(Value::as_bool).unwrap_or(true))
            .unwrap_or(false)
    }

    pub fn ghost(&self, text: &str) -> bool {
        ok_bool(self.call("host/ghost", json!({"text": text}), HOST_TTL))
    }

    pub fn link_rules(&self, rules: Value) -> bool {
        ok_bool(self.call("host/link_rules", json!({"rules": rules}), HOST_TTL))
    }

    pub fn notify(&self, title: &str, body: &str) -> bool {
        ok_bool(self.call(
            "host/notify",
            json!({"title": title, "body": body}),
            HOST_TTL,
        ))
    }

    pub fn sessions(&self, params: Value) -> Vec<Value> {
        self.call("host/sessions", params, HOST_TTL)
            .ok()
            .and_then(|v| v.get("sessions").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    }

    pub fn announce(&self, method: &str) {
        self.write(&json!({"method": method, "params": {}}));
    }

    pub fn ask(&self, questions: Value) -> Result<Value, String> {
        self.call(
            "host/ask",
            json!({"questions": questions, "blocking": true}),
            ASK_TTL,
        )
    }
}

fn ok_bool(result: Result<Value, String>) -> bool {
    result
        .map(|v| v.get("ok").and_then(Value::as_bool).unwrap_or(true))
        .unwrap_or(false)
}

pub fn answer_labels(result: &Value, id: &str) -> Option<Vec<String>> {
    let entry = result.get("answers")?.get(id)?;
    let list = |v: &Value| -> Option<Vec<String>> {
        v.as_array().map(|a| {
            a.iter()
                .filter_map(|e| e.as_str().map(str::to_string))
                .collect()
        })
    };
    entry
        .get("selected")
        .and_then(list)
        .or_else(|| entry.get("answers").and_then(list))
        .or_else(|| list(entry))
}
