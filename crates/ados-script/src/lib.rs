//! Script panels (VISION step 6 trial): a JavaScript panel from `ADOS_HOME/ui/<name>/`,
//! run by gpui-shell, hot-reloaded on save, with aDOs owning what the script may do.
mod gate;
mod host;
mod panel;

pub use gate::{Ask, Decisions, Manifest};
pub use panel::ScriptPanel;

pub fn init(cx: &mut gpui_kit::App) {
    gpui_shell::init(cx);
}
