//! What the Sessions panel remembers across relaunch: `state/sessions.json`.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// Key (`backend/id`) of the open session.
    pub selected: Option<String>,
    /// Its folder, so it can be reopened before the list has loaded.
    pub selected_cwd: Option<PathBuf>,
    /// Its title, shown while it reopens.
    pub selected_title: Option<String>,
    /// The sessions list (left side) is collapsed so the open chat has the full width.
    pub collapsed: bool,
    /// Filter box text.
    pub filter: String,
    /// Only this backend's sessions, when set.
    pub backend: Option<String>,
    /// List scroll position (pixels from the top).
    pub scroll: f32,
    /// Unsent composer text per session key.
    pub drafts: BTreeMap<String, String>,
}

fn path() -> PathBuf {
    ados_core::paths::state_file("sessions")
}

impl Saved {
    /// Load, or start empty with a sentence saying why.
    pub fn load() -> (Self, Option<String>) {
        let path = path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(saved) => (saved, None),
                Err(e) => (Self::default(), Some(format!("Couldn't read saved Sessions state ({e}); starting fresh."))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (Self::default(), Some(format!("Couldn't read saved Sessions state: {e}"))),
        }
    }

    /// Write atomically (temp file, then rename).
    pub fn save(&self) -> anyhow::Result<()> {
        let path = path();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Saved;

    #[test]
    fn old_or_partial_files_still_load() {
        let saved: Saved = serde_json::from_str(r#"{"filter":"week"}"#).unwrap();
        assert_eq!(saved.filter, "week");
        assert!(saved.selected.is_none() && saved.drafts.is_empty());
    }

    #[test]
    fn collapsed_round_trips_and_defaults_to_open() {
        assert!(!serde_json::from_str::<Saved>("{}").unwrap().collapsed);
        let saved = Saved { collapsed: true, ..Saved::default() };
        let back: Saved = serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert!(back.collapsed);
    }
}
