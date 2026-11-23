use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use super::mask::{MIN_LEN, Masker, Stream};
use super::{store, util};

pub const SOCKET_ENV: &str = "GRAY_SECRETS_SOCKET";
pub const TOKEN_ENV: &str = "GRAY_SECRETS_TOKEN";

const USAGE: &str = "usage: gray secrets exec --env NAME[,NAME] [--file NAME] -- CMD...";

fn session_values(names: &[String]) -> BTreeMap<String, String> {
    let (Ok(socket), Ok(token)) = (std::env::var(SOCKET_ENV), std::env::var(TOKEN_ENV)) else {
        return BTreeMap::new();
    };
    let fetch = || -> std::io::Result<BTreeMap<String, String>> {
        let mut stream = UnixStream::connect(&socket)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        writeln!(stream, "{}", json!({"token": token, "names": names}))?;
        let mut line = String::new();
        BufReader::new(stream.take(1 << 20)).read_line(&mut line)?;
        let reply: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        Ok(reply
            .get("values")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default())
    };
    fetch().unwrap_or_else(|e| {
        eprintln!("gray-secrets: session secrets unavailable ({e})");
        BTreeMap::new()
    })
}

fn pump(mut from: impl Read, mut to: impl Write, masker: &Masker) {
    let mut stream = Stream::new(masker);
    let mut buf = [0u8; 8192];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let out = stream.push(&buf[..n]);
                if to.write_all(&out).and_then(|_| to.flush()).is_err() {
                    break;
                }
            }
        }
    }
    let _ = to.write_all(&stream.finish());
    let _ = to.flush();
}

pub fn run(args: &[String]) -> i32 {
    let mut env_names: Vec<String> = Vec::new();
    let mut file_names: Vec<String> = Vec::new();
    let mut it = args.iter();
    let mut cmd: Vec<String> = Vec::new();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        match flag {
            "--env" | "--file" => {
                let Some(list) = inline.or_else(|| it.next().cloned()) else {
                    eprintln!("{USAGE}");
                    return 2;
                };
                let into = if flag == "--env" {
                    &mut env_names
                } else {
                    &mut file_names
                };
                into.extend(
                    list.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                );
            }
            "--" => {
                cmd = it.cloned().collect();
                break;
            }
            _ => {
                eprintln!("{USAGE}");
                return 2;
            }
        }
    }
    let mut names: Vec<String> = env_names.iter().chain(&file_names).cloned().collect();
    names.sort();
    names.dedup();
    if cmd.is_empty() || names.is_empty() {
        eprintln!("{USAGE}");
        return 2;
    }
    if let Some(bad) = names.iter().find(|n| !util::valid_name(n)) {
        eprintln!("gray-secrets: invalid secret name {bad:?}");
        return 2;
    }
    let session = util::session_id();
    let mut values = session_values(&names);
    let mut permanent = Vec::new();
    for name in &names {
        if values.contains_key(name) {
            continue;
        }
        match store::lookup(name) {
            Some(v) => {
                values.insert(name.clone(), v);
                permanent.push(name.clone());
            }
            None => {
                eprintln!(
                    "gray-secrets: secret {name} is not set; run `gray secrets set {name}` in a terminal"
                );
                util::audit(&session, name, "deny", "(not set)");
                return 1;
            }
        }
    }
    for (name, value) in &values {
        if value.len() < MIN_LEN {
            eprintln!(
                "gray-secrets: {name} is shorter than {MIN_LEN} characters, so it is not masked in output"
            );
        }
    }
    let masker = Masker::new(values.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let masked_cmd = masker.mask(&cmd.join(" "));

    let mut child_cmd = Command::new(&cmd[0]);
    child_cmd
        .args(&cmd[1..])
        .env_remove(SOCKET_ENV)
        .env_remove(TOKEN_ENV)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in &env_names {
        child_cmd.env(name, &values[name]);
    }
    let mut file_dir = None;
    if !file_names.is_empty() {
        let Some(rt) = util::runtime_dir() else {
            eprintln!("gray-secrets: --file needs XDG_RUNTIME_DIR");
            return 1;
        };
        let dir = match util::random_hex(8) {
            Ok(r) => rt.join(format!("gray-secrets-exec-{r}")),
            Err(e) => {
                eprintln!("gray-secrets: {e}");
                return 1;
            }
        };
        if let Err(e) = util::ensure_private_dir(&dir) {
            eprintln!("gray-secrets: could not create {}: {e}", dir.display());
            return 1;
        }
        file_dir = Some(dir.clone());
        for name in &file_names {
            let path = dir.join(name);
            if let Err(e) = util::write_private(&path, values[name].as_bytes()) {
                eprintln!("gray-secrets: could not write the {name} file: {e}");
                let _ = std::fs::remove_dir_all(&dir);
                return 1;
            }
            child_cmd.env(format!("{name}_FILE"), &path);
        }
    }
    let cleanup = || {
        if let Some(dir) = &file_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    };
    let mut child = match child_cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gray-secrets: could not run {}: {e}", cmd[0]);
            cleanup();
            return 127;
        }
    };
    unsafe {
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            libc::signal(sig, libc::SIG_IGN);
        }
    }
    for name in &names {
        util::audit(&session, name, "use", &masked_cmd);
    }
    store::mark_used(&permanent);
    let out = child.stdout.take();
    let err = child.stderr.take();
    let status = std::thread::scope(|s| {
        if let Some(out) = out {
            s.spawn(|| pump(out, std::io::stdout(), &masker));
        }
        if let Some(err) = err {
            s.spawn(|| pump(err, std::io::stderr(), &masker));
        }
        child.wait()
    });
    cleanup();
    match status {
        Ok(st) => st.code().unwrap_or_else(|| {
            use std::os::unix::process::ExitStatusExt;
            128 + st.signal().unwrap_or(0)
        }),
        Err(e) => {
            eprintln!("gray-secrets: wait failed: {e}");
            1
        }
    }
}
