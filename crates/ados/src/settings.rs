use crate::{layouts, shell::Shell, storage};
use gpui_kit::*;
use gpui_kit::component::{WindowExt, input::{InputState, InputEvent}};

impl Shell {
    pub(crate) fn open_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.prompt.is_some() { return; }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Layout name"));
        self.prompt_subscription = Some(cx.subscribe_in(&input, window, |s, input, event, w, cx| {
            if matches!(event, InputEvent::PressEnter { .. })
                && s.save_layout(Some(input.read(cx).value().to_string()), cx) {
                s.close_prompt(w, cx);
            }
        }));
        ados_core::overlay::OverlayGuard::open(cx);
        window.focus(&input.focus_handle(cx), cx);
        self.prompt = Some(input);
        self.error = None;
        window.refresh();
        cx.notify();
    }
    pub(crate) fn close_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.prompt.take().is_some() { ados_core::overlay::OverlayGuard::close(cx); }
        self.prompt_subscription = None;
        window.focus(&self.focus, cx);
        window.refresh();
        window.on_next_frame(|window, _| window.refresh());
        cx.notify();
    }
    pub(crate) fn confirm_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shell = cx.entity().downgrade();
        ados_core::overlay::OverlayGuard::open(cx);
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let shell = shell.clone();
            dialog.title("Reset layouts to defaults?")
                .description("Replace the built-in layouts and their working arrangements. Your named layouts and module data will be kept.")
                .show_cancel(true)
                .on_ok(move |_, w, cx| {
                    let _ = shell.update(cx, |s, cx| s.reset_layouts(w, cx));
                    true
                })
                .on_close(|_, _, cx| ados_core::overlay::OverlayGuard::close(cx))
        });
    }
    fn reset_layouts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for name in ["Research", "Code", "Focus"] {
            let layout = layouts::default_layout(name, window, cx);
            self.dock.update(cx, |dock, cx| dock.set_center(layout, window, cx));
            let state = self.dock.read(cx).dump(cx);
            if let Err(error) = storage::save(&storage::Layout::new(name, state)) {
                self.error = Some(format!("Could not reset layouts: {error}"));
                return;
            }
            self.arrangements.remove(name);
        }
        self.name = "Research".into();
        self.revert_layout(window, cx);
        self.error = Some("Built-in layouts reset. Named layouts were kept.".into());
        cx.notify();
    }
}
