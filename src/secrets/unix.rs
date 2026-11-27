use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde_json::{Value, json};

use super::exec::{SOCKET_ENV, TOKEN_ENV};
use super::mask::Masker;
use super::{cli, exec, store, util};
use crate::host::{ASK_TTL, Host};

const NO_PROMPT: &str =
    "No interactive prompt available; ask the user to run `gray secrets set {name}` in a terminal.";

struct State {
    session: BTreeMap<String, String>,
    seen: Vec<(String, String)>,
    server: Option<(PathBuf, String)>,
    lost: Vec<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    session: BTreeMap::new(),
    seen: Vec::new(),
    server: None,
    lost: Vec::new(),
});

static HOST: OnceLock<i32> = OnceLock::new();

fn host_pid() -> i32 {
    *HOST.get_or_init(|| unsafe { libc::getppid() })
}

fn secrets_runtime() -> Option<PathBuf> {
    let dir = util::runtime_dir()?.join("gray-secrets");
    util::ensure_private_dir(&dir).ok()?;
    Some(dir)
}

fn owner_of(file: &str) -> Option<i32> {
    let stem = file
        .strip_prefix("ledger-")
        .or_else(|| file.strip_prefix("reaper-"))
        .unwrap_or(file);
    stem.split(['-', '.']).next()?.parse().ok()
}

fn remove_host_files(dir: &std::path::Path, alive: impl Fn(i32) -> bool, sockets_of_self: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = owner_of(&name) else { continue };
        let stale_self = sockets_of_self && pid == host_pid() && name.ends_with(".sock");
        if stale_self || !alive(pid) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn pid_alive(pid: i32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

fn sweep() {
    if let Some(dir) = secrets_runtime() {
        remove_host_files(&dir, pid_alive, true);
    }
}

fn spawn_reaper() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(exe)
        .args(["secrets", "reap", &host_pid().to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn reap(args: &[String]) -> i32 {
    let Some(host) = args.first().and_then(|a| a.parse::<i32>().ok()) else {
        return 2;
    };
    let Some(dir) = secrets_runtime() else {
        return 1;
    };
    unsafe {
        libc::setsid();
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
    let Ok(lock) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("reaper-{host}.lock")))
    else {
        return 1;
    };
    use std::os::fd::AsRawFd;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return 0;
    }
    #[cfg(target_os = "linux")]
    let waited = {
        let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, host, 0) } as i32;
        if pidfd >= 0 {
            let mut fds = libc::pollfd {
                fd: pidfd,
                events: libc::POLLIN,
                revents: 0,
            };
            while unsafe { libc::poll(&mut fds, 1, -1) } < 0 {}
            true
        } else {
            false
        }
    };
    #[cfg(not(target_os = "linux"))]
    let waited = false;
    if !waited {
        while pid_alive(host) {
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
    remove_host_files(&dir, |pid| pid != host, false);
    0
}

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

impl State {
    fn remember(&mut self, name: &str, value: &str) {
        self.lost.retain(|n| n != name);
        if !self.seen.iter().any(|(n, v)| n == name && v == value) {
            self.seen.push((name.to_string(), value.to_string()));
        }
        self.write_ledger();
    }

    fn write_ledger(&self) {
        let Some(path) = ledger_path() else { return };
        let mut permanent: Vec<&str> = Vec::new();
        let mut session: Vec<&str> = self.lost.iter().map(String::as_str).collect();
        for (n, _) in &self.seen {
            let list = if self.session.contains_key(n) {
                &mut session
            } else {
                &mut permanent
            };
            if !list.contains(&n.as_str()) {
                list.push(n);
            }
        }
        let body = json!({"permanent": permanent, "session": session}).to_string();
        let tmp = path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp);
        if util::write_private(&tmp, body.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    fn masker(&self) -> Masker {
        let token = self
            .server
            .as_ref()
            .map(|(_, t)| ("session-token", t.as_str()));
        Masker::new(
            self.seen
                .iter()
                .map(|(n, v)| (n.as_str(), v.as_str()))
                .chain(token),
        )
    }
}

pub const TOOL_NAME: &str = "secret_request";

pub fn tool_def() -> Value {
    let m = raw_manifest();
    m["tools"][0].clone()
}

fn raw_manifest() -> Value {
    json!({
        "name": "secrets",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": "2.0",
        "description": "named secrets: masked in-chat entry, libsecret storage, run-time injection, output masking",
        "tools": [{
            "name": "secret_request",
            "label": "Secret",
            "description": "Ask the user for a secret (API key, password, token) by name. The user types it into a masked prompt in gray's terminal; you never see the value. Returns {name, status} where status is stored (saved in the keyring), session (kept for this session only) or declined. Afterwards reference it in bash commands as {{secret:NAME}}; the value is injected at run time and masked as <secret:NAME> in output. Never ask the user to paste a secret into the chat.",
            "parameters": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Secret name: letters, digits and underscores, starting with a letter, e.g. GITHUB_TOKEN"},
                    "purpose": {"type": "string", "description": "One sentence shown to the user saying what the secret is for"}
                },
                "required": ["name", "purpose"]
            }
        }],
        "commands": ["/secrets"],
        "command_descriptions": {"/secrets": "list secret names, lifetimes and last use (never values)"},
        "completion": ["set", "rm", "list", "exec", "elevate-setup"],
        "hooks": ["tool/before", "tool/after"],
        "required_hooks": ["tool/after"],
        "capabilities": ["host.ask"],
    })
}

fn ensure_server(st: &mut State) -> Option<(PathBuf, String)> {
    if let Some(s) = &st.server {
        return Some(s.clone());
    }
    let dir = secrets_runtime()?;
    let path = dir.join(format!(
        "{}-{}.sock",
        host_pid(),
        util::random_hex(12).ok()?
    ));
    let token = util::random_hex(32).ok()?;
    let listener = UnixListener::bind(&path).ok()?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).ok()?;
    let want = token.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let _ = conn.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            let mut line = String::new();
            let Ok(reader) = conn.try_clone() else {
                continue;
            };
            if BufReader::new(reader.take(1 << 16))
                .read_line(&mut line)
                .is_err()
            {
                continue;
            }
            let req: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            let token = req.get("token").and_then(Value::as_str).unwrap_or("");
            let reply = if util::same_secret(token, &want) {
                let st = state();
                let values: serde_json::Map<String, Value> = req
                    .get("names")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter_map(|n| Some((n.to_string(), json!(st.session.get(n)?))))
                    .collect();
                json!({"values": values})
            } else {
                json!({"error": "unauthorised"})
            };
            let _ = writeln!(conn, "{reply}");
        }
    });
    st.server = Some((path.clone(), token.clone()));
    Some((path, token))
}

