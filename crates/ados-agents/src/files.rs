//! Client side of `fs/read_text_file` and `fs/write_text_file`: plain file I/O,
//! limited to the working directory of the session that asks.
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Session id -> working directory, filled by `session/new` and `session/load`.
#[derive(Clone, Default)]
pub(crate) struct SessionDirs(Arc<Mutex<HashMap<String, PathBuf>>>);

impl SessionDirs {
    pub fn insert(&self, session_id: &str, cwd: &Path) {
        if let Ok(mut map) = self.0.lock() {
            map.insert(session_id.to_string(), cwd.to_path_buf());
        }
    }

    fn cwd(&self, session_id: &str) -> Result<PathBuf, String> {
        let map = self.0.lock().map_err(|_| "Session table is unavailable.".to_string())?;
        map.get(session_id)
            .cloned()
            .ok_or_else(|| format!("No session {session_id} was started from aDOs."))
    }

    /// Resolve `path` for `session_id`, refusing anything outside the session's cwd.
    pub fn resolve(&self, session_id: &str, path: &Path) -> Result<PathBuf, String> {
        let cwd = self.cwd(session_id)?;
        let joined = if path.is_absolute() { path.to_path_buf() } else { cwd.join(path) };
        let clean = normalize(&joined);
        if clean.starts_with(normalize(&cwd)) {
            Ok(clean)
        } else {
            Err(format!("{} is outside the session folder {}.", path.display(), cwd.display()))
        }
    }
}

/// Remove `.` and `..` without touching the disk (the file may not exist yet).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Read a text file, optionally from 1-based `line` for `limit` lines.
pub(crate) fn read(path: &Path, line: Option<u32>, limit: Option<u32>) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Could not read {}: {e}.", path.display()))?;
    if line.is_none() && limit.is_none() {
        return Ok(text);
    }
    let skip = line.unwrap_or(1).saturating_sub(1) as usize;
    let take = limit.map(|l| l as usize).unwrap_or(usize::MAX);
    Ok(text.split_inclusive('\n').skip(skip).take(take).collect())
}

/// Write a text file, creating its parent folders.
pub(crate) fn write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Could not create {}: {e}.", dir.display()))?;
    }
    std::fs::write(path, content).map_err(|e| format!("Could not write {}: {e}.", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_paths_outside_cwd() {
        let dirs = SessionDirs::default();
        dirs.insert("s", Path::new("/work/proj"));
        assert_eq!(dirs.resolve("s", Path::new("/work/proj/a.txt")).unwrap(), Path::new("/work/proj/a.txt"));
        assert_eq!(dirs.resolve("s", Path::new("b/c.txt")).unwrap(), Path::new("/work/proj/b/c.txt"));
        assert!(dirs.resolve("s", Path::new("/work/proj/../other/x")).is_err());
        assert!(dirs.resolve("s", Path::new("/etc/passwd")).is_err());
        assert!(dirs.resolve("unknown", Path::new("/work/proj/a")).is_err());
    }
}
