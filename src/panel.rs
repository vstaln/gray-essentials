use serde_json::{Value, json};

use crate::detect;
use crate::host::Host;
use crate::state::State;
use crate::{sessions, setup, sources, update};

const ARROW: &str = " \u{203a}";

pub fn open(host: &Host, params: &Value) -> Value {
    sessions::refresh(host, params);
    let state = State::load();
    let mut items: Vec<Value> = Vec::new();
    let mut push = |id: String, label: &str, value: String| {
        items.push(json!({"id": id, "label": label, "description": value}));
    };
    let shown = !state.hide_interrupted;
    push(
        "interrupted".into(),
        "Interrupted sessions",
        if shown { "\u{2713} shown" } else { "hidden" }.to_string(),
    );
    let interrupted = sessions::interrupted_count(host, params);
    if interrupted > 0 {
        push(
            "sessions".into(),
            "Interrupted here",
            format!("{interrupted}{ARROW}"),
        );
    }
    for f in detect::scan().found {
        let t = f.tool;
        let value = if detect::imported(&state, t.id) {
            "\u{2713} set up".to_string()
        } else if state.dismissed.contains(t.id) {
            continue;
        } else {
            match setup::pending_label(t) {
                Some(l) => format!("{l}{ARROW}"),
                None => continue,
            }
        };
        push(format!("tool.{}", t.id), t.name, value);
    }
    let update = match update::ready_version() {
        Some(v) => format!("{v} ready{ARROW}"),
        None => format!("check{ARROW}"),
    };
    push("update".into(), "Updates", update);
    push("help".into(), "Help", String::new());
    json!({"picker": {
        "title": "Quality of life",
        "items": items,
        "enter": "open",
        "keys": [{"key": "d", "label": "dismiss"}, {"key": "u", "label": "undo"},
            {"key": "i", "label": "show/hide interrupted"},
        ],
    }})
}

pub fn pick(host: &Host, params: &Value, session: &str, pick: &str) -> Value {
    let (id, key) = match pick.rsplit_once(':') {
        Some((id, k)) if k.len() == 1 => (id, k),
        _ => (pick, ""),
    };
    match (id, key) {
        (_, "i") | ("interrupted", "") => {
            sessions::toggle(host, params);
            open(host, params)
        }
        ("sessions", "") if sessions::shown() => {
            json!({"slash": "/sessions", "text": "/sessions to pick one up"})
        }
        ("sessions", "d") if sessions::shown() => sessions::dismiss_command(host, params),
        ("update", "") => update::run(host, session),
        ("help", _) => json!({"text": crate::USAGE}),
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
            "text": format!("{} imported (#{n}): {summary}\nu in /qol undoes it", t.name)
        }),
        (_, None) if detect::imported(&State::load(), t.id) => {
            let spec = sources::sub_plugin(t.id).map_or(t.id, |s| s.spec);
            json!({"text": format!("{} is set up through the {spec} plugin", t.name)})
        }
        _ => setup::command(host, Some(t.id)),
    }
}
