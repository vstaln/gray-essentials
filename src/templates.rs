use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::host::Host;
use crate::paths;
use crate::prompt;

#[derive(Clone)]
pub struct Template {
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
    pub content: String,
}

static TEMPLATES: Mutex<Vec<Template>> = Mutex::new(Vec::new());
static ANNOUNCED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CWD: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn session_start(host: &Host, params: &Value) {
    if let Some(cwd) = prompt::cwd(params) {
        *CWD.lock().unwrap_or_else(|e| e.into_inner()) = Some(cwd);
    }
    let list = discover();
    let names: Vec<String> = list.iter().map(|t| t.name.clone()).collect();
    let changed = {
        let mut announced = ANNOUNCED.lock().unwrap_or_else(|e| e.into_inner());
        if *announced == names {
            false
        } else {
            *announced = names;
            true
        }
    };
    *TEMPLATES.lock().unwrap_or_else(|e| e.into_inner()) = list;
    if changed {
        host.announce("host/commands_changed");
    }
}

pub fn list() -> Vec<Template> {
    TEMPLATES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn run(templates: &[Template], name: &str, rest: &str) -> Option<Value> {
    let name = name.strip_prefix('/').unwrap_or(name);
    let template = templates
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))?;
    Some(json!({"prompt": expand(template, rest)}))
}

fn current_cwd() -> Option<PathBuf> {
    if let Some(cwd) = CWD
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .cloned()
    {
        return Some(cwd);
    }
    std::env::current_dir().ok()
}

fn find_git_root(cwd: &Path) -> Option<PathBuf> {
    let mut cur = Some(cwd);
    while let Some(dir) = cur {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

fn search_dirs(cwd: &Path) -> Vec<PathBuf> {
    let git_root = find_git_root(cwd);
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut cur = Some(cwd.to_path_buf());
    while let Some(dir) = cur {
        dirs.push(dir.join(".gray").join("prompts"));
        if git_root.as_ref().is_none_or(|r| *r == dir) {
            break;
        }
        cur = dir.parent().map(Path::to_path_buf);
    }
    dirs.push(paths::gray_home().join("prompts"));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

fn discover() -> Vec<Template> {
    let Some(cwd) = current_cwd() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dir in search_dirs(&cwd) {
        load_dir(&dir, &mut out);
    }
    out
}

fn load_dir(dir: &Path, out: &mut Vec<Template>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md") && p.is_file())
        .collect();
    paths.sort();
    for path in paths {
        if let Some(t) = load_template(&path)
            && !out.iter().any(|o| o.name == t.name)
        {
            out.push(t);
        }
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_whitespace)
}

#[derive(Default)]
struct Frontmatter {
    description: Option<String>,
    argument_hint: Option<String>,
}

fn parse_frontmatter(content: &str) -> Option<(Frontmatter, String)> {
    let trimmed = content.trim_start();
    let Some(after) = trimmed.strip_prefix("---") else {
        return Some((Frontmatter::default(), content.to_string()));
    };
    let after = after
        .strip_prefix("\r\n")
        .or_else(|| after.strip_prefix('\n'))
        .unwrap_or(after);
    let mut offset = 0;
    for line in after.split_inclusive('\n') {
        if line.trim() == "---" {
            let body = after[offset + line.len()..].to_string();
            return Some((parse_fields(&after[..offset]), body));
        }
        offset += line.len();
    }
    None
}

fn parse_fields(block: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let lines: Vec<&str> = block.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        i += 1;
        if line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let mut val = val.trim().to_string();
        let folded = matches!(val.as_str(), ">" | ">-" | ">+");
        if folded || matches!(val.as_str(), "|" | "|-" | "|+") {
            let mut parts = Vec::new();
            while i < lines.len() {
                let next = lines[i];
                if !next.trim().is_empty() {
                    if !next.starts_with([' ', '\t']) {
                        break;
                    }
                    parts.push(next.trim());
                }
                i += 1;
            }
            val = parts.join(if folded { " " } else { "\n" });
        } else if val.len() >= 2
            && ((val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\'')))
        {
            val = val[1..val.len() - 1].to_string();
        }
        match key.trim().trim_matches(['"', '\'']) {
            "description" => fm.description = Some(val),
            "argument-hint" | "argument_hint" => fm.argument_hint = Some(val),
            _ => {}
        }
    }
    fm
}

fn load_template(path: &Path) -> Option<Template> {
    let name = path.file_stem()?.to_str()?.to_string();
    if !valid_name(&name) {
        return None;
    }
    let raw = std::fs::read_to_string(path).ok()?;
    let (fm, body) = parse_frontmatter(&raw)?;
    let description = fm
        .description
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| {
            let first = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            let first = first.trim().trim_start_matches('#').trim();
            let mut out: String = first.chars().take(60).collect();
            if first.chars().count() > 60 {
                out.push_str("...");
            }
            out
        });
    Some(Template {
        name,
        description,
        argument_hint: fm.argument_hint.filter(|h| !h.trim().is_empty()),
        content: body,
    })
}

