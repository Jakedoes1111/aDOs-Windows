//! MCP servers handed to agents on `session/new` and `session/load`.
//!
//! aDOs runs one MCP server of its own (crate `ados-mcp`). The shell registers it once
//! at launch with [`set_ados_server`]; the assistant and Sessions pass [`ados_servers`]
//! on every new or loaded session. Only the stdio form is used: it is the one transport
//! every ACP agent must accept (see `docs/design/backends.md`, "MCP transport").
use std::path::PathBuf;
use std::sync::Mutex;

use agent_client_protocol::v1::schema::{EnvVariable, McpServer, McpServerStdio};

/// One stdio MCP server: the agent runs `command args…` with `env` and talks MCP over its stdio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpEntry {
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl McpEntry {
    pub(crate) fn to_acp(&self) -> McpServer {
        McpServer::Stdio(
            McpServerStdio::new(self.name.clone(), self.command.clone())
                .args(self.args.clone())
                .env(self.env.iter().map(|(k, v)| EnvVariable::new(k.clone(), v.clone())).collect()),
        )
    }
}

static ADOS: Mutex<Option<McpEntry>> = Mutex::new(None);

/// Register aDOs's own MCP server for this run (the shell calls this once at launch).
pub fn set_ados_server(entry: Option<McpEntry>) {
    *ADOS.lock().unwrap_or_else(|e| e.into_inner()) = entry;
}

/// The servers every aDOs-started session gets: aDOs's own, when it is running.
pub fn ados_servers() -> Vec<McpEntry> {
    ADOS.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
}

pub(crate) fn to_acp(entries: &[McpEntry]) -> Vec<McpServer> {
    entries.iter().map(McpEntry::to_acp).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_entry_serialises_as_acp_expects() {
        let entry = McpEntry {
            name: "ados".into(),
            command: "/bin/ados".into(),
            args: vec!["mcp-bridge".into()],
            env: vec![("ADOS_MCP_SECRET".into(), "s".into())],
        };
        let json = serde_json::to_value(entry.to_acp()).expect("serialise");
        assert_eq!(
            json,
            serde_json::json!({
                "name": "ados", "command": "/bin/ados", "args": ["mcp-bridge"],
                "env": [{"name": "ADOS_MCP_SECRET", "value": "s"}]
            })
        );
    }
}
