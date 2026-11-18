//! The card's header (grip, backend picker, chats, new, collapse), its two menus,
//! and the status line (starting, start error with Retry, notices, errors).
use super::agent::Conn;
use super::Assistant;
use crate::chat_label;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    ActiveTheme, IconName, Sizable, ThemeStyled,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use ados_core::hints::Layer;

/// Hint-mode mark for a control on the card (above panels, below its own menus).
pub(super) fn float_mark() -> impl IntoElement {
    ados_core::hints::mark_in(Layer::Float)
}

impl Assistant {
    fn chat_title(&self) -> String {
        match &self.saved.chat {
            None => "New chat".into(),
            Some(id) => self
                .chats
                .iter()
                .find(|c| &c.id == id)
                .map(chat_label)
                .unwrap_or_else(|| "Chat".into()),
        }
    }

    pub(super) fn header(&self, cx: &mut Context<Self>) -> Div {
        let small = |button: Button| button.ghost().small().h(px(32.)).min_w(px(32.));
        div()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .h(px(40.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                small(Button::new("assistant-grip"))
                    .label("⠿")
                    .tooltip("Drag along the top bar · Left/Right to move")
                    .child(float_mark())
                    .on_key_down(cx.listener(Self::move_with_keys))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event, window, cx| {
                            let origin = this.bubble_position(window, cx);
                            this.start_drag(event, false, origin);
                            cx.stop_propagation();
                        }),
                    ),
            )
            .child(
                small(Button::new("assistant-backend"))
                    .label(self.backend_name())
                    .tooltip("Agent · switching starts a new chat")
                    .child(float_mark())
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_backend_menu(window, cx))),
            )
            .child(
                Button::new("assistant-chats")
                    .ghost()
                    .small()
                    .h(px(32.))
                    .flex_1()
                    .min_w_0()
                    .label(self.chat_title())
                    .tooltip("Recent chats")
                    .child(float_mark())
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_chats_list(window, cx))),
            )
            .child(ados_core::hints::marked_in(
                Layer::Float,
                small(Button::new("assistant-new"))
                    .icon(IconName::Plus)
                    .tooltip("New chat")
                    .on_click(cx.listener(|this, _, window, cx| this.new_chat(window, cx))),
            ))
            .child(ados_core::hints::marked_in(
                Layer::Float,
                small(Button::new("assistant-collapse"))
                    .icon(IconName::Minus)
                    .tooltip("Collapse")
                    .on_click(cx.listener(|this, _, window, cx| this.collapse(window, cx))),
            ))
    }

    /// Open or close the backend menu. While open, the card's own focus holds the keyboard
    /// so Up/Down/Enter/Esc reach `backend_menu_key` (as in the chats list).
    pub(super) fn toggle_backend_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.backend_menu = !self.backend_menu;
        self.switcher = false;
        if self.backend_menu {
            let current = self.backend_config().map(|b| b.id.clone());
            self.backend_cursor = self.backends.iter().position(|b| Some(&b.id) == current.as_ref()).unwrap_or(0);
            window.focus(&self.focus, cx);
        } else {
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// Keys while the backend menu is open. Returns true when the key was the menu's.
    pub(super) fn backend_menu_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let count = self.backends.len();
        match key {
            "escape" => self.toggle_backend_menu(window, cx),
            "down" | "tab" if count > 0 => self.backend_cursor = (self.backend_cursor + 1) % count,
            "up" if count > 0 => self.backend_cursor = (self.backend_cursor + count - 1) % count,
            "enter" | "space" => {
                if let Some(id) = self.backends.get(self.backend_cursor).map(|b| b.id.clone()) {
                    self.switch_backend(id, window, cx);
                    self.input.update(cx, |input, cx| input.focus(window, cx));
                }
            }
            "up" | "down" | "tab" => {}
            _ => return false,
        }
        cx.notify();
        true
    }

    fn menu(id: &'static str) -> Stateful<Div> {
        div()
            .id(id)
            .max_h(px(160.))
            .overflow_y_scroll()
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
    }

    /// A floating menu under the header's agent button, drawn above the chat so
    /// opening it never moves the card's contents.
    pub(super) fn backend_menu_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.backend_config().map(|b| b.id.clone());
        let mut menu = Self::menu("assistant-backend-menu")
            .absolute()
            .top(px(40.))
            .left(px(40.))
            .min_w(px(200.))
            .occlude()
            .popover_style(cx)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(10.))
            .child(ados_core::hints::cover(Layer::Menu));
        for (ix, backend) in self.backends.iter().enumerate() {
            let id = backend.id.clone();
            let chosen = current.as_deref() == Some(backend.id.as_str());
            let highlighted = ix == self.backend_cursor;
            menu = menu.child(
                Button::new(("backend-option", ix))
                    .ghost()
                    .small()
                    .h(px(32.))
                    .border_1()
                    .border_color(if highlighted { cx.theme().ring } else { transparent_black() })
                    .when(highlighted, |b| b.bg(cx.theme().list_active))
                    .label(if chosen { format!("✓ {}", backend.name) } else { backend.name.clone() })
                    .child(ados_core::hints::mark_in(Layer::Menu))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.switch_backend(id.clone(), window, cx)
                    })),
            );
        }
        menu.into_any_element()
    }

    pub(super) fn status_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        // Bounded: a long error scrolls inside the card instead of drawing past its edge.
        let mut status = div().id("assistant-status").flex().flex_col().flex_shrink_0().max_h(px(132.)).overflow_y_scroll().px_3().gap_1();
        let line = match &self.conn {
            Conn::Starting => Some(format!("Starting {}…", self.backend_name())),
            _ if self.replaying && self.queued.is_some() => Some("Opening chat… your message sends next.".into()),
            _ if self.replaying => Some("Opening chat…".into()),
            Conn::Idle | Conn::Ready if self.rows.is_empty() => Some("Ask about your workspace".into()),
            _ => None,
        };
        if let Some(line) = line {
            status = status.py_2().child(line);
        }
        if let Conn::Failed(message) = &self.conn {
            status = status.py_2().child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().whitespace_normal().text_color(cx.theme().danger).child(message.clone()))
                    .child(
                        Button::new("assistant-retry")
                            .small()
                            .h(px(32.))
                            .label("Retry")
                            .child(float_mark())
                            .on_click(cx.listener(|this, _, window, cx| this.retry(window, cx))),
                    ),
            );
        }
        if let Some(notice) = &self.notice {
            status = status.child(div().text_xs().text_color(muted).whitespace_normal().child(notice.clone()));
        }
        if let Some(error) = &self.error {
            status = status.child(div().whitespace_normal().text_color(cx.theme().danger).child(error.clone()));
        }
        status.into_any_element()
    }
}
