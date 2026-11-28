use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const REQUEST_CHARS: usize = 400;
const REPLY_CHARS: usize = 1_200;
const TITLE_CHARS: usize = 60;
const MAX_TITLE_CHARS: usize = 80;

const SYNTHETIC_USER_PREFIXES: &[&str] = &[
    "[Context update]",
    "<system-reminder>",
    "Subagent finished",
    "[Harness note]",
    "The conversation history before this point was compacted",
];

pub fn command(params: &Value) -> Value {
    let root = crate::paths::gray_home().join("sessions");
    let current = params["session"]["id"].as_str().unwrap_or("");
    let cwd = params["session"]["cwd"]
        .as_str()
        .filter(|c| !c.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let raw = params.get("raw").and_then(Value::as_str).unwrap_or("");
    let (source, extra) = split_source(&root, raw);

    let text = match source {
        Some(id) => build(&root, &id),
        None if valid_id(current) && has_real_turns(&root, current) => build(&root, current),
        None => recalled(&root, &cwd)
            .and_then(|id| build(&root, &id))
            .or_else(|| latest_here(&root, &cwd, current)),
    };
    match text {
        Some(text) => {
            let text = match extra {
                Some(extra) => format!("{text}\n\n{extra}"),
                None => text,
            };
            json!({"slash": format!("/new {text}")})
        }
        None => json!({"text": "nothing to continue"}),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn session_file(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.jsonl"))
}

fn split_source(root: &Path, raw: &str) -> (Option<String>, Option<String>) {
    let rest = raw.trim();
    if rest.is_empty() {
        return (None, None);
    }
    let (token, tail) = match rest.split_once(char::is_whitespace) {
        Some((t, r)) => (t, r.trim()),
        None => (rest, ""),
    };
    let tail = (!tail.is_empty()).then(|| tail.to_string());
    let lower = token.to_lowercase();
    if valid_id(&lower) && session_file(root, &lower).is_file() {
        return (Some(lower), tail);
    }
    if lower.chars().count() >= 8
        && lower.contains('-')
        && valid_id(&lower)
        && let Some(id) = unique_prefix(root, &lower)
    {
        return (Some(id), tail);
    }
    (None, Some(rest.to_string()))
}

fn unique_prefix(root: &Path, prefix: &str) -> Option<String> {
    let mut hits = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".jsonl").map(str::to_string)
        })
        .filter(|id| id.starts_with(prefix));
    match (hits.next(), hits.next()) {
        (Some(id), None) => Some(id),
        _ => None,
    }
}

fn fnv1a_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn recalled(root: &Path, cwd: &Path) -> Option<String> {
    let current = canonical(cwd);
    let path = root
        .join("remembered")
        .join(format!("{}.txt", fnv1a_hex(&current.display().to_string())));
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let stored = PathBuf::from(lines.next()?.trim());
    if canonical(&stored) != current && stored != current {
        return None;
    }
    let id = lines.next()?.trim().to_string();
    (valid_id(&id) && session_file(root, &id).is_file()).then_some(id)
}

fn header_cwd(path: &Path) -> Option<PathBuf> {
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(file).read_line(&mut line).ok()?;
    let header: Value = serde_json::from_str(line.trim()).ok()?;
    header["cwd"].as_str().map(PathBuf::from)
}

fn latest_here(root: &Path, cwd: &Path, skip: &str) -> Option<String> {
    let here = canonical(cwd);
    let mut files: Vec<(std::time::SystemTime, String)> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".jsonl")?.to_string();
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, id))
        })
        .filter(|(_, id)| id != skip && valid_id(id))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().find_map(|(_, id)| {
        let stored = header_cwd(&session_file(root, &id))?;
        if canonical(&stored) != here && stored != here {
            return None;
        }
        let entries = load(&session_file(root, &id))?;
        let first = entries.iter().find_map(real_user_text)?;
        if auxiliary(&first) {
            return None;
        }
        build(root, &id)
    })
}

fn auxiliary(first: &str) -> bool {
    let first = first.trim_start();
    let head: String = first.chars().take(160).collect();
    first.starts_with("[Background task notification]")
        || head.contains("subagent of a parent Gray")
        || (head.starts_with("You are") && head.contains(" worker"))
}

