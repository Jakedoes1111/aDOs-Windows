use super::*;

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ados-test-notes-logic-{}-{}",
            std::process::id(),
            short_id()
        ));
        fs::create_dir_all(&path).expect("create isolated fixture");
        Self(path)
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn titles_and_slugs_are_safe_and_bounded() {
    assert_eq!(title_from_text("# Hello world\nbody"), "Hello world");
    assert_eq!(title_from_text("###\nbody"), "Untitled");
    assert_eq!(slugify("../../hello / world?!"), "hello-world");
    assert_eq!(slugify("💡"), "untitled");
    assert_eq!(slugify("CAFÉ 日本"), "café-日本");
    assert!(slugify(&"a".repeat(1000)).len() <= 60);
}

#[test]
fn create_is_unique_and_list_uses_first_line_and_latest_first() {
    let dir = TestDir::new();
    let a = create_note_in(&dir.0, "# Alpha\nbody needle").expect("first note");
    std::thread::sleep(std::time::Duration::from_millis(20));
    let b = create_note_in(&dir.0, "# Alpha\nsecond").expect("second note");
    assert_ne!(a, b);
    let notes = list_notes_in(&dir.0).expect("list");
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].path, b);
    assert_eq!(notes[1].title, "Alpha");
    assert!(notes[1].search_text.contains("needle"));
}

#[test]
fn editing_body_does_not_rename_and_title_change_preserves_id() {
    let dir = TestDir::new();
    let original = create_note_in(&dir.0, "# Old title\nbody").expect("create");
    let unchanged = write_note(&original, "# Old title\nnew body").expect("save body");
    assert_eq!(unchanged, original);
    let renamed = write_note(&original, "# New title\nnew body").expect("save title");
    assert!(!original.exists());
    assert_eq!(
        fs::read_to_string(&renamed).expect("read"),
        "# New title\nnew body"
    );
    let id = original
        .file_stem()
        .expect("stem")
        .to_string_lossy()
        .rsplit('-')
        .next()
        .expect("id")
        .to_string();
    assert_eq!(
        renamed.file_name().expect("name").to_string_lossy(),
        format!("new-title-{id}.md")
    );
}

#[test]
fn rename_collision_never_overwrites_and_stays_stable() {
    let dir = TestDir::new();
    let old = dir.0.join("old-abc.md");
    let occupied = dir.0.join("new-abc.md");
    fs::write(&old, "# Old").expect("old fixture");
    fs::write(&occupied, "precious unrelated text").expect("collision fixture");
    let moved = write_note(&old, "# New\nbody").expect("rename around collision");
    assert_eq!(
        fs::read_to_string(occupied).expect("read occupied"),
        "precious unrelated text"
    );
    assert_eq!(moved.file_name().expect("name"), "new-1-abc.md");
    assert_eq!(
        write_note(&moved, "# New\nmore").expect("repeat save"),
        moved
    );
}

#[test]
fn trash_and_undo_preserve_contents_and_refuse_collision() {
    let dir = TestDir::new();
    let original = create_note_in(&dir.0, "# Keep me\ntext").expect("create");
    let trashed = trash_note(&original).expect("trash");
    assert!(!original.exists());
    assert!(list_notes_in(&dir.0).expect("list").is_empty());
    fs::write(&original, "another document").expect("occupy original path");
    assert!(restore_note(&trashed, &original).is_err());
    assert_eq!(
        fs::read_to_string(&original).expect("read occupied"),
        "another document"
    );
    assert_eq!(
        fs::read_to_string(&trashed).expect("read trash"),
        "# Keep me\ntext"
    );
    let _other_trash = trash_note(&original).expect("trash colliding item separately");
    restore_note(&trashed, &original).expect("undo");
    assert_eq!(
        fs::read_to_string(original).expect("read restored"),
        "# Keep me\ntext"
    );
}

#[test]
fn atomic_save_replaces_entire_document_and_leaves_no_temporary_files() {
    let dir = TestDir::new();
    let path = create_note_in(&dir.0, "a long original document").expect("create");
    atomic_write(&path, "é").expect("replace");
    assert_eq!(fs::read_to_string(path).expect("read"), "é");
    assert_eq!(fs::read_dir(&dir.0).expect("list").count(), 1);
}

#[test]
fn recovery_resolves_a_rename_before_state_commit() {
    let dir = TestDir::new();
    let old = create_note_in(&dir.0, "# Old").expect("create");
    let draft = crate::state::Draft {
        path: Some(old.clone()),
        text: "# Recovered\nunsaved text".into(),
        stamp: None,
    };
    let new = write_note(&old, &draft.text).expect("rename");
    assert_eq!(
        crate::state::draft_path(&old, &list_notes_in(&dir.0).expect("list")),
        new
    );
    let state = crate::state::NotesState {
        selected: Some(old),
        mode: "preview".into(),
        draft: Some(draft),
        ..Default::default()
    };
    let json = serde_json::to_string(&state).expect("serialize");
    let restored: crate::state::NotesState = serde_json::from_str(&json).expect("restore");
    assert_eq!(
        restored.draft.expect("draft").text,
        "# Recovered\nunsaved text"
    );
    assert_eq!(restored.mode, "preview");
}

#[test]
fn legacy_state_remains_readable() {
    let state: crate::state::NotesState =
        serde_json::from_str(r#"{"selected":null,"mode":"preview"}"#).expect("legacy state");
    assert_eq!(state.mode, "preview");
    assert!(state.draft.is_none());
}

#[test]
fn undo_of_append_keeps_owner_edits_or_refuses() {
    let before = "# Gamma\n\nline one\n";
    let added = "Assistant block";
    let written = appended(before, added);
    assert_eq!(written, "# Gamma\n\nline one\n\nAssistant block\n");
    // Untouched since: back to exactly what was there.
    assert_eq!(without_appended(&written, before, added).as_deref(), Some(before));
    // The owner typed under the block: only the block goes.
    let typed = format!("{written}Owner reply xi\n");
    assert_eq!(
        without_appended(&typed, before, added).as_deref(),
        Some("# Gamma\n\nline one\nOwner reply xi\n")
    );
    // The owner typed above it: kept too.
    let above = written.replace("line one", "line one edited");
    assert_eq!(
        without_appended(&above, before, added).as_deref(),
        Some("# Gamma\n\nline one edited\n")
    );
    // The owner edited inside the block: refuse.
    let inside = written.replace("Assistant block", "Assistant block, fixed");
    assert_eq!(without_appended(&inside, before, added), None);
    // The block appears twice (the owner pasted it): refuse rather than guess.
    let twice = format!("{written}\n\nAssistant block\n");
    assert_eq!(without_appended(&twice, before, added), None);
}
