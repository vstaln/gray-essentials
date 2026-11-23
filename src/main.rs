mod auto_continue;
mod cd;
mod copy;
mod detect;
mod export;
mod host;
mod hotkeys;
mod intro;
mod links;
mod notify;
mod panel;
mod paths;
mod permissions;
mod prompt;
mod questions;
mod recap;
mod secrets;
mod sessions;
mod settings;
mod setup;
mod sources;
mod state;
mod status_command;
mod suggest;
mod templates;
mod themes;
mod title;
mod update;
mod usage;
mod usage_warn;
mod vim;

use std::io::{BufRead, IsTerminal};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use host::Host;
use vim::{Press, State as VimState};

pub const PLUGIN_NAME: &str = "extras";
pub const PROTOCOL: &str = "1.1";
pub const COMMAND: &str = "/extras";

pub const WELCOME: &str = "Welcome to gray by alignment\ngray is a minimal agent that runs tools, edits code, and works with any model provider.";

const USAGE: &str = "/extras               sessions, features, detected tools, updates (alias: /qol)\n\
/extras setup [tool]  import rules, skills, MCP servers and a subscription plugin\n\
/extras undo [n]      revert the last n imports (default 1)\n\
/extras dismiss <t>   never mention a detected tool again\n\
/extras scanned       what the detector looked at and found\n\
/extras update        update provider CLIs, plugins and gray in one go\n\
/extras first-run     the first-run welcome\n\
/extras sessions dismiss  clear the interrupted-session hint\n\
\n\
/theme [name]         pick or set a color theme (alias: /palettes)\n\
/permissions [mode]   view or change tool approval mode (aliases: /perms, /access)\n\
/secrets              list secret names, lifetimes and last use (never values)\n\
/cd [path]            change gray's working directory and resume here (alias: /chdir)";

const COMMANDS: &[(&str, &str)] = &[
    (
        "/extras",
        "sessions, features, vim mode, detected tools, and updates",
    ),
    ("/qol", "alias for /extras"),
    (
        "/setup",
        "import rules, skills and MCP servers from other tools",
    ),
    ("/cost", "show token usage and cost estimates"),
    ("/copy", "copy the last reply to the clipboard"),
    (
        "/export",
        "export the session as markdown or text (clipboard or file)",
    ),
    ("/hotkeys", "show keybindings and your overrides"),
    ("/intro", "toggle the logo intro"),
    ("/cd", "change the working directory and resume here"),
    ("/chdir", "change the working directory and resume here"),
    ("/theme", "pick or apply a color theme"),
    ("/palettes", "list or install color palettes"),
    ("/permissions", "view or set tool approval mode"),
    ("/perms", "alias for /permissions"),
    ("/access", "alias for /permissions"),
    (
        "/secrets",
        "list secret names, lifetimes and last use (never values)",
    ),
];

