use serde_json::{Value, json};

use crate::detect;
use crate::host::Host;
use crate::state::State;
use crate::{sessions, settings, setup, sources, update};

const ARROW: &str = " \u{203a}";
const ON: &str = "\u{2713} on";

struct Rows {
    items: Vec<Value>,
    gap: bool,
}

impl Rows {
    fn push(&mut self, id: String, label: &str, value: String) {
        let mut item = json!({"id": id, "label": label, "description": value});
        if std::mem::take(&mut self.gap) && !self.items.is_empty() {
            item["gap_before"] = json!(true);
        }
        self.items.push(item);
    }

    fn section(&mut self) {
        self.gap = true;
    }
}

fn key(key: char, label: &str) -> Value {
    json!({"key": key.to_string(), "label": label})
}

fn picker(title: &str, rows: Rows, keys: Vec<Value>, at: Option<&str>) -> Value {
    let mut spec = json!({
        "title": title,
        "items": rows.items,
        "enter": "select",
        "keys": keys,
    });
    if let Some(id) = at {
        spec["selected_id"] = json!(id);
        spec["cursor_only"] = json!(true);
    }
    json!({"picker": spec})
}

pub fn open(host: &Host, params: &Value) -> Value {
    open_at(host, params, None)
}

fn open_at(host: &Host, params: &Value, at: Option<&str>) -> Value {
    sessions::refresh(host, params);
    let state = State::load();
    let mut rows = Rows {
        items: Vec::new(),
        gap: false,
    };
    let interrupted = sessions::interrupted_count(host, params);
    if interrupted > 0 {
        rows.push(
            "sessions".into(),
            "Interrupted here",
            format!("{interrupted}{ARROW}"),
        );
    }
    let notice = if state.hide_interrupted { "off" } else { ON };
    rows.push("interrupted".into(), "Interrupted notice", notice.into());
    rows.section();
    for r in settings::panel_rows() {
        rows.push(format!("set.{}", r.key), r.label, r.value);
    }
    rows.section();
    let mut set_up = 0;
    let mut pending = false;
    for f in detect::scan().found {
        let t = f.tool;
        if detect::imported(&state, t.id) {
            set_up += 1;
        } else if !state.dismissed.contains(t.id)
            && let Some(l) = setup::pending_label(t)
        {
            pending = true;
            rows.push(format!("tool.{}", t.id), t.name, format!("{l}{ARROW}"));
        }
    }
    if set_up > 0 {
        rows.push("tools".into(), "Set-up tools", format!("{set_up}{ARROW}"));
    }
    rows.section();
    let update = match update::ready_version() {
        Some(v) => format!("{v} ready{ARROW}"),
        None => format!("check{ARROW}"),
    };
    rows.push("update".into(), "Updates", update);
    let mut keys = Vec::new();
    if interrupted > 0 || pending {
        keys.push(key('d', "dismiss"));
    }
    keys.push(key('?', "help"));
    picker("Quality of life", rows, keys, at)
}

fn tools(at: Option<&str>) -> Value {
    let state = State::load();
    let mut rows = Rows {
        items: Vec::new(),
        gap: false,
    };
    for f in detect::scan().found {
        let t = f.tool;
        if !detect::imported(&state, t.id) {
            continue;
        }
        let how = match setup::import_summary(t.id) {
            Some((n, _)) => format!("import #{n}{ARROW}"),
            None => format!("plugin{ARROW}"),
        };
        rows.push(format!("tool.{}", t.id), t.name, how);
    }
    rows.section();
    rows.push("back".into(), "\u{2039} Back", String::new());
    picker(
        "Set-up tools",
        rows,
        vec![key('u', "undo import"), key('d', "dismiss")],
        at,
    )
}

pub fn pick(host: &Host, params: &Value, session: &str, pick: &str) -> Value {
    let (id, key) = match pick.rsplit_once(':') {
        Some((id, k)) if k.chars().count() == 1 => (id, k),
        _ => (pick, ""),
    };
    match (id, key) {
        (_, "?") | ("help", _) => json!({"text": crate::USAGE}),
        (_, "i") | ("interrupted", "") => {
            sessions::toggle(host, params);
            open_at(host, params, Some("interrupted"))
        }
        ("sessions", "") if sessions::shown() => {
            json!({"slash": "/sessions", "text": "/sessions to pick one up"})
        }
        ("sessions", "d") if sessions::shown() => sessions::dismiss_command(host, params),
        (id, "") if id.starts_with("set.") => match settings::step(&id[4..]) {
            Ok(()) => open_at(host, params, Some(id)),
            Err(e) => json!({"text": format!("qol: {e}")}),
        },
        ("tools", "") => tools(None),
        ("back", "") => open_at(host, params, Some("tools")),
        ("update", "") => update::run(host, session),
        (id, key) if id.starts_with("tool.") => tool(host, &id[5..], key),
        (_, "u") => json!({"text": "nothing to undo there"}),
        (_, "d") => json!({"text": "nothing to dismiss there"}),
        _ => json!({"text": "qol: nothing picked"}),
    }
}

fn tool(host: &Host, id: &str, key: &str) -> Value {
    let Some(t) = detect::tool(id) else {
        return json!({"text": format!("unknown tool {id:?}")});
    };
    let summary = setup::import_summary(t.id);
    match (key, summary) {
        ("d", _) => setup::dismiss(t.id),
        ("u", Some((n, _))) => setup::undo_import(n),
        ("u", None) => json!({"text": format!("nothing imported from {}", t.name)}),
        (_, Some((n, summary))) if detect::imported(&State::load(), t.id) => json!({
            "text": format!("{} imported (#{n}): {summary}\nu in /qol \u{203a} Set-up tools undoes it", t.name)
        }),
        (_, None) if detect::imported(&State::load(), t.id) => {
            let spec = sources::sub_plugin(t.id).map_or(t.id, |s| s.spec);
            json!({"text": format!("{} is set up through the {spec} plugin", t.name)})
        }
        _ => setup::command(host, Some(t.id)),
    }
}
