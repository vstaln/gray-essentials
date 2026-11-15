//! /export — the current session's conversation as markdown or plain text,
//! to the clipboard or a file, for carrying the work into another harness.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::copy::{copy_to_clipboard, session_path};
use crate::host::{self, Host};

const ARGS_MAX: usize = 1_500;
const RESULT_MAX: usize = 2_000;

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Markdown,
    Text,
}

enum Target {
    Clipboard,
    File(PathBuf),
}

pub const USAGE: &str = "/export            pick: copy to clipboard or save to a file\n\
/export clip       copy the session as markdown to the clipboard\n\
/export md | txt   write ./gray-<session>.md or .txt\n\
/export <file>     write to a file (.txt = plain text, anything else = markdown)";

pub fn export_command(host: Option<&Host>, session_id: &str, args: &[String]) -> Value {
    let arg = args.join(" ");
    let arg = arg.trim();
    if arg == "help" {
        return json!({"text": USAGE});
    }
    let Some(path) = session_path(session_id) else {
        return json!({"text": "nothing to export — no session file found"});
    };
    let sid = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();
    let picked = match (arg, host) {
        ("", Some(host)) => match pick(host) {
            Some(p) => p,
            None => return json!({"text": "export cancelled"}),
        },
        _ => arg,
    };
    let (format, target) = match picked {
        "" | "clip" | "clipboard" => (Format::Markdown, Target::Clipboard),
        "md" | "markdown" => (
            Format::Markdown,
            Target::File(here(&format!("gray-{sid}.md"))),
        ),
        "txt" | "text" => (Format::Text, Target::File(here(&format!("gray-{sid}.txt")))),
        file => {
            let p = expand(file);
            let format = if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt")) {
                Format::Text
            } else {
                Format::Markdown
            };
            (format, Target::File(p))
        }
    };
    let Some((header, entries)) = load(&path) else {
        return json!({"text": "nothing to export — the session file is unreadable"});
    };
    let (body, turns) = render(&header, &entries, format);
    if turns == 0 {
        return json!({"text": "nothing to export yet — no messages this session"});
    }
    let chars = body.chars().count();
    match target {
        Target::Clipboard if copy_to_clipboard(&body) => json!({"text": format!(
            "copied the session to the clipboard: {turns} messages, {chars} chars of markdown"
        )}),
        Target::Clipboard => {
            let p = here(&format!("gray-{sid}.md"));
            write(&p, &body, turns, chars, "no clipboard available — ")
        }
        Target::File(p) => write(&p, &body, turns, chars, ""),
    }
}

fn pick(host: &Host) -> Option<&'static str> {
    let questions = json!([{
        "id": "export",
        "header": "Export conversation",
        "question": "Select export method",
        "options": [
            {"label": "Copy to clipboard", "description": "Copy the conversation to your system clipboard"},
            {"label": "Save to file", "description": "Save the conversation to a file in the current directory"}
        ],
        "no_other": true
    }]);
    let answer = host.ask(questions).ok()?;
    let labels = host::answer_labels(&answer, "export")?;
    match labels.first()?.as_str() {
        "Copy to clipboard" => Some("clip"),
        "Save to file" => Some("md"),
        _ => None,
    }
}

fn write(p: &Path, body: &str, turns: usize, chars: usize, lead: &str) -> Value {
    if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(p, body) {
        Ok(()) => json!({"text": format!(
            "{lead}exported {turns} messages ({chars} chars) to {}",
            crate::paths::tilde(p)
        )}),
        Err(e) => json!({"text": format!("{lead}could not write {}: {e}", p.display())}),
    }
}

fn here(name: &str) -> PathBuf {
    std::env::current_dir().unwrap_or_default().join(name)
}

fn expand(file: &str) -> PathBuf {
    match file.strip_prefix('~') {
        Some(rest) => crate::paths::home().join(rest.trim_start_matches(['/', '\\'])),
        None if Path::new(file).is_absolute() => PathBuf::from(file),
        None => here(file),
    }
}