fn split_args(rest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in rest.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
                started = true;
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                    started = true;
                } else if c.is_whitespace() {
                    if started {
                        out.push(std::mem::take(&mut cur));
                        started = false;
                    }
                } else {
                    cur.push(c);
                    started = true;
                }
            }
        }
    }
    if started {
        out.push(cur);
    }
    out.retain(|a| !a.is_empty());
    out
}

fn nth<'a>(args: &'a [String], n: &str) -> Option<&'a String> {
    n.parse::<usize>()
        .ok()
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| args.get(i))
}

fn braced(inner: &str, args: &[String], all: &str) -> String {
    if let Some((target, default)) = inner.split_once(":-") {
        let value = match target {
            "@" | "ARGUMENTS" => all.to_string(),
            n => nth(args, n).cloned().unwrap_or_default(),
        };
        return if value.is_empty() {
            default.to_string()
        } else {
            value
        };
    }
    if let Some(rest) = inner.strip_prefix("@:") {
        let (start, len) = match rest.split_once(':') {
            Some((a, b)) => (a, Some(b)),
            None => (rest, None),
        };
        let start = start
            .parse::<usize>()
            .unwrap_or(1)
            .saturating_sub(1)
            .min(args.len());
        let end = match len.and_then(|l| l.parse::<usize>().ok()) {
            Some(l) => start.saturating_add(l).min(args.len()),
            None => args.len(),
        };
        return args[start..end].join(" ");
    }
    format!("${{{inner}}}")
}

fn has_placeholders(content: &str) -> bool {
    let chars: Vec<char> = content.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c != '$' {
            continue;
        }
        match chars.get(i + 1) {
            Some('{') | Some('@') => return true,
            Some(d) if d.is_ascii_digit() => return true,
            Some('A') => {
                let rest: String = chars[i + 1..].iter().take(9).collect();
                if rest == "ARGUMENTS" {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn substitute(content: &str, args: &[String]) -> String {
    let all = args.join(" ");
    let chars: Vec<char> = content.chars().collect();
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        match chars.get(i + 1).copied() {
            Some('{') => {
                let mut j = i + 2;
                let mut inner = String::new();
                let mut closed = false;
                while j < chars.len() {
                    if chars[j] == '}' {
                        closed = true;
                        break;
                    }
                    inner.push(chars[j]);
                    j += 1;
                }
                if !closed {
                    out.push('$');
                    i += 1;
                    continue;
                }
                out.push_str(&braced(&inner, args, &all));
                i = j + 1;
            }
            Some('@') => {
                out.push_str(&all);
                i += 2;
            }
            Some(c) if c.is_ascii_digit() => {
                let mut j = i + 1;
                let mut num = String::new();
                while j < chars.len() && chars[j].is_ascii_digit() {
                    num.push(chars[j]);
                    j += 1;
                }
                out.push_str(nth(args, &num).map(String::as_str).unwrap_or(""));
                i = j;
            }
            Some('A') => {
                let rest: String = chars[i + 1..].iter().take(9).collect();
                if rest == "ARGUMENTS" {
                    out.push_str(&all);
                    i += 10;
                } else {
                    out.push('$');
                    i += 1;
                }
            }
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
    out
}

pub fn expand(template: &Template, rest: &str) -> String {
    let mut args = split_args(rest);
    args.retain(|a| !a.is_empty());
    let body = template.content.trim();
    if has_placeholders(body) {
        substitute(body, &args)
    } else if rest.trim().is_empty() {
        body.to_string()
    } else {
        format!("{body}\n\n{}", rest.trim())
    }
}
