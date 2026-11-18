//! Mutating UI actions. Only leave the current document after a successful save.
use crate::{file, panel::NotesPanel};
use gpui_kit::component::{
    WindowExt,
    input::Position,
    button::{Button, ButtonVariants},
    notification::Notification,
};
use gpui_kit::*;

impl NotesPanel {
    pub(crate) fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.state.selected_capture.clone().or(self.state.selected.clone()) {
            self.delete_item(path, window, cx);
        }
    }

    pub(crate) fn delete_item(&mut self, path: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.state.selected_capture.as_ref().or(self.state.selected.as_ref()) == Some(&path);
        if !self.flush_visible(cx) {
            return;
        }
        // Saving can rename the selected note; trash its new path, never the stale one.
        let original = if active {
            self.state.selected_capture.clone().or(self.state.selected.clone()).unwrap_or(path)
        } else {
            path
        };
        let title = self.notes.iter().find(|n| n.path == original).map(|n| n.title.clone())
            .or_else(|| self.inbox.iter().find(|i| i.path == original)
                .map(|i| file::title_from_text(&i.text)))
            .or_else(|| std::fs::read_to_string(&original).ok().map(|s| file::title_from_text(&s)))
            .unwrap_or_else(|| "note".into());
        match file::trash_note(&original) {
            Ok(trash) => {
                self.undo.push((trash, original));
                if active {
                    self.state.selected = None;
                    self.state.selected_capture = None;
                    self.show_text(String::new(), window, cx);
                }
                self.refresh(cx);
                // Keep list commands available even when deleting the active (or last) item.
                self.list_focus.focus(window, cx);
                let weak = cx.entity().downgrade();
                // The shell's undo key (Ctrl+Shift+Z) brings it back too.
                let shell_undo = weak.clone();
                ados_core::notifications::remember_undo(move |window, cx| {
                    let _ = shell_undo.update(cx, |panel, cx| panel.undo_last(window, cx));
                }, cx);
                window.push_notification(
                    Notification::new().message(format!("Moved “{title}” to trash.")).content(|_, _, _| ados_core::hints::cover_padded(ados_core::hints::Layer::Float, px(16.)).into_any_element())
                        .id::<NotesPanel>()
                        .icon(crate::rows::trash_icon())
                        .autohide(false)
                        .action(move |_, _, _| {
                            let weak = weak.clone();
                            Button::new("undo-deleted-note")
                                .label("Undo")
                                .child(ados_core::hints::mark_in(ados_core::hints::Layer::Float))
                                .ghost()
                                .on_click(move |_, window, cx| {
                                    let _ =
                                        weak.update(cx, |panel, cx| panel.undo_last(window, cx));
                                })
                        }),
                    cx,
                );
            }
            Err(e) => {
                self.error = Some(format!("Could not move this item to trash: {e:#}"));
                cx.notify();
            }
        }
    }

    pub(crate) fn undo_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush_visible(cx) {
            return;
        }
        let Some((trash, original)) = self.undo.last().cloned() else {
            return;
        };
        match file::restore_note(&trash, &original) {
            Ok(()) => {
                self.undo.pop();
                self.refresh(cx);
                if let Some(item) = self.inbox.iter().find(|i| i.path == original).cloned() {
                    self.select_capture(item, window, cx);
                } else {
                    self.select_note(original, window, cx);
                }
                if self.undo.is_empty() {
                    window.remove_notification::<NotesPanel>(cx);
                }
                self.focus_content(window, cx);
            }
            Err(e) => {
                self.error = Some(format!(
                    "Could not restore the item: {e:#}. It is still in trash."
                ));
                cx.notify();
            }
        }
    }

    pub(crate) fn make_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush_visible(cx) {
            return;
        }
        let Some(capture_path) = self.state.selected_capture.clone() else {
            return;
        };
        match file::create_note(&self.text) {
            Ok(path) => {
                // Never retire a capture until the new note has been flushed to disk.
                let result = file::trash_note(&capture_path);
                self.state.selected_capture = None;
                self.known = Some(crate::disk::stamp(&path, &self.text));
                self.state.selected = Some(path);
                self.state.mode = "edit".into();
                self.persist(cx);
                self.refresh(cx);
                self.publish(cx);
                if let Err(e) = result {
                    self.error = Some(format!(
                        "The note was created, but the inbox copy was kept: {e:#}"
                    ));
                }
                self.notice = None;
                // Caret at the end, so typing adds to the capture instead of renaming it.
                self.editor.read(cx).focus_handle(cx).focus(window, cx);
                self.caret_to(Position::new(u32::MAX, u32::MAX), window, cx);
            }
            Err(e) => {
                self.error = Some(format!(
                    "Could not make a note; the capture is unchanged: {e:#}"
                ));
                cx.notify();
            }
        }
    }
}
