//! Notes changed by someone else (the assistant, its Undo, another program): the editor
//! follows the files and never shows a note that is gone as "Saved".
use crate::panel::NotesPanel;
use gpui_kit::component::input::{EditorState, Position};
use gpui_kit::*;

impl NotesPanel {
    /// Checks that show an outside change to the open, saved note before the owner
    /// works on it: window activation, and every key before it reaches the editor.
    pub(crate) fn watch_outside(editor: &Entity<EditorState>, window: &mut Window, cx: &mut Context<Self>) -> Vec<Subscription> {
        // Back from another window or app (alt-tab, un-minimise): show what changed meanwhile.
        // On X11 GPUI also reports "active" again on ordinary key presses (and not always a
        // clean inactive-to-active change), so: with nothing unsaved, any "active" checks
        // the file (one read); with unsaved typing, only a real return does, since that
        // saves and would otherwise skip the typing debounce.
        let mut was_active = window.is_window_active();
        let activation = cx.observe_window_activation(window, move |this, window, cx| {
            let active = window.is_window_active();
            let returned = active && (!was_active || !this.dirty);
            was_active = active;
            if returned {
                this.refresh(cx);
                this.leave_missing(window, cx);
                this.reload_changed(window, cx);
            }
        });
        // Interceptors run before key bindings and typed text, so with nothing unsaved an
        // outside change is shown first and the keystroke lands in the current file
        // instead of forking a conflict copy. One file read per key, only while the note
        // is saved; nothing while idle.
        let weak = cx.entity().downgrade();
        let editor_focus = editor.read(cx).focus_handle(cx);
        // Focus by any other way (a click, Tab) places the caret itself.
        let placed = cx.on_focus(&editor_focus, window, |this, _, _| this.caret_unplaced = false);
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            if event.keystroke.is_modifier_key() || !editor_focus.is_focused(window) {
                return;
            }
            let _ = weak.update(cx, |this, cx| {
                if !this.dirty {
                    this.reload_changed(window, cx);
                }
            });
        });
        vec![activation, keys, placed]
    }

    /// The open note or capture no longer exists: show the next note, or the empty state.
    /// Unsaved typing is never dropped here; its save makes the file again.
    pub(crate) fn leave_missing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dirty {
            return;
        }
        let open = self.state.selected_capture.as_ref().or(self.state.selected.as_ref());
        if open.is_none_or(|p| p.exists()) {
            return;
        }
        let had_focus = self.editor.read(cx).focus_handle(cx).contains_focused(window, cx);
        self.state.selected_capture = None;
        self.state.selected = None;
        let mut text = String::new();
        if let Some(next) = self.notes.iter().find(|n| n.path.exists()) {
            if let Ok(found) = std::fs::read_to_string(&next.path) {
                self.state.selected = Some(next.path.clone());
                text = found;
            }
        }
        self.load_text(text, window, cx);
        if had_focus {
            self.focus_content(window, cx);
            // At the end, so typing adds to the next note instead of renaming it.
            self.caret_to(Position::new(u32::MAX, u32::MAX), window, cx);
        }
    }

    /// Put the editor caret at `at` (clamped to the text) when the editor has focus.
    pub(crate) fn caret_to(&mut self, at: Position, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editor.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        self.caret_unplaced = false;
        let line = at.line.min(self.text.matches('\n').count() as u32);
        self.editor.update(cx, |editor, cx| {
            editor.set_cursor_position(Position::new(line, at.character), window, cx)
        });
    }

    /// Compare the open note with its file. Changed outside with nothing unsaved: show the
    /// file, caret kept. With unsaved edits: save them, which keeps the outside version and
    /// puts ours in a conflict copy.
    pub(crate) fn reload_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.selected_capture.is_some() {
            return;
        }
        let Some(path) = self.state.selected.clone() else { return };
        if self.dirty {
            self.flush_visible(cx);
            return;
        }
        let disk = crate::disk::read(&path);
        if let crate::disk::Decision::Reload(text) = crate::disk::decide(self.known.as_ref(), disk, &self.text, false) {
            let caret = self.editor.read(cx).cursor_position();
            // Same note: a conflict sentence still applies.
            let conflict = self.conflict.take();
            self.load_text(text, window, cx);
            self.conflict = conflict;
            self.caret_to(caret, window, cx);
        }
    }
}

/// Save whatever the Notes editor holds, before another crate reads or rewrites a note
/// file (an Undo comparing against what it wrote). Fine when Notes is not open.
pub fn flush_open(cx: &mut App) -> Result<(), String> {
    let Some(panel) = crate::panel::target(cx) else { return Ok(()) };
    panel
        .update(cx, |panel, cx| panel.flush(cx))
        .map_err(|e| format!("Could not save the open note first: {e:#}"))
}
