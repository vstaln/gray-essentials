mod auto_continue;
mod cd;
mod copy;
mod detect;
mod export;
mod guard;
mod handoff;
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
mod repguard;
mod secrets;
mod sessions;
mod settings;
mod setup;
mod skills;
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

mod parts;

pub const PLUGIN_NAME: &str = "essentials";
pub const PROTOCOL: &str = "1.1";
pub const COMMAND: &str = "/essentials";

pub const WELCOME: &str = "Welcome to gray by alignment\ngray is a minimal agent that runs tools, edits code, and works with any model provider.";

const USAGE: &str = "/essentials               sessions, features, detected tools, updates (alias: /qol)\n\
/essentials setup [tool]  import rules, skills, MCP servers and a subscription plugin\n\
/essentials undo [n]      revert the last n imports (default 1)\n\
/essentials dismiss <t>   never mention a detected tool again\n\
/essentials scanned       what the detector looked at and found\n\
/essentials update        update provider CLIs, plugins and gray in one go\n\
/essentials first-run     the first-run welcome\n\
/essentials sessions dismiss  clear the interrupted-session hint\n\
\n\
/theme [name]         pick or set a color theme (alias: /palettes)\n\
/permissions [mode]   view or change tool approval mode (aliases: /perms, /access)\n\
/secrets              list secret names, lifetimes and last use (never values)\n\
/skills [name|verb]   list, run, search, install and toggle agent skills\n\
/cd [path]            change gray's working directory and resume here (alias: /chdir)";

const PARTS: &[parts::Part] = &[
    parts::Part {
        name: "quality-of-life",
        default: true,
        description: "the /essentials panel and first-run welcome",
    },
    parts::Part {
        name: "vim",
        default: true,
        description: "vim keybindings in the input box",
    },
    parts::Part {
        name: "themes",
        default: true,
        description: "color themes and palettes",
    },
    parts::Part {
        name: "permissions",
        default: true,
        description: "tool approval modes and the approval prompt",
    },
    parts::Part {
        name: "guard",
        default: true,
        description: "warn about dangerous code patterns in files being written",
    },
    parts::Part {
        name: "repguard",
        default: true,
        description: "nudge the agent out of identical repeated tool calls",
    },
    parts::Part {
        name: "questions",
        default: true,
        description: "the request_user_input tool",
    },
    parts::Part {
        name: "secrets",
        default: true,
        description: "secret storage, masking and the secret_request tool",
    },
    parts::Part {
        name: "skills",
        default: true,
        description: "skill discovery, listing and the skill tools",
    },
    parts::Part {
        name: "auto-continue",
        default: true,
        description: "continue a turn after a usage-limit reset",
    },
    parts::Part {
        name: "cd",
        default: true,
        description: "change the working directory and resume here",
    },
    parts::Part {
        name: "continue",
        default: true,
        description: "hand a chat off to a fresh session with /continue",
    },
    parts::Part {
        name: "copy",
        default: true,
        description: "copy the last reply to the clipboard",
    },
    parts::Part {
        name: "detect",
        default: true,
        description: "notice other agent CLIs worth importing from",
    },
    parts::Part {
        name: "export",
        default: true,
        description: "export the session as markdown or text",
    },
    parts::Part {
        name: "hotkeys",
        default: true,
        description: "show keybindings and their overrides",
    },
    parts::Part {
        name: "intro",
        default: true,
        description: "toggle the logo intro",
    },
    parts::Part {
        name: "links",
        default: true,
        description: "link #123 in replies to the repo's issues",
    },
    parts::Part {
        name: "notify",
        default: true,
        description: "desktop notification when a turn finishes while away",
    },
    parts::Part {
        name: "recap",
        default: true,
        description: "recap what you were doing when you come back",
    },
    parts::Part {
        name: "sessions",
        default: true,
        description: "interrupted-session hint and idle-session pruning",
    },
    parts::Part {
        name: "setup",
        default: true,
        description: "import rules, skills and MCP servers from other tools",
    },
    parts::Part {
        name: "status-command",
        default: true,
        description: "run a shell command for the status line",
    },
    parts::Part {
        name: "suggest",
        default: true,
        description: "suggest the next prompt as ghost text",
    },
    parts::Part {
        name: "templates",
        default: true,
        description: "prompt templates from .gray/prompts as slash commands",
    },
    parts::Part {
        name: "title",
        default: true,
        description: "name sessions automatically",
    },
    parts::Part {
        name: "update",
        default: true,
        description: "update provider CLIs, plugins and gray in one go",
    },
    parts::Part {
        name: "usage",
        default: true,
        description: "token usage and cost figures",
    },
    parts::Part {
        name: "usage-warn",
        default: true,
        description: "warn at 75, 90 and 95 percent of a usage limit",
    },
];

