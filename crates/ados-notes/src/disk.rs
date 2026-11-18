//! What the editor last loaded or saved for the open note, and what to do when the file
//! on disk no longer matches it (another program, a second window, `git pull`).
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

/// A note file as the editor last knew it: content hash plus modification time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub hash: u64,
    pub modified: Option<SystemTime>,
}

/// FNV-1a: stable across builds, because the hash is stored with a draft in `notes.json`.
pub fn hash_text(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The stamp of `text`, just read from or written to `path`.
pub fn stamp(path: &Path, text: &str) -> Stamp {
    Stamp { hash: hash_text(text), modified: modified(path) }
}

/// The file as it is now. `None` when it is gone or unreadable.
pub fn read(path: &Path) -> Option<(String, Stamp)> {
    let text = std::fs::read_to_string(path).ok()?;
    let stamp = stamp(path, &text);
    Some((text, stamp))
}

/// What to do with the open note, given what the editor knew and what is on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The file is as the editor left it (or holds the buffer already): save if dirty.
    Write,
    /// The file changed outside and nothing is unsaved: show the file.
    Reload(String),
    /// The file changed outside and there are unsaved edits: keep the file, save the
    /// buffer as a conflict copy next to it.
    Conflict,
}

/// `known`: what the editor last loaded or saved (None: never read, e.g. a recovered
/// draft, which is newer than the file). `disk`: the file now (None: gone, a save
/// makes it again). `buffer`: the editor text. `dirty`: unsaved edits.
pub fn decide(known: Option<&Stamp>, disk: Option<(String, Stamp)>, buffer: &str, dirty: bool) -> Decision {
    let (Some(known), Some((text, now))) = (known, disk) else { return Decision::Write };
    if now.hash == known.hash || text == buffer {
        return Decision::Write;
    }
    if dirty { Decision::Conflict } else { Decision::Reload(text) }
}

/// What the editor knew about the file of a draft recovered after a crash or kill.
/// `stamp`: the hash stored with the draft. Without one (state files from before round
/// 12), a file that holds anything other than the draft counts as changed outside;
/// an empty file (a note made for the typing) has nothing to lose.
pub fn recovered_known(stamp: Option<u64>, disk_text: Option<&str>, draft: &str) -> Option<Stamp> {
    match (stamp, disk_text) {
        (Some(hash), _) => Some(Stamp { hash, modified: None }),
        (None, Some(text)) if !text.is_empty() && text != draft => {
            // A hash the file cannot match: `decide` sees an outside change.
            Some(Stamp { hash: hash_text(draft), modified: None })
        }
        _ => None,
    }
}

/// `<title> (conflict YYYY-MM-DD HH-MM).md` next to `note`, never an existing file.
pub fn conflict_path(note: &Path, title: &str, when: &str) -> PathBuf {
    let dir = note.parent().unwrap_or(Path::new("."));
    // Keep the title a plain file name.
    let title: String = title
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | '\0') { '-' } else { c })
        .take(60)
        .collect();
    let title = title.trim();
    let title = if title.is_empty() { "Untitled" } else { title };
    let first = dir.join(format!("{title} (conflict {when}).md"));
    if !first.exists() {
        return first;
    }
    (2..1000)
        .map(|n| dir.join(format!("{title} (conflict {when} {n}).md")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Conflict copies keep their name; title changes do not rename them.
pub fn is_conflict_copy(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.ends_with(')') && s.contains(" (conflict "))
}

pub fn now_label() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%d %H-%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(text: &str) -> Stamp {
        Stamp { hash: hash_text(text), modified: None }
    }

    #[test]
    fn unchanged_file_saves() {
        let disk = Some(("a".to_string(), st("a")));
        assert_eq!(decide(Some(&st("a")), disk, "a b", true), Decision::Write);
    }

    #[test]
    fn outside_change_reloads_when_clean() {
        let disk = Some(("a\nLine added in terminal\n".to_string(), st("a\nLine added in terminal\n")));
        assert_eq!(
            decide(Some(&st("a")), disk, "a", false),
            Decision::Reload("a\nLine added in terminal\n".into())
        );
    }

    #[test]
    fn outside_change_with_unsaved_edits_is_a_conflict() {
        let disk = Some(("a outside".to_string(), st("a outside")));
        assert_eq!(decide(Some(&st("a")), disk, "a typed", true), Decision::Conflict);
    }

    #[test]
    fn same_text_both_sides_is_no_conflict() {
        let disk = Some(("a b".to_string(), st("a b")));
        assert_eq!(decide(Some(&st("a")), disk, "a b", true), Decision::Write);
    }

    #[test]
    fn gone_or_unknown_writes() {
        assert_eq!(decide(Some(&st("a")), None, "a b", true), Decision::Write);
        let disk = Some(("old".to_string(), st("old")));
        assert_eq!(decide(None, disk, "draft", true), Decision::Write);
    }

    #[test]
    fn recovered_draft_with_outside_change_is_a_conflict() {
        // Typed "a typed", killed before the save; then a phone sync added a line.
        let stamp = Some(hash_text("a"));
        let outside = "a\nSynced from phone\n";
        let known = recovered_known(stamp, Some(outside), "a typed");
        let disk = Some((outside.to_string(), st(outside)));
        assert_eq!(decide(known.as_ref(), disk, "a typed", true), Decision::Conflict);
        // Nothing changed outside: the draft is saved over its own file.
        let known = recovered_known(stamp, Some("a"), "a typed");
        assert_eq!(decide(known.as_ref(), Some(("a".into(), st("a"))), "a typed", true), Decision::Write);
    }

    #[test]
    fn recovered_draft_without_stamp() {
        // Old state file: the file differs from the draft, so it is kept.
        let outside = "a\nLine from git pull\n";
        let known = recovered_known(None, Some(outside), "a typed");
        let disk = Some((outside.to_string(), st(outside)));
        assert_eq!(decide(known.as_ref(), disk, "a typed", true), Decision::Conflict);
        // The file already holds the draft, is empty (made for the typing) or is gone: save.
        let known = recovered_known(None, Some("a typed"), "a typed");
        assert_eq!(decide(known.as_ref(), Some(("a typed".into(), st("a typed"))), "a typed", true), Decision::Write);
        assert_eq!(recovered_known(None, Some(""), "a typed"), None);
        assert_eq!(recovered_known(None, None, "a typed"), None);
    }

    #[test]
    fn stamp_survives_state_json() {
        let draft: crate::state::Draft =
            serde_json::from_str(r#"{"path":"/n/a-1.md","text":"x","stamp":18446744073709551615}"#).expect("load");
        assert_eq!(draft.stamp, Some(u64::MAX));
        let old: crate::state::Draft = serde_json::from_str(r#"{"path":"/n/a-1.md","text":"x"}"#).expect("old");
        assert_eq!(old.stamp, None);
    }

    #[test]
    fn conflict_names() {
        let dir = std::env::temp_dir().join(format!("ados-notes-conflict-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let note = dir.join("ask-sam-1234.md");
        let first = conflict_path(&note, "Ask Sam / venue", "2026-11-15 14-03");
        assert_eq!(first.file_name().unwrap(), "Ask Sam - venue (conflict 2026-11-15 14-03).md");
        assert!(is_conflict_copy(&first));
        assert!(!is_conflict_copy(&note));
        std::fs::write(&first, "x").unwrap();
        let second = conflict_path(&note, "Ask Sam / venue", "2026-11-15 14-03");
        assert_eq!(second.file_name().unwrap(), "Ask Sam - venue (conflict 2026-11-15 14-03 2).md");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
