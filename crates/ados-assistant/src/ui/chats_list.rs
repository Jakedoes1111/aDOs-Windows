//! The chats list: an opaque floating list under the card's header. Lists the chats
//! started in the card's folder ($HOME) unless "All folders" is on. Up/Down move,
//! Enter opens, Esc closes.
use super::agent::{home, Conn};
use super::Assistant;
use crate::{chat_label, chats_in, folder_label};
use ados_agents::SessionSummary;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    ActiveTheme, Selectable, Sizable, ThemeStyled,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// How many chats the list shows (the first `session/list` page, newest first).
const MAX_CHATS: usize = 30;

impl Assistant {
    pub(super) fn visible_chats(&self) -> Vec<SessionSummary> {
        let mut chats = chats_in(&self.chats, &home(), self.saved.all_folders);
        chats.truncate(MAX_CHATS);
        chats
    }

    pub(super) fn toggle_chats_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switcher = !self.switcher;
        self.backend_menu = false;
        self.chat_cursor = 0;
        if self.switcher {
            // The card's own focus, not the composer's: the textarea's Up/Down/Enter
            // bindings would take the keys before the list sees them.
            window.focus(&self.focus, cx);
            self.list_chats(window, cx);
        } else {
            self.focus_composer(window, cx);
        }
        cx.notify();
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Keys while the list is open. Returns true when the key was the list's.
    pub(super) fn chats_list_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let count = self.visible_chats().len();
        match key {
            "escape" => {
                self.switcher = false;
                self.focus_composer(window, cx);
            }
            "down" if count > 0 => self.chat_cursor = (self.chat_cursor + 1).min(count - 1),
            "up" => self.chat_cursor = self.chat_cursor.saturating_sub(1),
            "enter" => {
                if let Some(chat) = self.visible_chats().into_iter().nth(self.chat_cursor) {
                    self.switcher = false;
                    self.open_chat(chat.id, chat.cwd, window, cx);
                }
            }
            "down" => {}
            _ => return false,
        }
        self.chats_scroll.scroll_to_item(self.chat_cursor);
        cx.notify();
        true
    }

    /// The floating list; `max_height` keeps it inside the card.
    pub(super) fn chats_list(&self, max_height: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, active, hover, ring) =
            (theme.muted_foreground, theme.border, theme.list_active, theme.list_hover, theme.ring);
        let all = self.saved.all_folders;
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .h(px(40.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(border)
            .child(div().text_color(muted).child("Chats"))
            .child(
                Button::new("assistant-all-folders")
                    .ghost()
                    .small()
                    .h(px(32.))
                    .selected(all)
                    .label(if all { "✓ All folders" } else { "All folders" })
                    .tooltip("List chats from every folder, not just your home folder")
                    .child(ados_core::hints::mark_in(ados_core::hints::Layer::Menu))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.saved.all_folders = !this.saved.all_folders;
                        this.chat_cursor = 0;
                        this.persist();
                        cx.notify();
                    })),
            );
        let chats = self.visible_chats();
        let mut rows = div()
            .id("assistant-chat-rows")
            .flex()
            .flex_col()
            .gap_1()
            .p_1()
            .max_h(px((max_height - 42.).max(40.)))
            .overflow_y_scroll()
            .track_scroll(&self.chats_scroll);
        if chats.is_empty() {
            let text = if self.chats_loading || matches!(self.conn, Conn::Starting) {
                "Loading chats…"
            } else if all {
                "No earlier chats"
            } else {
                "No earlier chats in your home folder"
            };
            rows = rows.child(div().px_2().py_2().text_color(muted).child(text));
        }
        for (ix, chat) in chats.into_iter().enumerate() {
            let current = ix == self.chat_cursor;
            let label = chat_label(&chat);
            let folder = folder_label(&chat.cwd);
            let (id, cwd) = (chat.id, chat.cwd);
            rows = rows.child(
                div()
                    .id(("recent-chat", ix))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .min_h(px(32.))
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if current { ring } else { transparent_black() })
                    .when(current, |row| row.bg(active))
                    .hover(move |style| style.bg(hover))
                    .cursor_pointer()
                    .relative()
                    .child(ados_core::hints::mark_in(ados_core::hints::Layer::Menu))
                    .child(div().truncate().child(label))
                    .when(all, |row| row.child(div().text_xs().text_color(muted).truncate().child(folder)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.switcher = false;
                        this.open_chat(id.clone(), cwd.clone(), window, cx)
                    })),
            );
        }
        div()
            .id("assistant-chats")
            .absolute()
            .top(px(44.))
            .left(px(8.))
            .right(px(8.))
            .occlude()
            .flex()
            .flex_col()
            .popover_style(cx)
            .border_1()
            .border_color(border)
            .rounded(px(10.))
            .child(ados_core::hints::cover(ados_core::hints::Layer::Menu))
            .child(header)
            .child(rows)
            .into_any_element()
    }
}
