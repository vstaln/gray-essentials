mod core;

use std::time::Duration;

use serde_json::{Value, json};

use crate::host::Host;
use core::{
    HOST_ASK, NO_ANSWER_YET, UserQuestion, answers_to_json, normalize_questions, parse_ask_result,
    validate_tool_args,
};

pub const TOOL_NAME: &str = core::TOOL_NAME;

const ASK_TTL: Duration = Duration::from_secs(23 * 3600 + 1800);
const NONBLOCK_TTL: Duration = Duration::from_secs(120);

pub fn tool_def() -> Value {
    core::manifest()["tools"][0].clone()
}

pub fn prompt_guidance() -> &'static str {
    core::prompt_guidance()
}

pub fn tool_call(host: &Host, params: &Value) -> Value {
    let args = params.get("args").cloned().unwrap_or(Value::Null);
    let questions: Vec<UserQuestion> = match args
        .get("questions")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
    {
        Ok(q) => q.unwrap_or_default(),
        Err(e) => {
            return json!({
                "content": format!("request_user_input: bad args: {e}"),
                "is_error": true,
            });
        }
    };
    if let Err(e) = validate_tool_args(&questions) {
        return json!({ "content": e, "is_error": true });
    }
    let normalized = match normalize_questions(questions) {
        Ok(q) => q,
        Err(e) => return json!({ "content": e, "is_error": true }),
    };
    let blocking = args
        .get("blocking")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let ttl = if blocking { ASK_TTL } else { NONBLOCK_TTL };
    match host.call(
        HOST_ASK,
        json!({ "questions": normalized, "blocking": blocking }),
        ttl,
    ) {
        Ok(result) => {
            let answers = parse_ask_result(&result);
            if !blocking && answers.iter().all(|a| a.answers.is_empty()) {
                return json!({ "content": NO_ANSWER_YET });
            }
            json!({ "content": answers_to_json(&answers).to_string() })
        }
        Err(err) if !blocking && err.contains("host did not answer") => {
            json!({ "content": NO_ANSWER_YET })
        }
        Err(err) if err.contains("host did not answer") => json!({
            "content": "request_user_input: host did not answer in time",
            "is_error": true,
        }),
        Err(err) => json!({
            "content": format!("request_user_input failed: {err}"),
            "is_error": true,
        }),
    }
}
