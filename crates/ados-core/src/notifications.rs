//! Shared latest-Undo action for notifications from independent modules.
use gpui_kit::*;
use std::rc::Rc;

type Undo = Rc<dyn Fn(&mut Window, &mut App)>;
#[derive(Default)]
pub struct LatestUndo {
    generation: u64,
    action: Option<Undo>,
}
impl Global for LatestUndo {}

/// Returns a token so an expired notification cannot clear a newer action.
pub fn remember_undo(action: impl Fn(&mut Window, &mut App) + 'static, cx: &mut App) -> u64 {
    let state = cx.global_mut::<LatestUndo>();
    state.generation += 1;
    state.action = Some(Rc::new(action));
    state.generation
}

pub fn forget_undo(token: u64, cx: &mut App) {
    let state = cx.global_mut::<LatestUndo>();
    if state.generation == token {
        state.action = None;
    }
}

pub fn undo_latest(window: &mut Window, cx: &mut App) -> bool {
    let action = cx.global_mut::<LatestUndo>().action.take();
    if let Some(action) = action {
        // An action may target the entity currently dispatching the shortcut.
        // Run after dispatch to avoid re-entering its mutable entity borrow.
        window.defer(cx, move |window, cx| action(window, cx));
        true
    } else {
        false
    }
}
