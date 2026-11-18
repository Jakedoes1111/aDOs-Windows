use crate::{
    disk,
    file::{self, NoteMeta},
    journal::{self, Target},
    state::{self, Draft, NotesState},
};
use ados_core::{
    capture::{self, Capture},
    hub::{ContextHub, Snapshot},
    ids,
};
use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    input::{EditorState, InputEvent, InputState, Position},
    text::TextViewState,
};
use gpui_kit::*;
use std::{path::PathBuf, time::Duration};

actions!(ados_notes, [NewNote, OpenLink, TogglePreview, FocusSearch, DeleteSelected, UndoDelete, SelectPrevious, SelectNext]);

#[derive(Default)]
struct NotesTarget(Option<WeakEntity<NotesPanel>>);
impl Global for NotesTarget {}

/// Show the notes list and inbox again in an open Notes panel, without moving focus
/// (another crate, e.g. aDOs's MCP server, changed the files).
/// The open note follows its file: changed on disk shows the new text, gone leaves it.
pub fn refresh_list(window: &mut Window, cx: &mut App) {
    if let Some(panel) = target(cx) {
        let _ = panel.update(cx, |panel, cx| {
            panel.refresh(cx);
            panel.leave_missing(window, cx);
            panel.reload_changed(window, cx);
        });
    }
}

pub(crate) fn target(cx: &App) -> Option<Entity<NotesPanel>> {
    cx.try_global::<NotesTarget>().and_then(|t| t.0.clone()).and_then(|w| w.upgrade())
}

pub fn init(cx: &mut App) {
    cx.set_global(NotesTarget::default());
    use ados_core::shortcuts::bind;
    bind(cx, "notes.new", NewNote);
    bind(cx, "notes.preview", TogglePreview);
    bind(cx, "notes.open_link", OpenLink);
    bind(cx, "notes.search", FocusSearch);
    bind(cx, "notes.delete", DeleteSelected);
    bind(cx, "notes.undo_delete", UndoDelete);
    bind(cx, "notes.previous", SelectPrevious);
    bind(cx, "notes.next", SelectNext);
    ados_core::actions::register(
        "notes.new",
        "New note",
        "notes markdown write",
        None,
        cx,
        |window, cx| {
            if let Some(panel) = cx.global::<NotesTarget>().0.clone() {
                let _ = panel.update(cx, |panel, cx| panel.new_note(window, cx));
            }
        },
    );
    ados_core::actions::register("notes.open_link", "Open link in note", "notes url browser", None, cx, |window, cx| {
        if let Some(panel) = cx.global::<NotesTarget>().0.clone() {
            let _ = panel.update(cx, |panel, cx| panel.open_link(window, cx));
        }
    });
    ados_core::actions::register("notes.delete", "Delete note", "notes inbox trash remove", None, cx, |window, cx| {
        if let Some(panel) = cx.global::<NotesTarget>().0.clone() {
            let _ = panel.update(cx, |panel, cx| panel.delete_selected(window, cx));
        }
    });
    ados_core::actions::register("notes.undo_delete", "Undo delete", "notes inbox restore trash", None, cx, |window, cx| {
        if let Some(panel) = cx.global::<NotesTarget>().0.clone() {
            let _ = panel.update(cx, |panel, cx| panel.undo_last(window, cx));
        }
    });
    ados_core::actions::register("notes.search", "Filter notes list", "notes inbox find filter", None, cx, |window, cx| {
        if let Some(panel) = cx.global::<NotesTarget>().0.clone() {
            let _ = panel.update(cx, |panel, cx| panel.focus_search(window, cx));
        }
    });
}

#[derive(Clone)]
pub struct NotesChanged;

