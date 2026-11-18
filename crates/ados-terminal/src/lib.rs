//! Native Alacritty terminal hosted by a GPUI custom element.
mod colors;
mod input;
mod panel;
mod pty;
mod terminal;

use gpui_kit::*;
pub use panel::TerminalPanel;
actions!(terminal, [ClearTerminal, CopyTerminal, OpenTerminalLink]);

struct Target(WeakEntity<TerminalPanel>);
impl Global for Target {}
pub(crate) fn set_target(panel: WeakEntity<TerminalPanel>, cx: &mut App) {
    cx.set_global(Target(panel));
}

pub fn init(cx: &mut App) {
    // GPUI's root otherwise consumes Tab/Shift+Tab for focus traversal before
    // raw key listeners run. Inside a terminal these belong to the application.
    // Ctrl+L stays clear-screen here: an intended override of the browser address shortcut.
    use ados_core::shortcuts::bind;
    bind(cx, "terminal.keep_ctrl_l", NoAction);
    bind(cx, "terminal.keep_tab", NoAction);
    bind(cx, "terminal.keep_shift_tab", NoAction);
    bind(cx, "terminal.clear", ClearTerminal);
    bind(cx, "terminal.copy", CopyTerminal);
    bind(cx, "terminal.open_link", OpenTerminalLink);
    cx.on_action(|_: &ClearTerminal, cx| clear(cx));
    ados_core::actions::register(
        "terminal.clear",
        "Clear terminal",
        "terminal shell clear",
        None,
        cx,
        |_, cx| clear(cx),
    );
}
fn clear(cx: &mut App) {
    if let Some(target) = cx.try_global::<Target>().map(|t| t.0.clone()) {
        let _ = target.update(cx, |panel, cx| panel.clear(cx));
    }
}
