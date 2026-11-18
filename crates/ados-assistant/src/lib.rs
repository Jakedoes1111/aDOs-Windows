mod text;
mod tilde;
pub mod ui;

pub use text::{
    chat_label, chats_in, first_meaningful_line, folder_label, newest_first, pick_backend, prompt_text, split_notice, with_preamble,
    start_error, chat_start_error, ContextItem,
};
pub use tilde::literal_tildes;
pub use ui::Assistant;

/// Register assistant palette actions. Call after `ados_core::init`.
pub fn init(cx: &mut gpui_kit::App) {
    ui::init(cx);
}