fn ledger_path() -> Option<PathBuf> {
    Some(secrets_runtime()?.join(format!("ledger-{}.json", host_pid())))
}

fn recover() {
    let Some(path) = ledger_path() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let ledger: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let names = |key: &str| -> Vec<String> {
        ledger
            .get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|n| util::valid_name(n))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut st = state();
    st.lost = names("session");
    for name in names("permanent") {
        match store::lookup(&name) {
            Some(v) => st.remember(&name, &v),
            None => st.lost.push(name),
        }
    }
    st.write_ledger();
}

pub fn init() {
    host_pid();
    sweep();
    spawn_reaper();
    recover();
}

pub fn cleanup() {
    let mut st = state();
    if let Some((path, _)) = st.server.take() {
        let _ = std::fs::remove_file(path);
    }
    st.seen.clear();
    st.session.clear();
    st.lost.clear();
    if let Some(path) = ledger_path() {
        let _ = std::fs::remove_file(path);
    }
}

fn session_of(params: &Value) -> String {
    params
        .pointer("/session/id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn has_word(command: &str, word: &str) -> bool {
    command.match_indices(word).any(|(i, _)| {
        let before = command[..i].chars().next_back();
        let after = command[i + word.len()..].chars().next();
        let edge = |c: Option<char>| {
            c.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        };
        edge(before) && edge(after)
    })
}

fn touches_state(command: &str) -> bool {
    let mut needles = vec![
        ".gray/secrets".to_string(),
        util::state_dir().to_string_lossy().into_owned(),
        TOKEN_ENV.to_string(),
        SOCKET_ENV.to_string(),
        "XDG_RUNTIME_DIR/gray-secrets".to_string(),
        "XDG_RUNTIME_DIR}/gray-secrets".to_string(),
    ];
    if let Some(rt) = util::runtime_dir() {
        needles.push(rt.join("gray-secrets").to_string_lossy().into_owned());
    }
    needles.iter().any(|n| command.contains(n.as_str()))
}

fn placeholders(command: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = command[from..].find("{{secret:") {
        let start = from + at;
        let body = start + "{{secret:".len();
        match command[body..].find("}}") {
            Some(len) => {
                out.push((
                    start,
                    body + len + 2,
                    command[body..body + len].trim().to_string(),
                ));
                from = body + len + 2;
            }
            None => break,
        }
    }
    out
}

fn substitute(command: &str, spots: &[(usize, usize, String)]) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum Q {
        None,
        Single,
        Double,
    }
    let mut out = String::with_capacity(command.len() + 32);
    let mut q = Q::None;
    let mut escaped = false;
    let mut next = spots.iter().peekable();
    let mut i = 0;
    while i < command.len() {
        if let Some((start, end, name)) = next.peek().copied()
            && *start == i
        {
            out.push_str(&match q {
                Q::None => format!("\"${{{name}}}\""),
                Q::Double => format!("${{{name}}}"),
                Q::Single => format!("'\"${{{name}}}\"'"),
            });
            i = *end;
            next.next();
            escaped = false;
            continue;
        }
        let c = command[i..].chars().next().unwrap_or_default();
        if escaped {
            escaped = false;
        } else {
            match (q, c) {
                (Q::None | Q::Double, '\\') => escaped = true,
                (Q::None, '\'') => q = Q::Single,
                (Q::Single, '\'') => q = Q::None,
                (Q::None, '"') => q = Q::Double,
                (Q::Double, '"') => q = Q::None,
                _ => {}
            }
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn secret_tool_deny_msg() -> String {
    "gray-secrets blocked this command: calling secret-tool directly would print stored secret values into the conversation. Reference the secret as {{secret:NAME}} in the command instead; `gray secrets list` shows the names.".into()
}

fn touches_state_deny_msg() -> String {
    "gray-secrets blocked this command: it touches the secrets plugin's own state (keyring index, audit log or session socket). Use `gray secrets list` for names and {{secret:NAME}} to use a value.".into()
}

fn invalid_name_deny_msg(bad: &str) -> String {
    format!(
        "invalid secret name {bad:?} in {{{{secret:...}}}}; use letters, digits and underscores"
    )
}

fn deny(reason: String) -> Value {
    json!({"decision": "deny", "reason": reason})
}

pub fn precheck(params: &Value) -> Option<Value> {
    if params.get("name").and_then(Value::as_str) != Some("bash") {
        return None;
    }
    let command = params
        .pointer("/args/command")
        .and_then(Value::as_str)
        .unwrap_or("");
    let sid = session_of(params);
    let masked = state().masker().mask(command);
    if has_word(command, "secret-tool") {
        util::audit(&sid, "-", "deny", &masked);
        return Some(deny(secret_tool_deny_msg()));
    }
    if touches_state(command) {
        util::audit(&sid, "-", "deny", &masked);
        return Some(deny(touches_state_deny_msg()));
    }
    let spots = placeholders(command);
    if spots.is_empty() {
        return None;
    }
    let mut names: Vec<String> = spots.iter().map(|(_, _, n)| n.clone()).collect();
    names.sort();
    names.dedup();
    if let Some(bad) = names.iter().find(|n| !util::valid_name(n)) {
        return Some(deny(invalid_name_deny_msg(bad)));
    }
    let st = state();
    for name in &names {
        if st.session.contains_key(name) {
            continue;
        }
        if store::lookup(name).is_none() {
            drop(st);
            util::audit(&sid, name, "deny", &masked);
            return Some(deny(format!(
                "secret {name} is not set. Call secret_request with name {name} and a purpose so the user can enter it, or ask the user to run `gray secrets set {name}` in a terminal."
            )));
        }
    }
    None
}

pub fn tool_before(params: &Value) -> Value {
    if params.get("name").and_then(Value::as_str) != Some("bash") {
        return json!({"decision": "allow"});
    }
    let command = params
        .pointer("/args/command")
        .and_then(Value::as_str)
        .unwrap_or("");
    let sid = session_of(params);
    let masked = state().masker().mask(command);
    if has_word(command, "secret-tool") {
        util::audit(&sid, "-", "deny", &masked);
        return deny("gray-secrets blocked this command: calling secret-tool directly would print stored secret values into the conversation. Reference the secret as {{secret:NAME}} in the command instead; `gray secrets list` shows the names.".into());
    }
    if touches_state(command) {
        util::audit(&sid, "-", "deny", &masked);
        return deny("gray-secrets blocked this command: it touches the secrets plugin's own state (keyring index, audit log or session socket). Use `gray secrets list` for names and {{secret:NAME}} to use a value.".into());
    }
    let spots = placeholders(command);
    if spots.is_empty() {
        return json!({"decision": "allow"});
    }
    let mut names: Vec<String> = spots.iter().map(|(_, _, n)| n.clone()).collect();
    names.sort();
    names.dedup();
    if let Some(bad) = names.iter().find(|n| !util::valid_name(n)) {
        return deny(format!(
            "invalid secret name {bad:?} in {{{{secret:...}}}}; use letters, digits and underscores"
        ));
    }
    let mut st = state();
    let mut need_server = false;
    for name in &names {
        if let Some(v) = st.session.get(name).cloned() {
            st.remember(name, &v);
            need_server = true;
            continue;
        }
        match store::lookup(name) {
            Some(v) => st.remember(name, &v),
            None => {
                drop(st);
                util::audit(&sid, name, "deny", &masked);
                return deny(format!(
                    "secret {name} is not set. Call secret_request with name {name} and a purpose so the user can enter it, or ask the user to run `gray secrets set {name}` in a terminal."
                ));
            }
        }
    }
    let mut prefix = String::new();
    if need_server {
        match ensure_server(&mut st) {
            Some((path, token)) => {
                prefix = format!(
                    "{SOCKET_ENV}={} {TOKEN_ENV}={token} ",
                    util::shell_quote(&path.to_string_lossy())
                );
            }
            None => {
                return deny(
                    "gray-secrets could not open its session socket (is XDG_RUNTIME_DIR set?)"
                        .into(),
                );
            }
        }
    }
    drop(st);
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "gray-essentials".into());
    let inner = substitute(command, &spots);
    let rewritten = format!(
        "{prefix}{} exec --env {} -- bash -c {}",
        util::shell_quote(&exe),
        names.join(","),
        util::shell_quote(&inner)
    );
    let mut args = params.get("args").cloned().unwrap_or_else(|| json!({}));
    args["command"] = json!(rewritten);
    json!({"decision": "modify", "args": args, "concurrent": true})
}

pub fn tool_after(params: &Value) -> Value {
    let content = params.get("content").and_then(Value::as_str).unwrap_or("");
    let st = state();
    if !st.lost.is_empty() {
        let names = st.lost.join(", ");
        return json!({
            "content": format!("gray-secrets restarted and no longer holds the session-only secret(s) {names}, so this output cannot be masked; output withheld. Call secret_request for {names} again (or restart gray) to continue."),
            "is_error": true
        });
    }
    let masked = st.masker().mask(content);
    drop(st);
    if masked == content {
        json!({})
    } else {
        json!({"content": masked})
    }
}

const SAVE_PERMANENTLY: &str = "Save permanently";
const SESSION_ONLY: &str = "This session only";

fn value_question(question: String) -> Value {
    json!({
        "id": "secret",
        "header": "Secret",
        "question": question,
        "options": [
            {"label": SAVE_PERMANENTLY, "freeform": true},
            {"label": SESSION_ONLY, "freeform": true},
        ],
        "no_other": true,
        "masked": true,
    })
}

fn typed(reply: &Value, id: &str) -> (String, Vec<String>) {
    let picked = crate::host::answer_labels(reply, id).unwrap_or_default();
    let text = picked
        .iter()
        .find_map(|a| a.strip_prefix("user_note: "))
        .unwrap_or("")
        .to_string();
    (text, picked)
}

fn ask(host: &Host, questions: Value) -> Result<Value, String> {
    let reply = host.call(
        "host/ask",
        json!({"blocking": true, "questions": questions}),
        ASK_TTL,
    )?;
    match reply.get("error").and_then(Value::as_str) {
        Some(e) => Err(e.to_string()),
        None => Ok(reply),
    }
}

fn prompt_value(host: &Host, name: &str, purpose: &str) -> Result<Option<(String, bool)>, String> {
    let question = if purpose.is_empty() {
        format!("Secret {name}")
    } else {
        format!("Secret {name}\n{purpose}")
    };
    let reply = ask(host, json!([value_question(question)]))?;
    let (value, picked) = typed(&reply, "secret");
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some((value, picked.iter().any(|a| a == SAVE_PERMANENTLY))))
}

fn save(sid: &str, name: &str, value: &str, persist: bool) -> Result<&'static str, String> {
    if persist {
        store::store(name, value)
            .map_err(|e| format!("could not store {name} in the keyring: {e}"))?;
        let mut st = state();
        st.session.remove(name);
        st.remember(name, value);
        drop(st);
        util::audit(sid, name, "set", "(permanent)");
        return Ok("stored");
    }
    if store::index().contains_key(name) {
        store::clear(name)?;
    }
    let mut st = state();
    st.session.insert(name.to_string(), value.to_string());
    st.remember(name, value);
    drop(st);
    util::audit(sid, name, "set", "(session)");
    Ok("session")
}