const COMMANDS: &[(&str, &str, &str)] = &[
    (
        "quality-of-life",
        "/essentials",
        "sessions, features, detected tools, and updates",
    ),
    ("quality-of-life", "/qol", "alias for /essentials"),
    (
        "setup",
        "/setup",
        "import rules, skills and MCP servers from other tools",
    ),
    ("usage", "/cost", "show token usage and cost estimates"),
    ("copy", "/copy", "copy the last reply to the clipboard"),
    (
        "continue",
        "/continue",
        "continue a chat in a fresh session",
    ),
    (
        "export",
        "/export",
        "export the session as markdown or text (clipboard or file)",
    ),
    ("hotkeys", "/hotkeys", "show keybindings and your overrides"),
    ("intro", "/intro", "toggle the logo intro"),
    ("cd", "/cd", "change the working directory and resume here"),
    (
        "cd",
        "/chdir",
        "change the working directory and resume here",
    ),
    ("themes", "/theme", "pick or apply a color theme"),
    ("themes", "/palettes", "list or install color palettes"),
    (
        "permissions",
        "/permissions",
        "view or set tool approval mode",
    ),
    ("permissions", "/perms", "alias for /permissions"),
    ("permissions", "/access", "alias for /permissions"),
    (
        "permissions",
        "/confirm",
        "list or toggle dangerous-command prompts",
    ),
    ("permissions", "/gate", "deny tools by name, permanently"),
    (
        "repguard",
        "/repguard",
        "show the repeated-call guard status",
    ),
    (
        "secrets",
        "/secrets",
        "list secret names, lifetimes and last use (never values)",
    ),
    (
        "skills",
        "/skills",
        "list, search, install and toggle skills",
    ),
];

const SETTING_PARTS: &[(&str, &str)] = &[
    ("away_recap", "recap"),
    ("prompt_suggestions", "suggest"),
    ("auto_titles", "title"),
    ("issue_links", "links"),
    ("session_cleanup", "sessions"),
    ("auto_continue", "auto-continue"),
    ("usage_warnings", "usage-warn"),
    ("notify_on_finish", "notify"),
    ("break_reminder", "notify"),
    ("vim_mode", "vim"),
    ("permissions_mode", "permissions"),
];

const TURN_LOCAL: &[(&str, &str, &[&str])] = &[
    ("usage", "/cost", &["*"]),
    ("copy", "/copy", &["*"]),
    ("export", "/export", &["*"]),
    ("hotkeys", "/hotkeys", &["*"]),
    ("intro", "/intro", &["*"]),
    ("quality-of-life", "/essentials", &["scanned"]),
    ("quality-of-life", "/qol", &["scanned"]),
    ("permissions", "/permissions", &["*"]),
    ("permissions", "/perms", &["*"]),
    ("permissions", "/access", &["*"]),
    ("secrets", "/secrets", &["*"]),
];

const COMMAND_COMPLETIONS: &[(&str, &str, &[&str])] = &[
    ("export", "/export", &["clip", "md", "txt", "help"]),
    (
        "secrets",
        "/secrets",
        &["set", "rm", "list", "exec", "elevate-setup"],
    ),
    (
        "permissions",
        "/permissions",
        &["auto", "full", "bypass", "read-only"],
    ),
    (
        "permissions",
        "/perms",
        &["auto", "full", "bypass", "read-only"],
    ),
    (
        "permissions",
        "/access",
        &["auto", "full", "bypass", "read-only"],
    ),
];

fn setting_rows(parts: &parts::Parts) -> Value {
    let rows = settings::manifest_rows();
    Value::Array(
        rows.as_array()
            .into_iter()
            .flatten()
            .filter(|row| {
                let key = row.get("key").and_then(Value::as_str).unwrap_or("");
                SETTING_PARTS
                    .iter()
                    .find(|(k, _)| *k == key)
                    .is_none_or(|(_, part)| parts.on(part))
            })
            .cloned()
            .collect(),
    )
}

