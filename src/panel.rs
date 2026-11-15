use serde_json::{Value, json};

use crate::detect::{self, ToolDef};
use crate::host::{self, Host};
use crate::state::State;
use crate::{sessions, setup, sources, update};

enum Pick {
    Sessions,
    Tool(&'static ToolDef),
    Update,
    Help,
}

fn choose(
    host: &Host,
    id: &str,
    question: &str,
    options: Vec<Value>,
) -> Result<Option<String>, String> {
    let result = host.ask(json!([{
        "id": id,
        "header": "qol",
        "question": question,
        "options": options,
        "multi": false,
    }]))?;
    Ok(host::answer_labels(&result, id).and_then(|l| l.into_iter().next()))
}

pub fn open(host: &Host, params: &Value, session: &str) -> Value {
    let state = State::load();
    let mut rows: Vec<(String, &str, Pick)> = Vec::new();
    let interrupted = sessions::interrupted_count(host, params);
    if interrupted > 0 {
        rows.push((
            format!("Interrupted sessions here ({interrupted})"),
            "pick one up or dismiss them",
            Pick::Sessions,
        ));
    }
    for f in detect::scan().found {
        let t = f.tool;
        let (status, desc) = if detect::imported(&state, t.id) {
            ("\u{2713} set up".to_string(), "see what was imported")
        } else if state.dismissed.contains(t.id) {
            ("dismissed".to_string(), "run setup")
        } else {
            match setup::pending_count(t) {
                0 => ("nothing to import".to_string(), "run setup"),
                1 => ("1 item to import".to_string(), "run setup"),
                n => (format!("{n} items to import"), "run setup"),
            }
        };
        rows.push((format!("{}  {status}", t.name), desc, Pick::Tool(t)));
    }
    rows.push(("Check for updates".to_string(), "/qol update", Pick::Update));
    rows.push(("Help".to_string(), "all /qol commands", Pick::Help));
    let options = rows
        .iter()
        .map(|(label, desc, _)| json!({"label": label, "description": desc}))
        .collect();
    let picked = match choose(host, "qol", "Quality of life", options) {
        Ok(Some(p)) => p,
        Ok(None) => return json!({"text": "qol: nothing picked"}),
        Err(_) => return json!({"text": crate::USAGE}),
    };
    let Some((_, _, pick)) = rows.into_iter().find(|(label, _, _)| *label == picked) else {
        return json!({"text": "qol: nothing picked"});
    };
    match pick {
        Pick::Sessions => {
            json!({"text": "/sessions to pick up \u{b7} /qol sessions dismiss to clear"})
        }
        Pick::Tool(t) => tool(host, &state, t),
        Pick::Update => update::run(host, session),
        Pick::Help => json!({"text": crate::USAGE}),
    }
}

fn tool(host: &Host, state: &State, t: &'static ToolDef) -> Value {
    if !detect::imported(state, t.id) {
        return setup::command(host, Some(t.id));
    }
    let Some((id, summary)) = setup::import_summary(t.id) else {
        let spec = sources::sub_plugin(t.id).map_or(t.id, |s| s.spec);
        return json!({"text": format!("{} is set up through the {spec} plugin", t.name)});
    };
    let undo = format!("Undo import #{id}");
    let options = vec![
        json!({"label": undo, "description": "revert everything this import added"}),
        json!({"label": "Keep", "description": "leave it as it is"}),
    ];
    let question = format!("{} imported: {summary}", t.name);
    match choose(host, "qol_tool", &question, options) {
        Ok(Some(p)) if p == undo => setup::undo_import(id),
        Ok(_) => json!({"text": format!("{} kept", t.name)}),
        Err(e) => json!({"text": format!("qol: {e}")}),
    }
}
