//! F5: when the last overlay closes (palette, capture card, menus, key editor, layout-name
//! prompt), the keyboard goes back to exactly where it was before the first one opened,
//! inside the Browser page included. A menu opened by a click focused its trigger button
//! first, so for a click-opened overlay "before" is the focus just before that click.
use crate::shell::Shell;
use gpui_kit::*;
use std::time::{Duration, Instant};

/// A click that opens a menu does so within the same press; older presses are history.
const CLICK_OPENS: Duration = Duration::from_millis(800);

impl Shell {
    /// Shell root, capture phase: remember the focus before a GPUI press moves it.
    pub(crate) fn note_press(&mut self, window: &Window, cx: &App) {
        self.pre_click = window.focused(cx).map(|focus| (focus, Instant::now()));
    }
    /// A key typed in aDOs, capture phase: the focus before the chord that may open an overlay.
    /// Read later, the focus is already the overlay's own input (palette from the assistant
    /// composer: the keyboard was then put back on that input, which is gone).
    pub(crate) fn note_key(&mut self, window: &Window, cx: &App) {
        self.pre_click = window.focused(cx).map(|focus| (focus, Instant::now()));
    }
    pub(crate) fn overlay_changed(&mut self, before: usize, now: usize, window: &mut Window, cx: &mut Context<Self>) {
        if before == 0 && now > 0 {
            let click = self.pre_click.take().filter(|(_, at)| at.elapsed() < CLICK_OPENS).map(|(focus, _)| focus);
            self.overlay_before = click.or_else(|| window.focused(cx));
        }
        if before > 0 && now == 0 {
            let Some(target) = self.overlay_before.take() else { return };
            // After the frame without the overlay is drawn: the palette has restored its own
            // previous focus and run its action by then, and a module either of them focused
            // is left alone. Earlier, GPUI can judge the target absent and report focus lost.
            let shell = cx.entity().downgrade();
            window.on_next_frame(move |window, cx| {
                let _ = shell.update(cx, |s, cx| {
                    if ados_core::overlay::OverlayGuard::any(cx) || target.is_focused(window) { return; }
                    let in_module = s.panels(cx).iter().any(|panel| crate::modules::root_focus(panel, cx).contains_focused(window, cx));
                    if in_module { return; }
                    window.focus(&target, cx);
                    // Focus listeners (the Browser hands X focus to the page in one) run on a frame.
                    window.refresh();
                });
            });
            window.refresh();
        }
    }
}
