//! Every command the palette can run. Modules and the shell register theirs in `init`.
use gpui_kit::{App, Global, SharedString, Window};
use std::rc::Rc;

#[derive(Clone)]
pub struct PaletteAction {
    /// Stable id, e.g. "notes.new". Registering the same id again replaces it.
    pub id: SharedString,
    pub title: SharedString,
    /// Extra words to match on.
    pub keywords: SharedString,
    /// Shortcut text shown on the right, e.g. "Ctrl+N". Display only.
    pub shortcut: Option<SharedString>,
    pub run: Rc<dyn Fn(&mut Window, &mut App)>,
}

#[derive(Default)]
pub struct ActionRegistry { actions: Vec<PaletteAction> }
impl Global for ActionRegistry {}

impl ActionRegistry {
    pub fn register(action: PaletteAction, cx: &mut App) {
        let reg = cx.global_mut::<Self>();
        reg.actions.retain(|a| a.id != action.id);
        reg.actions.push(action);
    }
    pub fn unregister(id: &str, cx: &mut App) {
        cx.global_mut::<Self>().actions.retain(|a| a.id.as_ref() != id);
    }
    pub fn all(cx: &App) -> Vec<PaletteAction> { cx.global::<Self>().actions.clone() }
}

/// Helper so a module can register in one line.
pub fn register(
    id: &'static str, title: &'static str, keywords: &'static str, shortcut: Option<&'static str>,
    cx: &mut App, run: impl Fn(&mut Window, &mut App) + 'static,
) {
    ActionRegistry::register(PaletteAction {
        id: id.into(), title: title.into(), keywords: keywords.into(),
        shortcut: shortcut.map(Into::into), run: Rc::new(run),
    }, cx);
}