fn load(path: &Path) -> Option<Vec<Value>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut by_id: HashMap<u64, Value> = HashMap::new();
    let mut last = None;
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(id) = v.get("entry_id").and_then(Value::as_u64) {
            last = Some(id);
            by_id.insert(id, v);
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
    Some(chain)
}

fn has_real_turns(root: &Path, id: &str) -> bool {
    load(&session_file(root, id)).is_some_and(|e| e.iter().any(|e| real_user_text(e).is_some()))
}

fn text_content(message: &Value) -> String {
    message["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn real_user_text(entry: &Value) -> Option<String> {
    let message = &entry["message"];
    if message["role"] != "user" || message["injected"] == true {
        return None;
    }
    let blocks = message["content"].as_array()?;
    if blocks.iter().any(|b| b["type"] == "tool_result") {
        return None;
    }
    let mut text = text_content(message);
    if text.is_empty() {
        if blocks.iter().any(|b| b["type"] == "image") {
            text = "[Image]".to_string();
        } else if blocks.iter().any(|b| b["type"] == "media") {
            text = "[File]".to_string();
        }
    }
    let head = text.trim_start();
    if head.is_empty() || SYNTHETIC_USER_PREFIXES.iter().any(|p| head.starts_with(p)) {
        return None;
    }
    Some(text)
}

fn read_title(root: &Path, id: &str) -> Option<String> {
    let raw = std::fs::read_to_string(root.join(format!("{id}.title"))).ok()?;
    let title: String = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    (!title.is_empty()).then_some(title)
}

fn build(root: &Path, id: &str) -> Option<String> {
    let entries = load(&session_file(root, id))?;
    let start = match entries
        .iter()
        .rposition(|e| e["compaction_boundary"] == true)
    {
        Some(i) if i + 1 < entries.len() => i + 1,
        _ => 0,
    };
    let active = &entries[start..];
    let first_user = active.iter().find_map(real_user_text);
    let last_user = active.iter().rev().find_map(real_user_text);
    let last_reply = active.iter().rev().find_map(|e| {
        if e["message"]["role"] != "assistant" {
            return None;
        }
        let text = text_content(&e["message"]).trim().to_string();
        let entry_id = e["entry_id"].as_u64()?;
        (!text.is_empty()).then_some((entry_id, text))
    });

    let title = read_title(root, id).or_else(|| first_user.as_deref().map(short_title));
    let mut out = match &title {
        Some(t) => format!("Continue from the \"{t}\" chat ({id})."),
        None => format!("Continue from chat {id}."),
    };
    let mut quoted_something = false;
    if let Some(text) = last_user {
        let (head, cut) = head_slice(&text, REQUEST_CHARS);
        out.push_str("\n\nLast request:\n");
        out.push_str(&quoted(&head, false, cut));
        quoted_something = true;
    }
    if let Some((entry_id, text)) = last_reply {
        let (tail, cut) = tail_slice(&text, REPLY_CHARS);
        out.push_str("\n\nWhere it left off (end of the last reply):\n");
        out.push_str(&quoted(&tail, cut, false));
        out.push_str(&format!(
            "\n\nEarlier turns: `kit recall --id {id}:{entry_id} --around 5` (raise --around for more)."
        ));
        quoted_something = true;
    }
    quoted_something.then_some(out)
}

fn short_title(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(TITLE_CHARS)
        .collect()
}

fn head_slice(text: &str, max: usize) -> (String, bool) {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    (head, chars.next().is_some())
}

fn tail_slice(text: &str, max: usize) -> (String, bool) {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return (text.to_string(), false);
    }
    let mut start = chars.len() - max;
    if let Some(off) = chars[start..].iter().position(|c| c.is_whitespace())
        && start + off + 1 < chars.len()
    {
        start += off + 1;
    }
    (chars[start..].iter().collect(), true)
}

fn quoted(body: &str, leading_ellipsis: bool, trailing_ellipsis: bool) -> String {
    let mut text = body.replace("**", "");
    if leading_ellipsis {
        text.insert(0, '…');
    }
    if trailing_ellipsis {
        text.push('…');
    }
    text.lines()
        .map(|line| {
            if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