fn manifest(parts: &parts::Parts) -> Value {
    let prompts = if parts.on("templates") {
        templates::list()
    } else {
        Vec::new()
    };
    let mut commands: Vec<String> = Vec::new();
    let mut descriptions = serde_json::Map::new();
    for (part, name, description) in COMMANDS {
        if parts.on(part) {
            commands.push((*name).to_string());
            descriptions.insert((*name).to_string(), json!(description));
        }
    }
    let mut turn_local = serde_json::Map::new();
    for (part, name, scopes) in TURN_LOCAL {
        if parts.on(part) {
            turn_local.insert((*name).to_string(), json!(scopes));
        }
    }
    let mut command_completions = serde_json::Map::new();
    for (part, name, words) in COMMAND_COMPLETIONS {
        if parts.on(part) {
            command_completions.insert((*name).to_string(), json!(words));
        }
    }
    if parts.on("skills") {
        command_completions.insert(
            "/skills".to_string(),
            Value::Array(skills::verb_completions()),
        );
    }
    let mut completion: Vec<String> = Vec::new();
    if parts.on("quality-of-life") {
        completion.extend(
            [
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
            .map(|s| (*s).to_string()),
        );
    }
    for template in &prompts {
        let name = format!("/{}", template.name);
        descriptions.insert(name.clone(), json!(template.description));
        commands.push(name);
        completion.push(template.name.clone());
    }
    let mut tools: Vec<Value> = Vec::new();
    if parts.on("questions") {
        tools.push(questions::tool_def());
    }
    if parts.on("secrets") {
        tools.push(secrets::tool_def());
    }
    if parts.on("skills") {
        tools.extend(skills::tool_defs());
    }
    let mut hooks: Vec<&str> = Vec::new();
    if parts.on("sessions")
        || parts.on("detect")
        || parts.on("update")
        || parts.on("links")
        || parts.on("templates")
        || parts.on("status-command")
    {
        hooks.push("session_start");
    }
    if parts.on("status-command")
        || parts.on("usage")
        || parts.on("sessions")
        || parts.on("auto-continue")
        || parts.on("notify")
        || parts.on("title")
        || parts.on("suggest")
        || parts.on("usage-warn")
    {
        hooks.push("turn_end");
    }
    if parts.on("sessions") || parts.on("recap") {
        hooks.push("user_return");
    }
    if parts.on("vim") {
        hooks.push("input/key");
    }
    if parts.on("secrets") || parts.on("permissions") || parts.on("guard") || parts.on("repguard") {
        hooks.push("tool/before");
    }
    if parts.on("secrets") || parts.on("guard") || parts.on("repguard") {
        hooks.push("tool/after");
    }
    if parts.on("questions") || parts.on("skills") {
        hooks.push("prompt/context");
    }
    let mut capabilities: Vec<&str> = Vec::new();
    if parts.on("auto-continue")
        || parts.on("recap")
        || parts.on("setup")
        || parts.on("update")
        || parts.on("usage-warn")
    {
        capabilities.push("host.say");
    }
    if parts.on("links") {
        capabilities.push("host.links");
    }
    if parts.on("setup") || parts.on("update") || parts.on("sessions") || parts.on("skills") {
        capabilities.push("host.turn");
    }
    if parts.on("sessions") || parts.on("update") {
        capabilities.push("host.sessions");
    }
    if parts.on("recap")
        || parts.on("suggest")
        || parts.on("title")
        || parts.on("usage-warn")
        || parts.on("notify")
    {
        capabilities.push("session.read");
    }
    if parts.on("title") {
        capabilities.push("session.write");
    }
    if parts.on("suggest") {
        capabilities.push("composer.ghost");
    }
    if parts.on("setup") {
        capabilities.push("plugins.manage");
    }
    if parts.on("permissions")
        || parts.on("questions")
        || parts.on("secrets")
        || parts.on("cd")
        || parts.on("export")
        || parts.on("setup")
        || parts.on("update")
        || parts.on("secrets")
    {
        capabilities.push("host.ask");
    }
    if parts.on("cd") || parts.on("update") || parts.on("themes") {
        capabilities.push("host.restart");
    }
    let required_hooks: Vec<&str> =
        if parts.on("secrets") || parts.on("guard") || parts.on("repguard") {
            vec!["tool/after"]
        } else {
            Vec::new()
        };
    json!({
        "name": PLUGIN_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": PROTOCOL,
        "description": "Gray essentials: UX bundle, vim mode, themes, permissions, questions, secrets, skills, setup, and updates",
        "tools": tools,
        "commands": commands,
        "command_descriptions": descriptions,
        "turn_local": turn_local,
        "hooks": hooks,
        "required_hooks": required_hooks,
        "first_run": parts.on("quality-of-life") || parts.on("setup"),
        "capabilities": capabilities,
        "settings": setting_rows(parts),
        "completion": completion,
        "command_completions": command_completions,
        "parts": parts.manifest(PARTS),
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

fn input_key(vim_state: &Mutex<VimState>, params: &Value, parts: &parts::Parts) -> Value {
    if !parts.on("vim") {
        return json!({"handled": false});
    }
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

fn command_run(
    host: &Host,
    gate: &permissions::ApprovalGate,
    params: &Value,
    parts: &parts::Parts,
) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(COMMAND);
    let argv = argv_of(params);
    if let Some(owner) = COMMANDS
        .iter()
        .find(|(_, cmd, _)| *cmd == name)
        .map(|(part, _, _)| *part)
        && !parts.on(owner)
    {
        return json!({"text": format!("unknown command {name}")});
    }
    if name == "/repguard" && parts.on("repguard") {
        return json!({"text": repguard::command(&argv, params)});
    }
    if (name == "/theme" || name == "/palettes") && parts.on("themes") {
        return themes::command(host, name, &argv);
    }
    if permissions::COMMANDS.contains(&name) && parts.on("permissions") {
        return permissions::command(name, gate, &argv);
    }
    if name == "/secrets" && parts.on("secrets") {
        return secrets::command(host, &argv);
    }
    if name == skills::COMMAND && parts.on("skills") {
        return skills::command(params);
    }
    if name == "/setup" && parts.on("setup") {
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
    if (name == "/cd" || name == "/chdir") && parts.on("cd") {
        return cd::command(host, &argv);
    }
    let session = params["session"]["id"].as_str().unwrap_or("").to_string();
    let words: Vec<&str> = argv.iter().map(String::as_str).collect();
    match name {
        "/cost" if parts.on("usage") => return usage::usage_command(&argv),
        "/copy" if parts.on("copy") => return copy::copy_command(&session),
        "/continue" if parts.on("continue") => return handoff::command(params),
        "/export" if parts.on("export") => {
            return export::export_command(Some(host), &session, &argv);
        }
        "/hotkeys" if parts.on("hotkeys") => return hotkeys::hotkeys_command(&argv),
        "/intro" if parts.on("intro") => return intro::intro_command(&argv),
        _ => {}
    }
    let rest = params.get("raw").and_then(Value::as_str).unwrap_or("");
    if parts.on("templates")
        && let Some(outcome) = templates::run(&templates::list(), name, rest)
    {
        return outcome;
    }
    if name == "/update" && parts.on("update") {
        return match words.first() {
            Some(&"everything") => update::run(host, &session),
            _ => json!({"text": "usage: /update everything"}),
        };
    }
    if !parts.on("quality-of-life") {
        return json!({"text": format!("unknown command {name}")});
    }
    match words.as_slice() {
        [] => panel::open(host, params),
        ["--pick", .., pick] => panel::pick(host, params, &session, pick),
        ["help"] => json!({"text": USAGE}),
        ["setup", rest @ ..] if parts.on("setup") => setup::command(host, rest.first().copied()),
        ["cd", ..] | ["chdir", ..] if parts.on("cd") => cd::command(host, &argv[1..]),
        ["undo"] if parts.on("setup") => setup::undo(1),
        ["undo", n] if parts.on("setup") => match n.parse::<usize>() {
            Ok(n) if n > 0 => setup::undo(n),
            _ => json!({"text": "usage: /essentials undo [n]"}),
        },
        ["dismiss", tool] if parts.on("setup") => setup::dismiss(tool),
        ["dismiss"] if parts.on("setup") => json!({"text": "usage: /essentials dismiss <tool>"}),
        ["scanned"] if parts.on("detect") => detect::scanned_report(),
        ["update", ..] if parts.on("update") => update::run(host, &session),
        ["first-run"] if parts.on("setup") => setup::first_run(host),
        ["sessions", "dismiss"] if parts.on("sessions") => sessions::dismiss_command(host, params),
        ["sessions"] if parts.on("sessions") => {
            json!({"text": "usage: /essentials sessions dismiss"})
        }
        _ => json!({"text": format!("unknown: {name} {}\n{USAGE}", argv.join(" "))}),
    }
}

fn on_event(host: &Arc<Host>, params: &Value, parts: &parts::Parts) {
    if parts.on("permissions") {
        permissions::note_host_event(params);
    }
    match params.get("type").and_then(Value::as_str) {
        Some("session_start") => {
            if parts.on("status-command") {
                status_command::on_event(host, params);
            }
            if parts.on("templates") {
                templates::session_start(host, params);
            }
            if parts.on("links") {
                links::session_start(host, params);
            }
            if prompt::interactive(params) {
                if parts.on("sessions") {
                    sessions::session_start(host, params);
                }
                if parts.on("detect") {
                    detect::notify(host);
                }
                if parts.on("update") {
                    let host = Arc::clone(host);
                    let _ = std::thread::spawn(move || update::notify(&host));
                }
            }
        }
        Some("user_return") => {
            if parts.on("sessions") {
                sessions::refresh(host, params);
            }
            if parts.on("recap") {
                recap::user_return(host, params);
            }
        }
        Some("turn_end") => {
            if parts.on("status-command") {
                status_command::on_event(host, params);
            }
            if parts.on("usage") {
                usage::record_turn(&prompt::session_id(params), &params["usage"]);
            }
            if parts.on("sessions") {
                sessions::refresh(host, params);
            }
            if parts.on("auto-continue") {
                auto_continue::turn_end(host, params);
            }
            if parts.on("notify") {
                notify::turn_end(host, params);
            }
            let mut workers: Vec<fn(&Host, &Value)> = Vec::new();
            if parts.on("title") {
                workers.push(title::turn_end);
            }
            if parts.on("suggest") {
                workers.push(suggest::turn_end);
            }
            if parts.on("usage-warn") {
                workers.push(usage_warn::turn_end);
            }
            for worker in workers {
                let host = Arc::clone(host);
                let params = params.clone();
                let _ = std::thread::spawn(move || worker(&host, &params));
            }
        }
        _ => {}
    }
}

fn tool_before(
    host: &Host,
    gate: &permissions::ApprovalGate,
    params: &Value,
    parts: &parts::Parts,
) -> Value {
    if parts.on("guard") {
        let verdict = guard::tool_before(params);
        if verdict.get("decision").and_then(Value::as_str) == Some("deny") {
            return verdict;
        }
    }
    if parts.on("repguard") {
        repguard::tool_before(params);
    }
    if parts.on("secrets")
        && let Some(denied) = secrets::precheck(params)
    {
        return denied;
    }
    if !parts.on("permissions") {
        return if parts.on("secrets") {
            secrets::tool_before(params)
        } else {
            json!({"decision": "allow"})
        };
    }
    let verdict = permissions::tool_before(host, gate, params);
    if !parts.on("secrets") {
        return verdict;
    }
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

fn merge_after(mut base: Value, next: Value) -> Value {
    let Some(add) = next.get("content").and_then(Value::as_str) else {
        return base;
    };
    if add.is_empty() {
        return base;
    }
    let joined = format!(
        "{}{}",
        base.get("content").and_then(Value::as_str).unwrap_or(""),
        add
    );
    base["content"] = json!(joined);
    base
}

fn handle(
    host: &Host,
    vim_state: &Mutex<VimState>,
    gate: &permissions::ApprovalGate,
    id: Value,
    method: &str,
    params: &Value,
    enabled: &parts::Parts,
) -> Value {
    match method {
        "plugin/manifest" => {
            if enabled.on("skills") {
                skills::publish_completion_for_params(params);
            }
            json!({"id": id, "result": manifest(enabled)})
        }
        "command/run" => json!({"id": id, "result": command_run(host, gate, params, enabled)}),
        "input/key" => json!({"id": id, "result": input_key(vim_state, params, enabled)}),
        "tool/before" => json!({"id": id, "result": tool_before(host, gate, params, enabled)}),
        "tool/after" => {
            let mut result = json!({});
            if enabled.on("repguard") {
                result = merge_after(result, repguard::tool_after(params));
            }
            if enabled.on("guard") {
                result = merge_after(result, guard::tool_after(params));
            }
            if enabled.on("secrets") {
                result = merge_after(result, secrets::tool_after(params));
            }
            json!({"id": id, "result": result})
        }
        "prompt/context" => {
            let mut texts = Vec::new();
            if enabled.on("questions") {
                let guidance = questions::prompt_guidance();
                if !guidance.is_empty() {
                    texts.push(guidance.to_string());
                }
            }
            if enabled.on("skills")
                && let Some(listing) = skills::prompt_context(params)
                && !listing.is_empty()
            {
                texts.push(listing);
            }
            json!({"id": id, "result": {"text": texts.join("

")}})
        }
        "tool/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let result =
                if (name == questions::TOOL_NAME || name.is_empty()) && enabled.on("questions") {
                    questions::tool_call(host, params)
                } else if name == secrets::TOOL_NAME && enabled.on("secrets") {
                    secrets::tool_call(host, params)
                } else if skills::TOOLS.contains(&name) && enabled.on("skills") {
                    skills::tool_call(name, params)
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

fn sidecar(enabled: Arc<parts::Parts>) {
    if enabled.on("secrets") {
        secrets::init();
    }
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
                let enabled = Arc::clone(&enabled);
                workers.push(std::thread::spawn(move || {
                    on_event(&host, &params, &enabled)
                }));
            }
            continue;
        };
        let method = method.to_string();
        let host = host.clone();
        let vim_state = vim_state.clone();
        let gate = gate.clone();
        let enabled = Arc::clone(&enabled);
        workers.push(std::thread::spawn(move || {
            let reply = handle(&host, &vim_state, &gate, id, &method, &params, &enabled);
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
    let enabled = parts::Parts::load("essentials", PARTS);
    let disabled = |part: &str| !enabled.on(part);
    match args.first().map(String::as_str) {
        Some("manifest") => {
            if enabled.on("skills") {
                skills::publish_completion_cwd();
            }
            println!("{}", manifest(&enabled));
        }
        Some("sidecar") => sidecar(Arc::new(enabled)),
        Some("update") if disabled("update") => println!("update is off in this install"),
        Some("update") => update::cli(),
        Some("usage") | Some("cost") if disabled("usage") => {
            println!("usage is off in this install")
        }
        Some("usage") | Some("cost") => {
            println!(
                "{}",
                usage::usage_command(&args[1..])["text"]
                    .as_str()
                    .unwrap_or("")
            );
        }
        Some("copy") if disabled("copy") => println!("copy is off in this install"),
        Some("copy") => println!("{}", copy::copy_command("")["text"].as_str().unwrap_or("")),
        Some("export") if disabled("export") => println!("export is off in this install"),
        Some("export") => println!(
            "{}",
            export::export_command(None, "", &args[1..])["text"]
                .as_str()
                .unwrap_or("")
        ),
        Some("hotkeys") if disabled("hotkeys") => println!("hotkeys is off in this install"),
        Some("hotkeys") => println!(
            "{}",
            hotkeys::hotkeys_command(&args[1..])["text"]
                .as_str()
                .unwrap_or("")
        ),
        Some("intro") if disabled("intro") => println!("intro is off in this install"),
        Some("intro") => println!(
            "{}",
            intro::intro_command(&args[1..])["text"]
                .as_str()
                .unwrap_or("")
        ),
        Some("scanned") if disabled("detect") => println!("detect is off in this install"),
        Some("scanned") => {
            println!(
                "{}",
                detect::scanned_report()["text"].as_str().unwrap_or("")
            )
        }
        Some("secrets") if disabled("secrets") => println!("secrets is off in this install"),
        Some("secrets") => std::process::exit(secrets::run_cli(&args[1..])),
        Some("set" | "rm" | "list" | "exec" | "elevate-setup" | "reap") if disabled("secrets") => {
            println!("secrets is off in this install")
        }
        Some("set" | "rm" | "list" | "exec" | "elevate-setup" | "reap") => {
            std::process::exit(secrets::run_cli(&args))
        }
        Some(_) => println!("{USAGE}"),
        None if std::io::stdin().is_terminal() => {
            eprintln!(
                "gray-essentials is a gray plugin sidecar — install with `gray plugin install`"
            );
        }
        None => sidecar(Arc::new(enabled)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_includes_skills_secrets_and_questions() {
        let m = manifest(&parts::Parts::load("essentials", PARTS));
        assert_eq!(m["name"], "essentials");
        let commands: Vec<&str> = m["commands"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(commands.contains(&"/skills"));
        assert!(commands.contains(&"/secrets"));
        assert!(commands.contains(&"/essentials"));
        let tools: Vec<&str> = m["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(
            tools,
            [
                "request_user_input",
                "secret_request",
                "skill",
                "skills_search",
                "skills_install",
            ]
        );
        assert!(m["command_completions"]["/skills"].is_array());
    }
}