fn manifest() -> Value {
    let prompts = templates::list();
    let mut commands: Vec<String> = COMMANDS
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();
    let mut descriptions = serde_json::Map::new();
    for (name, description) in COMMANDS {
        descriptions.insert((*name).to_string(), json!(description));
    }
    let mut completion: Vec<String> = [
        "setup",
        "undo",
        "dismiss",
        "scanned",
        "update",
        "first-run",
        "cd",
        "sessions",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    for template in &prompts {
        let name = format!("/{}", template.name);
        descriptions.insert(name.clone(), json!(template.description));
        commands.push(name);
        completion.push(template.name.clone());
    }
    json!({
        "name": PLUGIN_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": PROTOCOL,
        "description": "Gray extras: UX bundle, vim mode, themes, permissions, questions, secrets, setup, and updates",
        "tools": [questions::tool_def(), secrets::tool_def()],
        "commands": commands,
        "command_descriptions": descriptions,
        "turn_local": {
            "/cost": ["*"],
            "/copy": ["*"],
            "/export": ["*"],
            "/hotkeys": ["*"],
            "/intro": ["*"],
            "/extras": ["scanned"],
            "/qol": ["scanned"],
            "/permissions": ["*"],
            "/perms": ["*"],
            "/access": ["*"],
            "/secrets": ["*"],
        },
        "hooks": [
            "session_start",
            "turn_end",
            "user_return",
            "input/key",
            "tool/before",
            "tool/after",
            "prompt/context",
        ],
        "required_hooks": ["tool/after"],
        "first_run": true,
        "capabilities": [
            "host.say",
            "host.links",
            "host.turn",
            "host.sessions",
            "session.read",
            "session.write",
            "composer.ghost",
            "plugins.manage",
            "host.ask",
            "host.restart",
            "host.secret",
        ],
        "settings": settings::manifest_rows(),
        "completion": completion,
        "command_completions": {"/export": ["clip", "md", "txt", "help"], "/secrets": ["set", "rm", "list", "exec", "elevate-setup"]},
    })
}

fn argv_of(params: &Value) -> Vec<String> {
    let argv: Vec<String> = params
        .get("argv")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if !argv.is_empty() {
        return argv;
    }
    params
        .get("args")
        .and_then(Value::as_str)
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

fn input_key(vim_state: &Mutex<VimState>, params: &Value) -> Value {
    if !settings::on("vim_mode") {
        return json!({"handled": false});
    }
    let key = params.get("key").cloned().unwrap_or(Value::Null);
    let code = key.get("code").and_then(Value::as_str).unwrap_or("");
    let press = Press {
        code: code.to_string(),
        ctrl: key.get("ctrl").and_then(Value::as_bool).unwrap_or(false),
        alt: key.get("alt").and_then(Value::as_bool).unwrap_or(false),
    };
    let text = params.get("text").and_then(Value::as_str).unwrap_or("");
    let cursor = params.get("cursor").and_then(Value::as_u64).unwrap_or(0) as usize;
    let completing = params
        .get("completing")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut state = vim_state.lock().unwrap_or_else(|e| e.into_inner());
    let reply = state.handle(&press, text, cursor, completing);
    if !reply.handled {
        state.clear_pending();
    }
    reply.to_json()
}

fn command_run(host: &Host, gate: &permissions::ApprovalGate, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(COMMAND);
    let argv = argv_of(params);
    if name == "/theme" || name == "/palettes" {
        return themes::command(host, name, &argv);
    }
    if permissions::COMMANDS.contains(&name) {
        return permissions::command(gate, &argv);
    }
    if name == "/secrets" {
        return secrets::command();
    }
    if name == "/setup" {
        let words: Vec<&str> = argv.iter().map(String::as_str).collect();
        return match words.as_slice() {
            ["undo"] => setup::undo(1),
            ["undo", n] => match n.parse::<usize>() {
                Ok(n) if n > 0 => setup::undo(n),
                _ => json!({"text": "usage: /setup undo [n]"}),
            },
            ["dismiss", tool] => setup::dismiss(tool),
            ["dismiss"] => json!({"text": "usage: /setup dismiss <tool>"}),
            ["scanned"] => detect::scanned_report(),
            _ => setup::command(host, words.first().copied()),
        };
    }
    if name == "/cd" || name == "/chdir" {
        return cd::command(host, &argv);
    }
    let session = params["session"]["id"].as_str().unwrap_or("").to_string();
    let words: Vec<&str> = argv.iter().map(String::as_str).collect();
    match name {
        "/cost" => return usage::usage_command(&argv),
        "/copy" => return copy::copy_command(&session),
        "/export" => return export::export_command(Some(host), &session, &argv),
        "/hotkeys" => return hotkeys::hotkeys_command(&argv),
        "/intro" => return intro::intro_command(&argv),
        _ => {}
    }
    let rest = params.get("raw").and_then(Value::as_str).unwrap_or("");
    if let Some(outcome) = templates::run(&templates::list(), name, rest) {
        return outcome;
    }
    if name == "/update" {
        return match words.first() {
            Some(&"everything") => update::run(host, &session),
            _ => json!({"text": "usage: /update everything"}),
        };
    }
    match words.as_slice() {
        [] => panel::open(host, params),
        ["--pick", .., pick] => panel::pick(host, params, &session, pick),
        ["help"] => json!({"text": USAGE}),
        ["setup", rest @ ..] => setup::command(host, rest.first().copied()),
        ["cd", ..] | ["chdir", ..] => cd::command(host, &argv[1..]),
        ["undo"] => setup::undo(1),
        ["undo", n] => match n.parse::<usize>() {
            Ok(n) if n > 0 => setup::undo(n),
            _ => json!({"text": "usage: /extras undo [n]"}),
        },
        ["dismiss", tool] => setup::dismiss(tool),
        ["dismiss"] => json!({"text": "usage: /extras dismiss <tool>"}),
        ["scanned"] => detect::scanned_report(),
        ["update", ..] => update::run(host, &session),
        ["first-run"] => setup::first_run(host),
        ["sessions", "dismiss"] => sessions::dismiss_command(host, params),
        ["sessions"] => json!({"text": "usage: /extras sessions dismiss"}),
        _ => json!({"text": format!("unknown: {name} {}\n{USAGE}", argv.join(" "))}),
    }
}

fn on_event(host: &Arc<Host>, params: &Value) {
    permissions::note_host_event(params);
    match params.get("type").and_then(Value::as_str) {
        Some("session_start") => {
            status_command::on_event(host, params);
            templates::session_start(host, params);
            links::session_start(host, params);
            if prompt::interactive(params) {
                sessions::session_start(host, params);
                detect::notify(host);
                let host = Arc::clone(host);
                let _ = std::thread::spawn(move || update::notify(&host));
            }
        }
        Some("user_return") => {
            sessions::refresh(host, params);
            recap::user_return(host, params);
        }
        Some("turn_end") => {
            status_command::on_event(host, params);
            usage::record_turn(&prompt::session_id(params), &params["usage"]);
            sessions::refresh(host, params);
            auto_continue::turn_end(host, params);
            notify::turn_end(host, params);
            for worker in [title::turn_end, suggest::turn_end, usage_warn::turn_end] {
                let host = Arc::clone(host);
                let params = params.clone();
                let _ = std::thread::spawn(move || worker(&host, &params));
            }
        }
        _ => {}
    }
}

fn tool_before(host: &Host, gate: &permissions::ApprovalGate, params: &Value) -> Value {
    if let Some(denied) = secrets::precheck(params) {
        return denied;
    }
    let verdict = permissions::tool_before(host, gate, params);
    match verdict.get("decision").and_then(Value::as_str) {
        Some("allow") => secrets::tool_before(params),
        Some("modify") => {
            let mut modified = params.clone();
            if let Some(args) = verdict.get("args") {
                modified["args"] = args.clone();
            }
            let after = secrets::tool_before(&modified);
            if after.get("decision").and_then(Value::as_str) == Some("allow") {
                verdict
            } else {
                after
            }
        }
        _ => verdict,
    }
}

fn handle(
    host: &Host,
    vim_state: &Mutex<VimState>,
    gate: &permissions::ApprovalGate,
    id: Value,
    method: &str,
    params: &Value,
) -> Value {
    match method {
        "plugin/manifest" => json!({"id": id, "result": manifest()}),
        "command/run" => json!({"id": id, "result": command_run(host, gate, params)}),
        "input/key" => json!({"id": id, "result": input_key(vim_state, params)}),
        "tool/before" => json!({"id": id, "result": tool_before(host, gate, params)}),
        "tool/after" => json!({"id": id, "result": secrets::tool_after(params)}),
        "prompt/context" => json!({"id": id, "result": {"text": questions::prompt_guidance()}}),
        "tool/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let result = if name == questions::TOOL_NAME || name.is_empty() {
                questions::tool_call(host, params)
            } else if name == secrets::TOOL_NAME {
                secrets::tool_call(host, params)
            } else {
                json!({"content": format!("unknown tool: {name}"), "is_error": true})
            };
            json!({"id": id, "result": result})
        }
        other => json!({"id": id, "error": {
            "code": -32601,
            "message": format!("method not found: {other}")
        }}),
    }
}