pub struct NotesPanel {
    /// Tracked on the panel root, so focus anywhere inside Notes is inside this handle.
    pub(crate) root_focus: FocusHandle,
    pub(crate) list_focus: FocusHandle,
    pub(crate) row_focus: std::collections::HashMap<PathBuf, FocusHandle>,
    pub(crate) narrow: bool,
    /// The last list row that had focus and where it was, so ↑/↓ continue from its place after it is removed.
    pub(crate) last_row: Option<(usize, PathBuf)>,
    pub(crate) list_open: bool,
    pub(crate) search: Entity<InputState>,
    pub(crate) editor: Entity<EditorState>,
    pub(crate) preview: Entity<TextViewState>,
    pub(crate) notes: Vec<NoteMeta>,
    pub(crate) inbox: Vec<Capture>,
    pub(crate) state: NotesState,
    pub(crate) text: String,
    pub(crate) error: Option<String>,
    /// A plain sentence about something done for the owner (a recovered draft).
    pub(crate) notice: Option<String>,
    pub(crate) dirty: bool,
    /// The loaded note's caret is still where loading left it (the start), so entering
    /// Notes from the keyboard should put it at the end. Any direct focus clears it.
    pub(crate) caret_unplaced: bool,
    /// The open note's file as the editor last loaded or saved it.
    pub(crate) known: Option<disk::Stamp>,
    /// The name of the conflict copy unsaved edits went to because the file changed
    /// outside; shown as one short line until dismissed.
    pub(crate) conflict: Option<String>,
    pub(crate) undo: Vec<(PathBuf, PathBuf)>,
    pub(crate) query: String,
    save_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    _capture_task: Task<()>,
}

