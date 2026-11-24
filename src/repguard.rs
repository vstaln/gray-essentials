//! `tool/before` hashes (name, canonicalized-args) and counts consecutive
//! repeats per session. When the same call runs `REPGUARD_MAX_REPEATS`
//! (default 3) times in a row, `tool/after` appends a "you are looping,
//! change approach" nudge to that call's result. A different call — or a
//! gap longer than `REPGUARD_WINDOW_SECS` (default 600) — resets the run,
//! so interleaved iteration never trips the guard.
//!
//! Canonicalization sorts object keys recursively before hashing, so
//! `{"a":1,"b":2}` and `{"b":2,"a":1}` count as the same call.
//!
//! `/repguard` shows config and the current session's repeat counts.

use std::collections::{HashMap, VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Consecutive identical calls that trigger the nudge.
fn max_repeats() -> usize {
    std::env::var("REPGUARD_MAX_REPEATS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n >= 2)
        .unwrap_or(3)
}

/// Sliding window for tracked calls; also the run-staleness cutoff.
fn window() -> Duration {
    Duration::from_secs(
        std::env::var("REPGUARD_WINDOW_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(600),
    )
}

/// Recursively sort object keys so key order can't dodge the hash.
fn canon(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::with_capacity(m.len());
            for k in keys {
                out.insert(k.clone(), canon(&m[k]));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(canon).collect()),
        _ => v.clone(),
    }
}

fn call_hash(name: &str, args: &Value) -> u64 {
    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    canon(args).to_string().hash(&mut h);
    h.finish()
}

fn session_key(params: &Value) -> String {
    let s = params.get("session").cloned().unwrap_or(Value::Null);
    s.get("id")
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .or_else(|| s.get("cwd").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

/// Per-session sliding window of recent call hashes, the consecutive-run
/// counter, and the flag the last `tool/before` raised for `tool/after`.
#[derive(Default)]
struct Sess {
    calls: VecDeque<(u64, Instant)>,
    last: Option<(u64, Instant)>,
    run: usize,
    flag: Option<usize>,
}

type State = Mutex<HashMap<String, Sess>>;

static STATE: LazyLock<State> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Record this call and return its run length: the run only grows when the
/// immediately preceding call had the same hash within `win`.
fn record(sess: &mut Sess, h: u64, win: Duration) -> usize {
    let now = Instant::now();
    while sess
        .calls
        .front()
        .is_some_and(|(_, t)| now.duration_since(*t) > win)
    {
        sess.calls.pop_front();
    }
    sess.calls.push_back((h, now));
    sess.run = match sess.last {
        Some((lh, lt)) if lh == h && now.duration_since(lt) <= win => sess.run + 1,
        _ => 1,
    };
    sess.last = Some((h, now));
    sess.run
}

pub fn tool_before(params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("args").cloned().unwrap_or(Value::Null);
    let h = call_hash(name, &args);
    let key = session_key(params);
    let mut guard = STATE.lock().expect("state");
    let sess = guard.entry(key).or_default();
    let run = record(sess, h, window());
    sess.flag = (run >= max_repeats()).then_some(run);
    json!({"decision": "allow"})
}

pub fn tool_after(params: &Value) -> Value {
    let key = session_key(params);
    let mut guard = STATE.lock().expect("state");
    let Some(sess) = guard.get_mut(&key) else {
        return json!({});
    };
    let Some(run) = sess.flag.take() else {
        return json!({});
    };
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let content = params.get("content").and_then(Value::as_str).unwrap_or("");
    let nudge = format!(
        "\n\n[repguard] `{name}` has now run {run} times in a row with identical \
         arguments — you are looping. Change approach: use different \
         arguments, a different tool, or stop and report what is blocking you."
    );
    json!({ "content": format!("{content}{nudge}") })
}

pub fn command(argv: &[String], params: &Value) -> String {
    let words: Vec<&str> = argv.iter().map(String::as_str).collect();
    if words.first().copied() == Some("status") || words.is_empty() {
        let key = session_key(params);
        let guard = STATE.lock().expect("state");
        let (tracked, run) = guard
            .get(&key)
            .map(|s| (s.calls.len(), s.run))
            .unwrap_or((0, 0));
        return format!(
            "repguard {} — nudge at {} identical calls in a row / {}s \
             window (env REPGUARD_MAX_REPEATS, REPGUARD_WINDOW_SECS)\n\
             this session: {} calls tracked, current run {}x",
            env!("CARGO_PKG_VERSION"),
            max_repeats(),
            window().as_secs(),
            tracked,
            run,
        );
    }
    "usage: /repguard [status]".into()
}
