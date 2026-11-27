#[cfg(unix)]
pub mod cli;
#[cfg(unix)]
pub mod exec;
#[cfg(unix)]
pub mod mask;
#[cfg(unix)]
pub mod store;
#[cfg(unix)]
pub mod util;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(not(unix))]
mod fallback {
    use serde_json::{Value, json};

    use crate::host::Host;

    pub const TOOL_NAME: &str = "secret_request";

    pub fn tool_def() -> Value {
        json!({
            "name": TOOL_NAME,
            "description": "Ask the user for a secret (API key, password, token) via a masked prompt. The value is stored in the keyring or kept in session memory and is never shown to the model; use {{secret:NAME}} in bash commands to reference it.",
            "parameters": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Secret identifier (letters, digits, underscores; starts with a letter)."
                    },
                    "purpose": {
                        "type": "string",
                        "description": "Short explanation shown to the user for why the secret is needed."
                    }
                },
                "required": ["name", "purpose"]
            }
        })
    }

    pub fn init() {}
    pub fn cleanup() {}

    pub fn precheck(_params: &Value) -> Option<Value> {
        None
    }

    pub fn tool_before(_params: &Value) -> Value {
        json!({"decision": "allow"})
    }

    pub fn tool_after(_params: &Value) -> Value {
        json!({})
    }

    pub fn tool_call(_host: &Host, _params: &Value) -> Value {
        json!({
            "content": "gray-secrets is only supported on Unix systems.",
            "is_error": true
        })
    }

    pub fn command(_host: &Host, _argv: &[String]) -> Value {
        json!({"text": "gray-secrets is only supported on Unix systems."})
    }

    pub fn run_cli(_args: &[String]) -> i32 {
        eprintln!("gray-secrets is only supported on Unix systems.");
        1
    }
}

#[cfg(not(unix))]
pub use fallback::*;
