//! Where aDOs keeps its data. `ADOS_HOME` overrides everything (tests set it).
use std::path::PathBuf;

pub fn home() -> PathBuf {
    let dir = std::env::var_os("ADOS_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
        base.join("ados")
    });
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn sub(name: &str) -> PathBuf {
    let dir = home().join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Markdown notes, one file per note.
pub fn notes_dir() -> PathBuf { sub("notes") }
/// Quick captures, one markdown file per capture (see `capture`).
pub fn inbox_dir() -> PathBuf { sub("inbox") }
/// Saved layouts, one JSON file per layout.
pub fn layouts_dir() -> PathBuf { sub("layouts") }
/// Small per-module state files: `state/<module>.json`.
pub fn state_file(module: &str) -> PathBuf { sub("state").join(format!("{module}.json")) }