fn load(path: &Path) -> Option<(Value, Vec<Value>)> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut header = Value::Null;
    let mut by_id: HashMap<u64, Value> = HashMap::new();
    let mut last = None;
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("entry_id").and_then(Value::as_u64) {
            Some(id) => {
                last = Some(id);
                by_id.insert(id, v);
            }
            None if header.is_null() => header = v,
            None => {}
        }
    }
    let mut chain = Vec::new();
    let mut at = last;
    while let Some(id) = at {
        let Some(entry) = by_id.remove(&id) else {
            break;
        };
        at = entry.get("parent_id").and_then(Value::as_u64);
        chain.push(entry);
    }
    chain.reverse();
    Some((header, chain))
}

fn render(header: &Value, entries: &[Value], format: Format) -> (String, usize) {
    let mut out = String::new();
    let sid = header["id"].as_str().unwrap_or("session");
    match format {
        Format::Markdown => {
            out.push_str(&format!("# Gray session `{sid}`\n\n"));
            for (k, label) in [("cwd", "Directory"), ("model", "Model")] {
                if let Some(v) = header[k].as_str() {
                    out.push_str(&format!("- {label}: `{v}`\n"));
                }
            }
            out.push('\n');
        }
        Format::Text => {
            out.push_str(&format!("Gray session {sid}\n"));
            for (k, label) in [("cwd", "Directory"), ("model", "Model")] {
                if let Some(v) = header[k].as_str() {
                    out.push_str(&format!("{label}: {v}\n"));
                }
            }
            out.push('\n');
        }
    }
    let mut turns = 0;
    let mut last_role = "";
    for e in entries {
        if e.get("compaction_boundary").and_then(Value::as_bool) == Some(true) {
            out.push_str(match format {
                Format::Markdown => "---\n\n*(context was compacted here)*\n\n---\n\n",
                Format::Text => "-------- context was compacted here --------\n\n",
            });
            last_role = "";
            continue;
        }
        let msg = &e["message"];
        let role = msg["role"].as_str().unwrap_or("");
        let Some(blocks) = msg["content"].as_array() else {
            continue;
        };
        let mut parts = Vec::new();
        for b in blocks {
            if let Some(p) = block(b, format) {
                parts.push(p);
            }
        }
        if parts.is_empty() || role == "system" {
            continue;
        }
        let is_results = blocks
            .iter()
            .all(|b| b["type"].as_str() == Some("tool_result"));
        let speaker = if role == "assistant" || is_results {
            "Assistant"
        } else {
            "User"
        };
        if speaker != last_role {
            out.push_str(&match format {
                Format::Markdown => format!("## {speaker}\n\n"),
                Format::Text => format!("{}:\n", speaker.to_uppercase()),
            });
            last_role = if speaker == "Assistant" {
                "Assistant"
            } else {
                "User"
            };
        }
        if !is_results {
            turns += 1;
        }
        for p in parts {
            out.push_str(&p);
            out.push_str("\n\n");
        }
    }
    (out.trim_end().to_string() + "\n", turns)
}

fn block(b: &Value, format: Format) -> Option<String> {
    match b["type"].as_str()? {
        "text" => {
            let t = b["text"].as_str()?.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        "image" => Some("[image]".to_string()),
        "tool_use" => {
            let name = b["name"].as_str().unwrap_or("tool");
            let args = b
                .get("args")
                .or_else(|| b.get("input"))
                .cloned()
                .unwrap_or(Value::Null);
            let args = match &args {
                Value::String(s) => s.clone(),
                v => serde_json::to_string_pretty(v).unwrap_or_default(),
            };
            let args = clip(&args, ARGS_MAX);
            Some(match format {
                Format::Markdown => format!("**Tool call:** `{name}`\n\n{}", fenced(&args, "json")),
                Format::Text => format!("[tool call: {name}]\n{}", indent(&args)),
            })
        }
        "tool_result" => {
            let content = match &b["content"] {
                Value::String(s) => s.clone(),
                Value::Array(a) => a
                    .iter()
                    .filter_map(|x| x["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            let content = clip(content.trim(), RESULT_MAX);
            let label = if b["is_error"].as_bool() == Some(true) {
                "error"
            } else {
                "result"
            };
            Some(match format {
                Format::Markdown => format!("**Tool {label}:**\n\n{}", fenced(&content, "")),
                Format::Text => format!("[tool {label}]\n{}", indent(&content)),
            })
        }
        _ => None,
    }
}

fn clip(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}\n… ({} more chars cut)", n - max)
}

fn fenced(s: &str, lang: &str) -> String {
    let mut run = 0;
    let mut longest = 0;
    for c in s.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{lang}\n{s}\n{fence}")
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
