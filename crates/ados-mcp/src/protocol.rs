//! MCP over JSON-RPC 2.0, one message per line: request routing, no I/O.
//!
//! Handles `initialize`, `ping`, `tools/list`, `tools/call`; notifications get no reply.
use serde_json::{json, Value};

use crate::tools;

/// Protocol revisions this server speaks; the newest is offered when the client asks for another.
const VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

pub const INSTRUCTIONS: &str = "Tools for aDOs, the owner's desktop workspace: its commands, what each module shows, layouts, agent sessions, notes, captures, tasks and calendar. Prefer these over shell commands or file edits for anything in aDOs. Every change shows in aDOs with Undo where possible.";

/// What a tool call produced: a JSON value for the agent, or a plain error sentence.
pub type ToolResult = Result<Value, String>;

/// Answer one incoming line. `call` runs a validated tool. `None` means no reply (a notification).
pub fn handle_line(line: &str, call: &mut dyn FnMut(&'static tools::Tool, Value) -> ToolResult) -> Option<Value> {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(error(Value::Null, -32700, &format!("Parse error: {e}"))),
    };
    handle(&msg, call)
}

pub fn handle(msg: &Value, call: &mut dyn FnMut(&'static tools::Tool, Value) -> ToolResult) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str);
    let (Some(id), Some(method)) = (id, method) else {
        // Notifications (no id) and responses (no method) need no answer.
        return match (msg.get("id"), method) {
            (None, Some(_)) => None,
            (Some(_), None) => None,
            _ => Some(error(Value::Null, -32600, "Invalid request")),
        };
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = VERSIONS.iter().find(|v| **v == asked).unwrap_or(&VERSIONS[0]);
            ok(id, json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "ados", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, tools::list()),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let Some(tool) = tools::find(name) else {
                return Some(error(id, -32602, &format!("Unknown tool: {name}")));
            };
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            let result = tools::validate(tool, &args).and_then(|()| call(tool, args));
            ok(id, tool_result(result))
        }
        other => error(id, -32601, &format!("Method not found: {other}")),
    })
}

/// A `tools/call` result: the value as JSON text, or the error with `isError`.
pub fn tool_result(result: ToolResult) -> Value {
    match result {
        Ok(Value::String(text)) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_default();
            json!({ "content": [{ "type": "text", "text": text }], "isError": false })
        }
        Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
    }
}

pub fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(line: &str) -> Option<Value> {
        handle_line(line, &mut |tool, args| Ok(json!({ "tool": tool.name, "args": args })))
    }

    #[test]
    fn initialize_negotiates_the_version() {
        let r = run(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#).expect("reply");
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "ados");
        let r = run(r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#).expect("reply");
        assert_eq!(r["result"]["protocolVersion"], VERSIONS[0]);
    }

    #[test]
    fn notifications_get_no_reply() {
        assert!(run(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    }

    #[test]
    fn routes_calls_and_reports_errors() {
        let r = run(r#"{"jsonrpc":"2.0","id":"a","method":"tools/call","params":{"name":"tasks_add","arguments":{"text":"Buy milk"}}}"#).expect("reply");
        assert_eq!(r["id"], "a");
        assert_eq!(r["result"]["isError"], false);
        let text = r["result"]["content"][0]["text"].as_str().expect("text");
        assert!(text.contains("\"tool\": \"tasks_add\""));

        let r = run(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tasks_add","arguments":{}}}"#).expect("reply");
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["content"][0]["text"], "tasks_add needs \"text\".");

        let r = run(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope"}}"#).expect("reply");
        assert_eq!(r["error"]["code"], -32602);
        let r = run(r#"{"jsonrpc":"2.0","id":5,"method":"resources/list"}"#).expect("reply");
        assert_eq!(r["error"]["code"], -32601);
        let r = run("not json").expect("reply");
        assert_eq!(r["error"]["code"], -32700);
        let r = run(r#"{"jsonrpc":"2.0","id":6,"method":"tools/list"}"#).expect("reply");
        assert!(r["result"]["tools"].as_array().is_some_and(|t| t.len() == tools::available().count()));
    }
}
