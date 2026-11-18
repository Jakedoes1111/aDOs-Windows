//! Files under `ADOS_HOME/calendar`: native events, subscription list, cached copies, view state.
use crate::model::Event;
use ados_core::paths;
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub fn dir() -> PathBuf {
    let dir = paths::home().join("calendar");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
pub fn events_file() -> PathBuf { dir().join("events.json") }
pub fn subscriptions_file() -> PathBuf { dir().join("subscriptions.toml") }
pub fn cache_dir() -> PathBuf {
    let dir = dir().join("cache");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Write to a sibling temp file, then rename over the target.
pub fn atomic_write(path: &Path, text: &str) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct EventsFile {
    #[serde(default)]
    events: Vec<Event>,
}

pub fn parse_events(text: &str) -> anyhow::Result<Vec<Event>> {
    Ok(serde_json::from_str::<EventsFile>(text)?.events)
}

pub fn render_events(events: &[Event]) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&EventsFile { events: events.to_vec() })?)
}

pub fn load_events() -> anyhow::Result<Vec<Event>> {
    match std::fs::read_to_string(events_file()) {
        Ok(text) => parse_events(&text).context("events.json is not valid"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e).context("reading events.json"),
    }
}

pub fn save_events(events: &[Event]) -> anyhow::Result<()> {
    atomic_write(&events_file(), &render_events(events)?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewKind {
    Day,
    #[default]
    Week,
    Agenda,
}

/// What the panel shows, restored on reopen.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    pub view: ViewKind,
    /// The day the view is anchored on, `YYYY-MM-DD`. Missing means today.
    pub anchor: Option<String>,
    pub selected: Option<String>,
    /// The skipped-events note the user hid (its text); it shows again when that changes.
    pub skipped_dismissed: Option<String>,
}

pub fn load_view() -> anyhow::Result<ViewState> {
    match std::fs::read_to_string(paths::state_file("calendar")) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ViewState::default()),
        Err(e) => Err(e.into()),
    }
}

pub fn save_view(state: &ViewState) -> anyhow::Result<()> {
    atomic_write(&paths::state_file("calendar"), &serde_json::to_string_pretty(state)?)
}