pub fn tool_call(host: &Host, params: &Value) -> Value {
    let args = params.get("args").cloned().unwrap_or(Value::Null);
    let field = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let name = field("name");
    let purpose = field("purpose");
    let sid = session_of(params);
    let fail = |text: String| json!({"content": text, "is_error": true});
    if !util::valid_name(&name) {
        return fail(format!(
            "invalid secret name {name:?}: use letters, digits and underscores, starting with a letter"
        ));
    }
    util::audit(&sid, &name, "request", &purpose);
    let status = match prompt_value(host, &name, &purpose) {
        Err(err) => {
            util::audit(&sid, &name, "deny", "(no prompt)");
            if err.contains("capability_not_granted") {
                return fail(format!(
                    "gray-secrets may not open the masked prompt yet (the host.ask capability is not granted). Ask the user to run `gray plugin capabilities essentials --all`, or to run `gray secrets set {name}` in a terminal."
                ));
            }
            return fail(NO_PROMPT.replace("{name}", &name));
        }
        Ok(None) => {
            util::audit(&sid, &name, "deny", "(declined)");
            "declined"
        }
        Ok(Some((value, persist))) => match save(&sid, &name, &value, persist) {
            Ok(s) => s,
            Err(e) => return fail(e),
        },
    };
    json!({"content": json!({"name": name, "status": status}).to_string()})
}

