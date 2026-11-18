//! `ADOS_HOME/backends.toml`: which agents aDOs can start. Data, not code, so
//! adding a backend needs no rebuild.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// One `[[backend]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BackendConfig {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    backend: Vec<BackendConfig>,
    #[serde(default)]
    settings: Settings,
}

/// The optional `[settings]` table of `backends.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Stop a backend this many seconds after its last user lets go of it.
    pub idle_stop_secs: u64,
}

/// Three minutes: long enough that switching panels does not restart agents.
pub const DEFAULT_IDLE_STOP_SECS: u64 = 180;

impl Default for Settings {
    fn default() -> Self {
        Self { idle_stop_secs: DEFAULT_IDLE_STOP_SECS }
    }
}

/// The file written when `backends.toml` is missing (the launch set in `backends.md`).
pub const DEFAULT_BACKENDS: &str = r#"# Agent backends aDOs can start. Each speaks ACP over stdio.
# Edit freely: aDOs reads this file when it starts a backend.
# Backends no aDOs view is using are stopped after idle_stop_secs and started
# again on next use.

[settings]
idle_stop_secs = 180

[[backend]]
id = "pi"
name = "pi"
command = "npx"
args = ["-y", "pi-acp@0.0.34"]
env = {}

[[backend]]
id = "codex"
name = "Codex"
command = "npx"
args = ["-y", "@agentclientprotocol/codex-acp@2.6.0"]
env = {}

[[backend]]
id = "claude"
name = "Claude Code"
command = "claude-agent-acp"
args = []
env = {}
"#;

/// Where aDOs keeps its data. Same rule as `ados_core::paths::home` (that crate
/// pulls in GPUI, which this crate must not): `ADOS_HOME`, else
/// `$XDG_DATA_HOME/ados`, else `~/.local/share/ados`.
pub fn ados_home() -> PathBuf {
    std::env::var_os("ADOS_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
        base.join("ados")
    })
}

/// `ADOS_HOME/backends.toml`.
pub fn backends_file() -> PathBuf {
    ados_home().join("backends.toml")
}

/// Read the backend list from `ADOS_HOME/backends.toml`, writing the default first if missing.
pub fn load() -> Result<Vec<BackendConfig>> {
    load_from(&backends_file())
}

/// Read the backend list from `path`, writing the default file there first if it is missing.
pub fn load_from(path: &Path) -> Result<Vec<BackendConfig>> {
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Could not create {}", dir.display()))?;
        }
        std::fs::write(path, DEFAULT_BACKENDS)
            .with_context(|| format!("Could not write {}", path.display()))?;
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Could not read {}", path.display()))?;
    parse(&text).with_context(|| format!("{} is not a valid backend list", path.display()))
}

/// Parse the contents of a `backends.toml`.
pub fn parse(text: &str) -> Result<Vec<BackendConfig>> {
    let file: File = toml::from_str(text)?;
    Ok(file.backend)
}

/// The `[settings]` of `ADOS_HOME/backends.toml`; defaults when the file or table is missing
/// or unreadable (the backend list reports file problems on its own).
pub fn settings() -> Settings {
    std::fs::read_to_string(backends_file())
        .ok()
        .and_then(|text| parse_settings(&text).ok())
        .unwrap_or_default()
}

pub fn parse_settings(text: &str) -> Result<Settings> {
    let file: File = toml::from_str(text)?;
    Ok(file.settings)
}

/// Find a backend by id.
pub fn find<'a>(backends: &'a [BackendConfig], id: &str) -> Option<&'a BackendConfig> {
    backends.iter().find(|b| b.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_parses_to_three_backends() {
        let list = parse(DEFAULT_BACKENDS).expect("default parses");
        let ids: Vec<_> = list.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["pi", "codex", "claude"]);
        assert_eq!(list[2].command, "claude-agent-acp");
    }

    #[test]
    fn settings_default_and_override() {
        assert_eq!(parse_settings(DEFAULT_BACKENDS).unwrap().idle_stop_secs, 180);
        assert_eq!(parse_settings("").unwrap(), Settings::default());
        let s = parse_settings("[settings]\nidle_stop_secs = 5\n").unwrap();
        assert_eq!(s.idle_stop_secs, 5);
    }

    #[test]
    fn env_and_args_are_optional() {
        let list = parse("[[backend]]\nid = \"x\"\nname = \"X\"\ncommand = \"x-acp\"\n").unwrap();
        assert!(list[0].args.is_empty() && list[0].env.is_empty());
    }
}
