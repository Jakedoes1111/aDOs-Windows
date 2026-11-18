use super::Assistant;
use gpui_kit::component::Disableable;
use gpui_kit::component::{
    button::Button,
    input::{IndentInline, Textarea},
    ActiveTheme, Sizable,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// The textarea's own vertical chrome: 1px border plus 8px editor padding on
/// each edge. The card reserves this in addition to the text rows so the
/// bottom border and the text inset are never clipped.
const INPUT_CHROME: f32 = 18.;

impl Assistant {
    pub(super) fn card(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let size = Self::available_size(window);
        let width = 440_f32.min((f32::from(size.width) - 16.).max(0.));
        // The card uses `text_sm`, whose line box is taller than the old 22px guess. Use
        // the window's real line height so the card reserves enough for the label rows
        // and never clips the first "You" label under the header.
        let line_height = f32::from(window.line_height());
        let (bubble_x, bubble_y) = self.bubble_position(window, cx);
        let input_rows =
            super::markdown::estimated_rows(&self.input.read(cx).value()).clamp(1., 6.);
        let steps_height = if self.steps.is_empty() {
            0.
        } else if self.saved.minimized {
            32.
        } else {
            32. + (self.steps.len() as f32 * 26.).min(112.)
        };
        let question_height = if self.questions.is_empty() { 0. } else { 220. };
        let wanted = 180.
            + input_rows * line_height
            + INPUT_CHROME
            + self.content_height(line_height)
            + steps_height
            + question_height
            // Room for a start error with Retry, so it never pushes the composer out.
            + if matches!(self.conn, super::agent::Conn::Failed(_)) || self.error.is_some() { 96. } else { 0. }
            + if self.switcher || self.picker { 160. } else { 0. };
        let (x, y, height) = super::position::card_rect((bubble_x, bubble_y), width, wanted, size);
        let padding = gpui_kit::component::window_paddings(window);
        ados_core::overlay::set_cutout(
            Some(Bounds {
                origin: point(px(x) + padding.left, px(y) + padding.top),
                size: gpui_kit::size(px(width), px(height)),
            }),
            cx,
        );
        let header = self.header(cx);
        let mut body = div()
            .id("assistant-card")
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(width))
            .h(px(height))
            .occlude()
            .tab_group()
            .flex()
            .flex_col()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().foreground)
            .text_sm()
            .track_focus(&self.focus)
            .child(ados_core::hints::cover(ados_core::hints::Layer::Float))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if this.tab_to_question(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.switcher {
                    if this.chats_list_key(&event.keystroke.key, window, cx) {
                        cx.stop_propagation();
                    }
                    return;
                }
                if this.backend_menu {
                    if this.backend_menu_key(&event.keystroke.key, window, cx) {
                        cx.stop_propagation();
                    }
                    return;
                }
                if event.keystroke.key == "escape" {
                    this.collapse(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(header);
        if self.picker {
            body = body.child(self.context_picker(cx));
        }
        body = body.child(self.status_view(cx));
        body = body.child(
            div().relative().flex_1().min_h_0().overflow_hidden()
                .child(div().absolute().inset_0().child(self.reading(cx)))
        ).child(self.context_chips(cx));
        body = body.child(self.steps_strip(cx));
        if !self.questions.is_empty() {
            body = body.child(self.questions_view(cx));
        }
        body.child(
            div()
                .p_2()
                .flex_shrink_0()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(Textarea::new(&self.input).w_full())
                .child(
                    div().flex().justify_end().pt_1().child(
                        Button::new("assistant-send")
                            .small()
                            .h(px(32.))
                            .min_w(px(48.))
                            .label(if self.running { "Stop" } else { "Send" })
                            .child(super::header::float_mark())
                            .disabled(!(self.running || self.can_send()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.running {
                                    this.stop_turn();
                                } else {
                                    this.send_prompt(window, cx);
                                }
                            })),
                    ),
                ),
        )
        .when(self.switcher, |body| body.child(self.chats_list(height - 52., cx)))
        .when(self.backend_menu, |body| body.child(self.backend_menu_view(cx)))
        .into_any_element()
    }
}