struct Entry {
    name: String,
    saved: bool,
    created: Option<String>,
    used: Option<String>,
}

fn entries() -> Vec<Entry> {
    let index = store::index();
    let field = |meta: &Value, k: &str| meta.get(k).and_then(Value::as_str).map(str::to_string);
    let mut out: Vec<Entry> = index
        .iter()
        .map(|(name, meta)| Entry {
            name: name.clone(),
            saved: true,
            created: field(meta, "created"),
            used: field(meta, "last_used"),
        })
        .collect();
    for name in state().session.keys().filter(|n| !index.contains_key(*n)) {
        out.push(Entry {
            name: name.clone(),
            saved: false,
            created: None,
            used: None,
        });
    }
    out
}

fn ago(iso: Option<&str>) -> String {
    let Some(t) = iso.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()) else {
        return "never used".to_string();
    };
    let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc))
        .num_seconds()
        .max(0);
    match secs {
        0..60 => "used just now".to_string(),
        60..3600 => format!("used {}m ago", secs / 60),
        3600..86400 => format!("used {}h ago", secs / 3600),
        _ => format!("used {}d ago", secs / 86400),
    }
}

fn lifetime(saved: bool) -> &'static str {
    if saved { "saved" } else { "session" }
}

fn picker(at: Option<&str>) -> Value {
    let list = entries();
    let width = list.iter().map(|e| e.name.len()).max().unwrap_or(0);
    let mut items: Vec<Value> = list
        .iter()
        .map(|e| {
            json!({
                "id": format!("s.{}", e.name),
                "label": format!("{:<width$}", e.name),
                "description": format!("{:<8} {}", lifetime(e.saved), ago(e.used.as_deref())),
            })
        })
        .collect();
    items.push(json!({"id": "new", "label": "+ Add secret", "gap_before": !list.is_empty()}));
    let mut spec = json!({
        "title": "Secrets",
        "items": items,
        "enter": "actions",
        "keys": [{"key": "n", "label": "new"}, {"key": "d", "label": "delete"}],
    });
    if let Some(id) = at {
        spec["selected_id"] = json!(id);
        spec["cursor_only"] = json!(true);
    }
    json!({"picker": spec})
}