impl NotesPanel {
    /// Stable handle on the panel root: focus anywhere in Notes is inside it.
    pub fn root_focus(&self) -> FocusHandle {
        self.root_focus.clone()
    }
    pub fn focus_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.selected.is_some() && self.state.selected_capture.is_none() {
            self.state.mode = "edit".into();
            self.list_open = false;
            self.persist(cx);
        }
        if let Some(capture) = self.state.selected_capture.clone() {
            // A capture is read-only: keys go to its row (typing shows how to edit it),
            // never into the search field.
            match self.row_focus.get(&capture) {
                Some(row) if !self.narrow => row.focus(window, cx),
                _ => self.list_focus.focus(window, cx),
            }
        } else {
            let place = self.caret_unplaced;
            self.focus_handle(cx).focus(window, cx);
            // A note restored at launch (or reloaded while away) has its caret at the start:
            // Ctrl+2, the palette and Home put it at the end, so typing adds to the note
            // instead of renaming it. A caret the owner placed stays where it is.
            if place { self.caret_to(Position::new(u32::MAX, u32::MAX), window, cx); }
        }
        cx.notify();
    }

    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut error = None;
        let mut state = state::load().unwrap_or_else(|e| {
            error = Some(format!("Could not restore notes state: {e}"));
            NotesState::default()
        });
        let mut notice = None;
        // Text whose note was never made (an orphan draft) becomes a note now.
        if let Some(draft) = state.draft.clone().filter(|d| d.path.is_none()) {
            match journal::recover_orphan(&draft, &ados_core::paths::notes_dir()) {
                Ok(path) => {
                    state.draft = None;
                    state.selected = Some(path);
                    state.selected_capture = None;
                    state.mode = "edit".into();
                    if let Err(e) = state::save(&state) {
                        error = Some(format!("Recovered your unsaved text, but could not remember it: {e:#}"));
                    }
                    notice = Some("Text that was not saved last time is now this note.".to_string());
                }
                Err(e) => {
                    error = Some(format!(
                        "Text from last time is kept, but could not become a note yet: {e:#}"
                    ));
                }
            }
        }
        let notes = file::list_notes().unwrap_or_else(|e| {
            error = Some(e.to_string());
            Vec::new()
        });
        let mut captures = capture::subscribe();
        let capture_task = cx.spawn(async move |weak, cx| {
            loop {
                captures.changed().await;
                if weak.update(cx, |this, cx| {
                    this.inbox = capture::list();
                    cx.notify();
                }).is_err() { break; }
            }
        });
        let inbox = capture::list();
        let dirty = state.draft.is_some();
        let mut known = None;
        let text = if let Some(draft) = &state.draft {
            state.selected = draft.path.as_deref().map(|p| state::draft_path(p, &notes));
            state.selected_capture = None;
            // The file may have changed after the crash (a sync, `git pull`): the save
            // below then keeps it and puts the draft in a conflict copy.
            if let Some(path) = &state.selected {
                let on_disk = std::fs::read_to_string(path).ok();
                known = disk::recovered_known(draft.stamp, on_disk.as_deref(), &draft.text);
            }
            draft.text.clone()
        } else if let Some(item) = inbox
            .iter()
            .find(|i| Some(&i.path) == state.selected_capture.as_ref())
        {
            item.text.clone()
        } else {
            state.selected_capture = None;
            if state.selected.as_ref().is_none_or(|p| !p.exists()) {
                state.selected = notes.first().map(|n| n.path.clone());
            }
            if let Some(path) = state.selected.clone() {
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        known = Some(disk::stamp(&path, &text));
                        text
                    }
                    Err(e) => {
                        error = Some(format!("Could not open {}: {e}", path.display()));
                        state.selected = None;
                        String::new()
                    }
                }
            } else {
                String::new()
            }
        };
        let root_focus = cx.focus_handle();
        let list_focus = cx.focus_handle();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .searchable(true)
                .language("markdown")
                .line_number(false)
                .soft_wrap(true)
                .default_value(text.clone())
        });
        // Compare edits with what the editor holds (it may normalise line endings).
        let text = if dirty { text } else { editor.read(cx).value().to_string() };
        let preview = cx.new(|cx| TextViewState::markdown(&crate::preview::hard_breaks(&text), cx));
        let mut subscriptions = vec![
            cx.subscribe_in(&search, window, |this, _, event, window, cx| match event {
                InputEvent::Change => {
                    this.query = this.search.read(cx).value().to_string();
                    cx.notify();
                }
                // Enter in the search box opens the first match (the ringed row).
                InputEvent::PressEnter { .. } => this.open_first_match(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&editor, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.changed(window, cx),
                InputEvent::Blur => {
                    this.flush_visible(cx);
                }
                InputEvent::Focus => this.reload_changed(window, cx),
                _ => {}
            }),
            cx.on_release(|this, cx| {
                if let Err(e) = this.flush(cx) {
                    eprintln!("Could not save notes on close; recovery draft retained: {e:#}");
                }
            }),
            cx.on_app_quit(|this, cx| {
                // Synchronous: the debounce may not have run yet when the window closes.
                if let Err(e) = this.flush(cx) {
                    eprintln!("Could not save notes on quit; recovery draft retained: {e:#}");
                }
                async {}
            }),
            cx.on_focus_in(&root_focus, window, |this, window, cx| {
                ContextHub::set_active(ids::NOTES, cx);
                this.refresh(cx);
                this.leave_missing(window, cx);
                this.reload_changed(window, cx);
                this.publish(cx);
            }),
        ];
        subscriptions.extend(Self::watch_outside(&editor, window, cx));
        cx.global_mut::<NotesTarget>().0 = Some(cx.entity().downgrade());
        let mut panel = Self {
            root_focus,
            list_focus,
            row_focus: Default::default(),
            narrow: false,
            last_row: None,
            list_open: false,
            search,
            editor,
            preview,
            notes,
            inbox,
            state,
            text,
            error,
            notice,
            dirty,
            caret_unplaced: true,
            known,
            conflict: None,
            undo: Vec::new(),
            query: String::new(),
            save_task: None,
            _subscriptions: subscriptions,
            _capture_task: capture_task,
        };
        if panel.dirty {
            panel.flush_visible(cx);
        }
        panel.publish(cx);
        panel
    }

    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        if text == self.text {
            return;
        }
        // The recovery sentence has done its job once the owner writes on.
        self.notice = None;
        self.take_edit(text);
        self.preview
            .update(cx, |view, cx| view.set_text(&crate::preview::hard_breaks(&self.text), cx));
        self.publish(cx);
        self.save_task = Some(cx.spawn_in(window, async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = cx.update(|_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.flush_visible(cx);
                });
            });
        }));
        cx.notify();
    }

    /// Record `text` as the editor's content: it always belongs to exactly one note. With no
    /// note selected (or a read-only capture), a new note is made for it at once. The text
    /// is journaled in `state/notes.json` before anything else, so a crash, a kill or a
    /// failed write never loses it; with no note it stays there as an orphan draft.
    fn take_edit(&mut self, text: String) {
        self.text = text;
        self.dirty = true;
        let selected = self.state.selected.as_deref();
        let capture = self.state.selected_capture.as_deref();
        if journal::edit_target(selected, capture) == Target::New {
            match file::create_note("") {
                Ok(path) => {
                    self.known = Some(disk::stamp(&path, ""));
                    self.state.selected = Some(path);
                    self.state.selected_capture = None;
                    self.state.mode = "edit".into();
                }
                Err(e) => {
                    self.state.selected = None;
                    self.error = Some(format!(
                        "Could not make a note for this text: {e:#}. It is kept and comes back as a note next time."
                    ));
                }
            }
        }
        self.state.draft = Some(Draft {
            path: self.state.selected.clone(),
            text: self.text.clone(),
            stamp: self.known.as_ref().map(|k| k.hash),
        });
        // A small atomic recovery journal protects the debounce window and unexpected exits.
        if let Err(e) = state::save(&self.state) {
            self.error = Some(format!(
                "Could not protect this draft: {e:#}. Keep this panel open and retry."
            ));
        }
    }

    /// Pick up editor text no change event reported (an input method's commit, say):
    /// every save path reads the editor itself, never only the last event.
    fn sync_editor(&mut self, cx: &App) {
        let value = self.editor.read(cx).value();
        if value.as_ref() != self.text {
            self.take_edit(value.to_string());
        }
    }

    pub(crate) fn flush(&mut self, cx: &App) -> anyhow::Result<()> {
        self.sync_editor(cx);
        if self.dirty && self.state.selected.is_none() {
            // An orphan draft: make its note now (Retry), or keep it and say so.
            let path = file::create_note("")?;
            self.state.selected = Some(path);
            self.state.selected_capture = None;
        }
        if self.dirty {
            if let Some(path) = self.state.selected.clone() {
                // Never overwrite a change made outside: keep that file, save ours beside it.
                let new_path = match disk::decide(self.known.as_ref(), disk::read(&path), &self.text, true) {
                    disk::Decision::Conflict => {
                        let title = file::title_from_text(&self.text);
                        let copy = disk::conflict_path(&path, &title, &disk::now_label());
                        file::write_new(&copy, &self.text)?;
                        let name = copy.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        self.conflict = Some(name);
                        copy
                    }
                    _ => file::write_note(&path, &self.text)?,
                };
                self.known = Some(disk::stamp(&new_path, &self.text));
                self.state.selected = Some(new_path);
                // Keep the draft until both document and state have reached disk.
                let mut committed = self.state.clone();
                committed.draft = None;
                state::save(&committed)?;
                self.state = committed;
                self.dirty = false;
                if let Ok(notes) = file::list_notes() {
                    self.notes = notes;
                }
            }
        }
        state::save(&self.state)
    }

    pub(crate) fn flush_visible(&mut self, cx: &mut Context<Self>) -> bool {
        match self.flush(cx) {
            Ok(()) => {
                self.error = None;
                cx.emit(NotesChanged);
                self.publish(cx);
                cx.notify();
                true
            }
            Err(e) => {
                self.error = Some(format!(
                    "Could not save your note: {e:#}. Your draft is kept; retry before switching."
                ));
                cx.notify();
                false
            }
        }
    }

    pub(crate) fn publish(&self, cx: &mut Context<Self>) {
        let location = self
            .state
            .selected_capture
            .as_ref()
            .or(self.state.selected.as_ref());
        let selection = self.editor.read(cx).selected_text().to_string();
        // Stay within core's byte cap without splitting a UTF-8 codepoint.
        let mut end = self.text.len().min(64 * 1024);
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        ContextHub::publish(
            Snapshot {
                module: ids::NOTES.into(),
                title: file::title_from_text(&self.text).into(),
                location: location.map(|p| p.to_string_lossy().into_owned().into()),
                text: self.text[..end].to_owned().into(),
                selection: (!selection.is_empty()
                    && self.state.mode != "preview"
                    && self.state.selected_capture.is_none())
                .then(|| selection.into()),
            },
            cx,
        );
    }

    pub(crate) fn show_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.load_text(text, window, cx);
        self.focus_content(window, cx);
    }

    /// Put `text` in the editor and preview without moving focus.
    pub(crate) fn load_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.known = match (&self.state.selected, &self.state.selected_capture) {
            (Some(path), None) => Some(disk::stamp(path, &text)),
            _ => None,
        };
        self.text = text;
        self.caret_unplaced = true;
        self.notice = None;
        self.conflict = None;
        self.editor.update(cx, |editor, cx| {
            editor.set_search_query("", true, cx);
            editor.close_search(cx);
            editor.set_value(self.text.clone(), window, cx)
        });
        self.text = self.editor.read(cx).value().to_string();
        self.preview
            .update(cx, |view, cx| view.set_text(&crate::preview::hard_breaks(&self.text), cx));
        self.persist(cx);
        self.publish(cx);
        cx.notify();
    }

    pub(crate) fn escape_to_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conflict.take().is_some() {
            // Esc first dismisses the conflict line; focus stays where it is.
            cx.notify();
            return;
        }
        if self.editor.read(cx).search_session().open {
            self.editor.update(cx, |editor, cx| {
                editor.close_search(cx);
                editor.focus(window, cx);
            });
        } else {
            self.flush_visible(cx);
            self.focus_search(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn focus_content(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.selected.is_some()
            && self.state.selected_capture.is_none()
            && self.state.mode == "edit"
        {
            self.editor.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            // `self.focus` is never tracked by an element, so focusing it leaves keys dead (round 5, R3).
            self.list_focus.focus(window, cx);
        }
    }

    pub(crate) fn persist(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = state::save(&self.state) {
            self.error = Some(format!("Could not remember notes state: {e:#}"));
        }
        cx.notify();
    }

    pub(crate) fn select_note(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.flush_visible(cx) {
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                self.state.selected = Some(path);
                self.state.selected_capture = None;
                self.show_text(text, window, cx);
                // Caret at the end (as "Make note"), so typing adds to the note instead
                // of renaming it: search Enter, row click, hint pick, Undo restore.
                self.caret_to(Position::new(u32::MAX, u32::MAX), window, cx);
            }
            Err(e) => {
                self.error = Some(format!("Could not open note: {e}"));
                cx.notify();
            }
        }
    }

    pub(crate) fn select_capture(
        &mut self,
        item: Capture,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.flush_visible(cx) {
            return;
        }
        self.state.selected = None;
        self.state.selected_capture = Some(item.path);
        self.show_text(item.text, window, cx);
    }

    pub(crate) fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush_visible(cx) {
            return;
        }
        match file::create_note("") {
            Ok(path) => {
                self.state.selected = Some(path);
                self.state.selected_capture = None;
                self.state.mode = "edit".into();
                self.search
                    .update(cx, |search, cx| search.set_value("", window, cx));
                self.query.clear();
                self.refresh(cx);
                self.show_text(String::new(), window, cx);
                self.editor.read(cx).focus_handle(cx).focus(window, cx);
            }
            Err(e) => {
                self.error = Some(format!("Could not create a note: {e:#}"));
                cx.notify();
            }
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        match file::list_notes() {
            Ok(notes) => self.notes = notes,
            Err(e) => self.error = Some(format!("Could not list notes: {e:#}")),
        }
        self.inbox = capture::list();
        cx.emit(NotesChanged);
        cx.notify();
    }

    pub(crate) fn toggle_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush_visible(cx) {
            return;
        }
        self.state.mode = if self.state.mode == "preview" {
            "edit"
        } else {
            "preview"
        }
        .into();
        self.persist(cx);
        self.focus_content(window, cx);
    }
}

impl EventEmitter<PanelEvent> for NotesPanel {}
impl EventEmitter<NotesChanged> for NotesPanel {}
impl Focusable for NotesPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.state.selected.is_some() && self.state.mode == "edit" {
            self.editor.read(cx).focus_handle(cx)
        } else if self.state.selected_capture.is_some() {
            // Read-only capture: the list, where typing says how to edit it.
            self.list_focus.clone()
        } else {
            self.search.read(cx).focus_handle(cx)
        }
    }
}
impl Panel for NotesPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Notes"
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}
impl BasePanel for NotesPanel {
    fn panel_name(&self) -> &'static str {
        ids::NOTES
    }
    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_visible(cx);
    }
    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if active {
            // Selected tabs in different splits can all be visible; only focus owns context.
            self.refresh(cx);
            self.leave_missing(window, cx);
            self.reload_changed(window, cx);
            self.publish(cx);
        } else {
            self.flush_visible(cx);
        }
    }
}
