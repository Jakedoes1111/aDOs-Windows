//! Hand a finished prompt to the assistant from any module without depending on it.
//! The assistant registers the handler; it decides which chat receives the text (only one
//! aDOs started, never the owner's existing chats).
use gpui_kit::{App, Global, Window};
use std::rc::Rc;

type Handler = Rc<dyn Fn(String, &mut Window, &mut App)>;

#[derive(Default)]
struct Ask(Option<Handler>);
impl Global for Ask {}

/// The assistant calls this once per window it serves.
pub fn set_handler(cx: &mut App, handler: impl Fn(String, &mut Window, &mut App) + 'static) {
    cx.set_global(Ask(Some(Rc::new(handler))));
}

/// Send `text` to the assistant. False when no assistant is running in this build.
pub fn send(text: String, window: &mut Window, cx: &mut App) -> bool {
    let Some(handler) = cx.try_global::<Ask>().and_then(|a| a.0.clone()) else { return false };
    handler(text, window, cx);
    true
}
