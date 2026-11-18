//! Where typed text goes. Every edit belongs to exactly one note file; text whose note
//! could not be made is kept as an orphan draft and becomes a note on the next launch.
use crate::{file, state::Draft};
use std::path::{Path, PathBuf};

/// The file an edit in the editor is saved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The selected note.
    Note(PathBuf),
    /// Nothing editable is selected (no note, or a read-only capture): the text gets a
    /// new note of its own, and a selected capture is left untouched.
    New,
}

pub fn edit_target(selected: Option<&Path>, capture: Option<&Path>) -> Target {
    match (selected, capture) {
        (Some(path), None) => Target::Note(path.to_path_buf()),
        _ => Target::New,
    }
}

/// Turn a draft whose note was never made into a note in `dir`. The caller clears the
/// draft only after this succeeded.
pub fn recover_orphan(draft: &Draft, dir: &Path) -> anyhow::Result<PathBuf> {
    file::create_note_in(dir, &draft.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::NotesState;

    #[test]
    fn every_edit_has_exactly_one_target() {
        let note = Path::new("/n/a-1.md");
        let capture = Path::new("/i/1-thought.md");
        assert_eq!(edit_target(Some(note), None), Target::Note(note.into()));
        // The owner's state after the loss: no note, a capture selected.
        assert_eq!(edit_target(None, Some(capture)), Target::New);
        assert_eq!(edit_target(None, None), Target::New);
        assert_eq!(edit_target(Some(note), Some(capture)), Target::New);
    }

    #[test]
    fn orphan_draft_survives_state_and_becomes_a_note() {
        // Old state files (draft path is a string) and orphan drafts (null) both load.
        let old: NotesState = serde_json::from_str(r#"{"selected":null,"draft":{"path":"/n/x-1.md","text":"hi"}}"#).expect("old");
        assert_eq!(old.draft.as_ref().and_then(|d| d.path.clone()), Some(PathBuf::from("/n/x-1.md")));
        let orphan: NotesState = serde_json::from_str(r#"{"selected":null,"selected_capture":"/i/1.md","mode":"edit","draft":{"path":null,"text":"lost words"}}"#).expect("orphan");
        let draft = orphan.draft.clone().expect("draft kept");
        assert!(draft.path.is_none());
        let saved = serde_json::to_string(&orphan).expect("save");
        assert!(saved.contains("lost words"), "an orphan draft is written back, never dropped");

        let dir = std::env::temp_dir().join(format!("ados-test-notes-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = recover_orphan(&draft, &dir).expect("recovered");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "lost words");
        assert!(path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("lost-words-")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
