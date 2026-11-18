//! The tool list: names, descriptions, input schemas, and argument checking.
//!
//! Names follow `docs/design/backends.md` with `.` written as `_` (`actions.list` is
//! `actions_list`): Anthropic and OpenAI tool names allow only letters, digits, `_` and `-`.
use serde_json::{json, Map, Value};

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// `(name, type, description, required)`; type is "string", "integer" or "boolean".
    pub params: &'static [(&'static str, &'static str, &'static str, bool)],
    /// Changes something the owner can see; shows a toast in aDOs.
    pub mutates: bool,
    /// Seconds before the agent gets a plain timeout error.
    pub timeout: u64,
}

const S: &str = "string";

pub const TOOLS: &[Tool] = &[
    Tool { name: "actions_list", description: "List every aDOs command (the palette's actions): id, title, keywords.", params: &[], mutates: false, timeout: 10 },
    Tool { name: "actions_run", description: "Run one aDOs command by id (from actions_list). Returns the focused module and layout before and after.", params: &[("id", S, "Action id, e.g. shell.focus.tasks", true)], mutates: true, timeout: 10 },
    Tool { name: "context_active", description: "What the module the owner is in shows right now: title, location, visible text, selection.", params: &[], mutates: false, timeout: 10 },
    Tool { name: "context_module", description: "What one module shows right now (ids: notes, terminal, browser, sessions, tasks, calendar, home).", params: &[("id", S, "Module id", true)], mutates: false, timeout: 10 },
    Tool { name: "context_all", description: "What every open module shows right now, and which one is active.", params: &[], mutates: false, timeout: 10 },
    Tool { name: "layouts_list", description: "List saved workspace layouts and the current one.", params: &[], mutates: false, timeout: 10 },
    Tool { name: "layouts_switch", description: "Switch the workspace to a saved layout by name.", params: &[("name", S, "Layout name from layouts_list", true)], mutates: true, timeout: 10 },
    Tool { name: "layouts_save", description: "Save the current arrangement into the current layout.", params: &[], mutates: true, timeout: 10 },
    Tool { name: "sessions_list", description: "List agent sessions across backends (pi, Codex, Claude Code…), newest first per backend. started_here marks sessions aDOs started in this run (the only ones sessions_prompt accepts).", params: &[("backend", S, "Only this backend id", false)], mutates: false, timeout: 90 },
    Tool { name: "sessions_read", description: "Read a session's transcript (user, agent and tool lines). Run sessions_list first for sessions aDOs did not start.", params: &[("id", S, "Session id", true), ("backend", S, "Backend id, if known", false)], mutates: false, timeout: 90 },
    Tool { name: "sessions_prompt", description: "Send a prompt into a session aDOs started in this run. Refused for any other session (the owner's existing chats are never written into). Returns once sent; read the reply with sessions_read.", params: &[("id", S, "Session id", true), ("text", S, "Prompt text", true)], mutates: true, timeout: 10 },
    Tool { name: "notes_list", description: "List notes, newest first: title and path.", params: &[], mutates: false, timeout: 10 },
    Tool { name: "notes_read", description: "Read one note's markdown by path (from notes_list).", params: &[("path", S, "Note path", true)], mutates: false, timeout: 10 },
    Tool { name: "notes_create", description: "Create a note. The title becomes its first line (# title).", params: &[("title", S, "Note title", true), ("body", S, "Markdown body", true)], mutates: true, timeout: 10 },
    Tool { name: "notes_append", description: "Append markdown to the end of a note.", params: &[("path", S, "Note path", true), ("text", S, "Markdown to append", true)], mutates: true, timeout: 10 },
    Tool { name: "capture_save", description: "Save a quick capture to the inbox (a thought, URL or file path; the kind is detected).", params: &[("text", S, "What to capture", true)], mutates: true, timeout: 10 },
    Tool { name: "tasks_list", description: "List tasks: id, title, status, due, project, my_day.", params: &[], mutates: false, timeout: 10 },
    Tool { name: "tasks_add", description: "Add a task with quick-add syntax: '#project' anywhere sets the project; a last word of today, tomorrow, a weekday (fri = next Friday) or YYYY-MM-DD sets the due date. Example: 'Call the bank fri #home'.", params: &[("text", S, "Quick-add text", true)], mutates: true, timeout: 10 },
    Tool { name: "tasks_set_done", description: "Mark a task done (done=true) or open again (done=false).", params: &[("id", S, "Task id from tasks_list", true), ("done", "boolean", "Done or open", true)], mutates: true, timeout: 10 },
    Tool { name: "calendar_events_between", description: "Calendar events overlapping [start, end): native and subscribed. Times are RFC 3339 (2026-11-14T09:00:00Z) or a date (2026-11-14, local midnight).", params: &[("start", S, "Start time or date", true), ("end", S, "End time or date (exclusive)", true)], mutates: false, timeout: 10 },
    Tool { name: "browser_tabs", description: "List the Browser's page tabs: index, title, URL, which is active. select=index shows that tab first; the other browser_* tools act on the active tab.", params: &[("select", "integer", "Index of the tab to show", false)], mutates: false, timeout: 10 },
    Tool { name: "browser_open", description: "Open an http(s) URL in the Browser and bring the Browser into view if the current layout has it (otherwise the answer says so). file: and other schemes are refused.", params: &[("url", S, "http or https URL", true), ("new_tab", "boolean", "Open in a new page tab instead of the active one", false)], mutates: true, timeout: 20 },
    Tool { name: "browser_read", description: "Read the page as text you can act on: headings, links (text and href), form fields and buttons, each with a ref like e12 for browser_click and browser_type. Long pages are cut; pass the returned `more` as `cursor` for the next part. Refs go stale when the page changes: read again.", params: &[("selector", S, "CSS selector: read only inside the first match", false), ("cursor", "integer", "Item offset from a previous `more`", false)], mutates: false, timeout: 20 },
    Tool { name: "browser_click", description: "Click an element by ref (from browser_read) or CSS selector.", params: &[("ref", S, "Ref from browser_read, e.g. e12", false), ("selector", S, "CSS selector, used when no ref is given", false)], mutates: true, timeout: 20 },
    Tool { name: "browser_type", description: "Replace the text of a form field, by ref (from browser_read) or CSS selector. submit=true then presses Enter (submits the form).", params: &[("text", S, "Text to put in the field", true), ("ref", S, "Ref from browser_read, e.g. e3", false), ("selector", S, "CSS selector, used when no ref is given", false), ("submit", "boolean", "Press Enter afterwards", false)], mutates: true, timeout: 20 },
    Tool { name: "browser_selection", description: "The text the owner has selected in the Browser's page, with the nearest heading above it and the page URL.", params: &[], mutates: false, timeout: 20 },
    Tool { name: "browser_screenshot", description: "Take a PNG of the Browser's page with the engine's own capture, save it under ADOS_HOME/cache and return its path.", params: &[], mutates: false, timeout: 30 },
];

pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The JSON Schema for a tool's arguments.
pub fn schema(tool: &Tool) -> Value {
    let mut props = Map::new();
    let mut required = Vec::new();
    for (name, ty, desc, req) in tool.params {
        props.insert((*name).into(), json!({ "type": ty, "description": desc }));
        if *req {
            required.push(json!(name));
        }
    }
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

/// The `tools/list` result.
pub fn list() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": schema(t),
                "annotations": { "readOnlyHint": !t.mutates },
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// Check arguments against the tool's schema. Errors are plain sentences for the agent.
pub fn validate(tool: &Tool, args: &Value) -> Result<(), String> {
    let empty = Map::new();
    let obj = match args {
        Value::Object(o) => o,
        Value::Null => &empty,
        _ => return Err(format!("{} takes an object of arguments.", tool.name)),
    };
    for key in obj.keys() {
        if !tool.params.iter().any(|(n, ..)| n == key) {
            return Err(format!("{} has no argument \"{key}\".", tool.name));
        }
    }
    for (name, ty, _, required) in tool.params {
        match obj.get(*name) {
            None | Some(Value::Null) if *required => {
                return Err(format!("{} needs \"{name}\".", tool.name));
            }
            None | Some(Value::Null) => {}
            Some(v) => {
                let ok = match *ty {
                    "string" => v.is_string(),
                    "boolean" => v.is_boolean(),
                    "integer" => v.is_i64() || v.is_u64(),
                    _ => false,
                };
                if !ok {
                    return Err(format!("\"{name}\" must be a {ty}."));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_valid_for_every_model_api() {
        for t in TOOLS {
            assert!(t.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{}", t.name);
            assert!(t.name.len() <= 64);
        }
        assert_eq!(list()["tools"].as_array().map(Vec::len), Some(TOOLS.len()));
    }

    #[test]
    fn validation_checks_required_types_and_unknowns() {
        let done = find("tasks_set_done").expect("tool");
        assert!(validate(done, &json!({"id": "t1", "done": true})).is_ok());
        assert_eq!(validate(done, &json!({"id": "t1"})), Err("tasks_set_done needs \"done\".".into()));
        assert_eq!(validate(done, &json!({"id": "t1", "done": "yes"})), Err("\"done\" must be a boolean.".into()));
        assert_eq!(validate(done, &json!({"id": "t1", "done": true, "x": 1})), Err("tasks_set_done has no argument \"x\".".into()));
        assert!(validate(done, &json!([1])).is_err());
        let list = find("sessions_list").expect("tool");
        assert!(validate(list, &Value::Null).is_ok());
        assert!(validate(list, &json!({"backend": "pi"})).is_ok());
    }

    #[test]
    fn schema_lists_required_fields() {
        let s = schema(find("notes_create").expect("tool"));
        assert_eq!(s["required"], json!(["title", "body"]));
        assert_eq!(s["properties"]["body"]["type"], "string");
    }
}