fn actions(name: &str) -> Value {
    let Some(e) = entries().into_iter().find(|e| e.name == name) else {
        return picker(None);
    };
    let mut info = vec![lifetime(e.saved).to_string()];
    if let Some(day) = e
        .created
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
    {
        info.push(format!("created {}", day.format("%b %-d")));
    }
    info.push(ago(e.used.as_deref()));
    json!({"picker": {
        "title": format!("{name}  {}", info.join(" \u{b7} ")),
        "items": [
            {"id": format!("replace.{name}"), "label": "Replace value"},
            {"id": format!("life.{name}"), "label": if e.saved { "Make session only" } else { SAVE_PERMANENTLY }},
            {"id": format!("delete.{name}"), "label": "Delete"},
        ],
    }})
}

fn confirm_delete(name: &str) -> Value {
    json!({"picker": {
        "title": format!("Delete {name}?"),
        "items": [
            {"id": format!("rm.{name}"), "label": "Delete"},
            {"id": format!("keep.{name}"), "label": "Keep"},
        ],
    }})
}

fn delete(sid: &str, name: &str) -> Result<(), String> {
    if store::index().contains_key(name) {
        store::clear(name)?;
    }
    state().session.remove(name);
    util::audit(sid, name, "rm", "(/secrets)");
    Ok(())
}

