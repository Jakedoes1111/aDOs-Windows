//! Selection, mode and a crash-recovery draft, committed atomically.
use ados_core::paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Typed text not yet in its file. `path` is None when no note could be made for it
/// (an orphan): it is kept here and turned into a note on the next launch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub path: Option<PathBuf>,
    pub text: String,
    /// Hash of the note file as the editor last loaded or saved it (`disk::hash_text`).
    /// On recovery a file that no longer matches it changed outside after the crash.
    /// Missing in state files from before round 12.
    #[serde(default)]
    pub stamp: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotesState {
    pub selected: Option<PathBuf>,
    pub selected_capture: Option<PathBuf>,
    pub mode: String,
    pub draft: Option<Draft>,
}

impl Default for NotesState {
    fn default() -> Self {
        Self {
            selected: None,
            selected_capture: None,
            mode: "edit".into(),
            draft: None,
        }
    }
}

pub fn load() -> anyhow::Result<NotesState> {
    match std::fs::read_to_string(paths::state_file("notes")) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(NotesState::default()),
        Err(e) => Err(e.into()),
    }
}

pub fn save(state: &NotesState) -> anyhow::Result<()> {
    crate::file::atomic_write(
        &paths::state_file("notes"),
        &serde_json::to_string_pretty(state)?,
    )
}

/// Resolve a crash between the file rename and the state update by its stable suffix.
pub fn draft_path(draft_path: &std::path::Path, notes: &[crate::file::NoteMeta]) -> PathBuf {
    if draft_path.exists() {
        return draft_path.to_path_buf();
    }
    let suffix = draft_path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.rsplit_once('-'))
        .map(|(_, id)| format!("-{id}.md"));
    suffix
        .and_then(|suffix| {
            notes
                .iter()
                .find(|n| n.path.to_string_lossy().ends_with(&suffix))
                .map(|n| n.path.clone())
        })
        .unwrap_or_else(|| draft_path.to_path_buf())
}