fn sidecar() {
    secrets::init();
    let host = Arc::new(Host::new());
    let vim_state = Arc::new(Mutex::new(VimState::default()));
    let gate = Arc::new(permissions::new_gate());
    let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let method = v.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "plugin/shutdown" {
            secrets::cleanup();
            if let Some(id) = v.get("id").cloned() {
                host.write(&json!({"id": id, "result": {}}));
            }
            break;
        }
        if method.is_empty() {
            if let Some(id) = v.get("id").and_then(Value::as_str) {
                host.deliver(id, v.clone());
            }
            continue;
        }
        let params = v.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = v.get("id").cloned() else {
            if method == "event/notify" {
                let host = host.clone();
                workers.push(std::thread::spawn(move || on_event(&host, &params)));
            }
            continue;
        };
        let method = method.to_string();
        let host = host.clone();
        let vim_state = vim_state.clone();
        let gate = gate.clone();
        workers.push(std::thread::spawn(move || {
            let reply = handle(&host, &vim_state, &gate, id, &method, &params);
            host.write(&reply);
        }));
    }
    host.close();
    for w in workers {
        let _ = w.join();
    }
    secrets::cleanup();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("manifest") => println!("{}", manifest()),
        Some("sidecar") => sidecar(),
        Some("update") => update::cli(),
        Some("usage") | Some("cost") => {
            println!(
                "{}",
                usage::usage_command(&args[1..])["text"]
                    .as_str()
                    .unwrap_or("")
            );
        }
        Some("copy") => println!("{}", copy::copy_command("")["text"].as_str().unwrap_or("")),
        Some("export") => println!(
            "{}",
            export::export_command(None, "", &args[1..])["text"]
                .as_str()
                .unwrap_or("")
        ),
        Some("hotkeys") => println!(
            "{}",
            hotkeys::hotkeys_command(&args[1..])["text"]
                .as_str()
                .unwrap_or("")
        ),
        Some("intro") => println!(
            "{}",
            intro::intro_command(&args[1..])["text"]
                .as_str()
                .unwrap_or("")
        ),
        Some("scanned") => {
            println!(
                "{}",
                detect::scanned_report()["text"].as_str().unwrap_or("")
            )
        }
        Some("secrets") => std::process::exit(secrets::run_cli(&args[1..])),
        Some("set" | "rm" | "list" | "exec" | "elevate-setup" | "reap") => {
            std::process::exit(secrets::run_cli(&args))
        }
        Some(_) => println!("{USAGE}"),
        None if std::io::stdin().is_terminal() => {
            eprintln!("gray-extras is a gray plugin sidecar — install with `gray plugin install`");
        }
        None => sidecar(),
    }
}
