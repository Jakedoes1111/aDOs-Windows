//! Shell glue: open/close the editor and turn registry notices into toasts.
use super::{KeysEditor, KeysEvent};
use crate::shell::Shell;
use ados_core::shortcuts::{self as sc, Notice};
use gpui_kit::component::{button::{Button, ButtonVariants}, notification::Notification, WindowExt};
use gpui_kit::*;

struct ConflictToast;

impl Shell {
    pub(crate) fn open_keys(&mut self, only_conflicts: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &self.keys {
            editor.update(cx, |e, cx| { e.only_conflicts = only_conflicts; cx.notify(); });
            window.focus(&editor.focus_handle(cx), cx);
            return;
        }
        // As for Settings: the editor focuses its search before the guard opens (`overlay_focus.rs`).
        if self.overlay_before.is_none() { self.note_key(window, cx); }
        let editor = cx.new(|cx| KeysEditor::new(only_conflicts, window, cx));
        self.keys_subscription = Some(cx.subscribe_in(&editor, window, |s, _, event, w, cx| match event {
            KeysEvent::Close => s.close_keys(w, cx),
        }));
        ados_core::overlay::OverlayGuard::open(cx);
        self.keys = Some(editor);
        cx.notify();
    }

    pub(crate) fn close_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.keys.take().is_some() { ados_core::overlay::OverlayGuard::close(cx); }
        self.keys_subscription = None;
        window.focus(&self.focus, cx);
        window.refresh();
        cx.notify();
    }

    /// Bad keymap entries and conflicts found while loading `keymap.toml`.
    pub(crate) fn shortcut_notices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for notice in sc::take_notices(cx) {
            match notice {
                Notice::Problem(text) => window.push_notification(Notification::warning(text), cx),
                Notice::Conflicts(count) => {
                    let shell = cx.entity().downgrade();
                    window.push_notification(
                        Notification::warning(format!("{count} shortcut conflict{}", if count == 1 { "" } else { "s" }))
                            .id::<ConflictToast>()
                            .autohide(false)
                            .action(move |_, _, _| {
                                let shell = shell.clone();
                                Button::new("show-shortcut-conflicts").label("Show").ghost()
                                    .on_click(move |_, window, cx| {
                                        let _ = shell.update(cx, |s, cx| s.open_keys(true, window, cx));
                                    })
                            }),
                        cx,
                    );
                }
            }
        }
    }
}
