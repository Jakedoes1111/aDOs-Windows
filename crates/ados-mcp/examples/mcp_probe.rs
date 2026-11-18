//! An MCP client that reaches aDOs the way an agent adapter does: it runs the stdio
//! command aDOs hands agents (`ados mcp-bridge` with the socket and secret in its env),
//! speaks MCP over that process's stdio, lists the tools and calls them.
//!
//! ```text
//! # The bridge finds the running aDOs through its current.json (or ADOS_MCP_SOCKET/SECRET).
//! mcp_probe <path to ados> all                 # every tool, in a sensible order
//! mcp_probe <path to ados> call <tool> '<json>'
//! mcp_probe <path to ados> refuse              # socket without / with a wrong secret
//! ```
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(windows)]
use uds_windows::UnixStream;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Client {
    fn start(ados: &str) -> Result<Self, String> {
        let mut child = Command::new(ados)
            .arg("mcp-bridge")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run {ados} mcp-bridge: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("no stdout")?);
        let mut c = Client { child, stdin, stdout, next: 1 };
        let init = c.request("initialize", json!({
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": { "name": "mcp_probe", "version": "0.1.0" }
        }))?;
        println!("initialize → server {} {}, protocol {}", init["serverInfo"]["name"], init["serverInfo"]["version"], init["protocolVersion"]);
        writeln!(c.stdin, "{}", json!({"jsonrpc":"2.0","method":"notifications/initialized"})).map_err(|e| e.to_string())?;
        Ok(c)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next;
        self.next += 1;
        writeln!(self.stdin, "{}", json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})).map_err(|e| e.to_string())?;
        loop {
            let mut line = String::new();
            if self.stdout.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("the bridge closed".into());
            }
            let v: Value = serde_json::from_str(&line).map_err(|e| format!("bad JSON from the bridge: {e}: {line}"))?;
            if let Some(err) = v.get("error") {
                return Err(format!("JSON-RPC error {}: {}", err["code"], err["message"]));
            }
            if v["id"] == id {
                return Ok(v["result"].clone());
            }
        }
    }

    /// Call a tool; prints a short form and returns the parsed text (JSON when it is JSON).
    fn call(&mut self, tool: &str, args: Value) -> Value {
        let r = match self.request("tools/call", json!({ "name": tool, "arguments": args })) {
            Ok(r) => r,
            Err(e) => {
                println!("✗ {tool} {args}: {e}");
                return Value::Null;
            }
        };
        let text = r["content"][0]["text"].as_str().unwrap_or("").to_string();
        let mark = if r["isError"] == true { "✗ (isError)" } else { "✓" };
        let mut short: String = text.chars().take(600).collect();
        if text.chars().count() > 600 {
            short.push_str(" …");
        }
        println!("{mark} {tool} {args}\n    {}", short.replace('\n', "\n    "));
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn all(c: &mut Client) {
    let tools = c.request("tools/list", json!({})).unwrap_or_default();
    let names: Vec<&str> = tools["tools"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str()).collect();
    println!("tools/list → {} tools: {}", names.len(), names.join(", "));
    let actions = c.call("actions_list", json!({}));
    println!("    ({} actions)", actions.as_array().map_or(0, Vec::len));
    c.call("context_active", json!({}));
    c.call("context_all", json!({}));
    c.call("context_module", json!({ "id": "nope" }));
    c.call("layouts_list", json!({}));
    c.call("layouts_switch", json!({ "name": "Focus" }));
    c.call("layouts_save", json!({}));
    c.call("actions_run", json!({ "id": "shell.focus.tasks" }));
    let note = c.call("notes_create", json!({ "title": "Probe note", "body": "Written by mcp_probe." }));
    c.call("notes_list", json!({}));
    if let Some(path) = note["path"].as_str() {
        c.call("notes_read", json!({ "path": path }));
        c.call("notes_append", json!({ "path": path, "text": "A second paragraph." }));
        c.call("notes_read", json!({ "path": path }));
    }
    c.call("notes_read", json!({ "path": "/etc/passwd" }));
    c.call("capture_save", json!({ "text": "probe: remember to water the plants" }));
    let task = c.call("tasks_add", json!({ "text": "Probe task tomorrow #probe" }));
    c.call("tasks_list", json!({}));
    if let Some(id) = task["id"].as_str() {
        c.call("tasks_set_done", json!({ "id": id, "done": true }));
        c.call("tasks_set_done", json!({ "id": id, "done": false }));
    }
    c.call("tasks_add", json!({}));
    c.call("calendar_events_between", json!({ "start": "2026-11-09", "end": "2026-11-16" }));
    let sessions = c.call("sessions_list", json!({}));
    let first = sessions["backends"].as_array().into_iter().flatten()
        .find_map(|b| b["sessions"].as_array().and_then(|s| s.first()).cloned());
    if let Some(s) = first {
        let id = s["id"].as_str().unwrap_or_default().to_string();
        c.call("sessions_read", json!({ "id": id }));
        c.call("sessions_prompt", json!({ "id": id, "text": "hello from the probe" }));
    }
    for s in sessions["started_here"].as_array().into_iter().flatten() {
        let id = s["id"].as_str().unwrap_or_default();
        c.call("sessions_prompt", json!({ "id": id, "text": "hello from the probe" }));
        c.call("sessions_read", json!({ "id": id }));
    }
}

/// Talk to the socket directly: no secret, then a wrong one. Both must be refused.
fn refuse() -> Result<(), String> {
    let socket = match std::env::var_os("ADOS_MCP_SOCKET") {
        Some(s) => std::path::PathBuf::from(s),
        None => ados_mcp::server::read_current()?.0,
    };
    for (label, first) in [
        ("no secret", json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string()),
        ("wrong secret", "ADOS-MCP-SECRET 0000".to_string()),
    ] {
        let mut s = UnixStream::connect(&socket).map_err(|e| e.to_string())?;
        writeln!(s, "{first}").map_err(|e| e.to_string())?;
        writeln!(s, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).map_err(|e| e.to_string())?;
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        println!("{label}: {}", out.trim());
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(ados), Some(mode)) = (args.first(), args.get(1)) else {
        eprintln!("Usage: mcp_probe <ados binary> all | call <tool> '<json>' | refuse");
        std::process::exit(2);
    };
    if mode == "refuse" {
        if let Err(e) = refuse() {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    let mut client = match Client::start(ados) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    match mode.as_str() {
        "all" => all(&mut client),
        "call" => {
            let tool = args.get(2).cloned().unwrap_or_default();
            let json: Value = args.get(3).and_then(|a| serde_json::from_str(a).ok()).unwrap_or(json!({}));
            client.call(&tool, json);
        }
        other => eprintln!("Unknown mode {other}"),
    }
}