fn switch_lifetime(sid: &str, name: &str) -> Result<(), String> {
    let saved = store::index().contains_key(name);
    let value = if saved {
        store::lookup(name)
    } else {
        state().session.get(name).cloned()
    }
    .ok_or_else(|| format!("{name} has no value to move"))?;
    save(sid, name, &value, !saved).map(|_| ())
}

fn add(host: &Host, sid: &str) -> Value {
    let questions = json!([
        {
            "id": "name",
            "header": "Name",
            "question": "New secret",
            "options": [{"label": "Name", "freeform": true}],
            "no_other": true,
        },
        value_question("Value".to_string()),
    ]);
    let reply = match ask(host, questions) {
        Ok(r) => r,
        Err(e) => return json!({"text": format!("{} ({e})", NO_PROMPT.replace("{name}", "NAME"))}),
    };
    let name = typed(&reply, "name").0.trim().to_string();
    let (value, picked) = typed(&reply, "secret");
    if name.is_empty() && value.is_empty() {
        return picker(None);
    }
    if !util::valid_name(&name) {
        return json!({"text": format!("\u{2717} {name:?} is not a valid name: use letters, digits and underscores, starting with a letter")});
    }
    if value.is_empty() {
        return json!({"text": format!("\u{2717} {name} was not saved: no value entered")});
    }
    match save(
        sid,
        &name,
        &value,
        picked.iter().any(|a| a == SAVE_PERMANENTLY),
    ) {
        Ok(_) => picker(Some(&format!("s.{name}"))),
        Err(e) => json!({"text": format!("\u{2717} {e}")}),
    }
}

fn on_pick(host: &Host, pick: &str) -> Value {
    let sid = util::session_id();
    let (id, key) = match pick.rsplit_once(':') {
        Some((id, k)) if k.chars().count() == 1 => (id, k),
        _ => (pick, ""),
    };
    let back = |name: &str, r: Result<(), String>| match r {
        Ok(()) => picker(Some(&format!("s.{name}"))),
        Err(e) => json!({"text": format!("\u{2717} {e}")}),
    };
    let (verb, name) = id.split_once('.').unwrap_or((id, ""));
    match (verb, key) {
        (_, "n") | ("new", _) => add(host, &sid),
        ("s", "d") | ("delete", _) => confirm_delete(name),
        ("s", _) => actions(name),
        ("replace", _) => match prompt_value(host, name, "") {
            Ok(Some((value, persist))) => back(name, save(&sid, name, &value, persist).map(|_| ())),
            Ok(None) => picker(Some(&format!("s.{name}"))),
            Err(_) => json!({"text": NO_PROMPT.replace("{name}", name)}),
        },
        ("life", _) => back(name, switch_lifetime(&sid, name)),
        ("rm", _) => match delete(&sid, name) {
            Ok(()) => picker(None),
            Err(e) => json!({"text": format!("\u{2717} {e}")}),
        },
        ("keep", _) => picker(Some(&format!("s.{name}"))),
        _ => picker(None),
    }
}

pub fn command(host: &Host, argv: &[String]) -> Value {
    match argv {
        [flag, .., pick] if flag == "--pick" => on_pick(host, pick),
        [list] if list == "list" || list == "ls" => {
            let session: Vec<String> = state().session.keys().cloned().collect();
            json!({"text": store::table(&session)})
        }
        _ => picker(None),
    }
}

pub fn run_cli(args: &[String]) -> i32 {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("set") => cli::set(rest),
        Some("rm") => cli::rm(rest),
        Some("list") => cli::list(),
        Some("exec") => exec::run(rest),
        Some("elevate-setup") => cli::elevate_setup(),
        Some("reap") => reap(rest),
        None | Some("help" | "--help" | "-h") => {
            println!("{}", cli::USAGE);
            0
        }
        Some(other) => {
            eprintln!("unknown command {other:?}\n{}", cli::USAGE);
            2
        }
    }
}
