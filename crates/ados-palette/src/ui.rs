use super::{Mode, Palette, Next, Previous, Thought, File, Image, Link};
use ados_core::{brand, capture};
use gpui_kit::*;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, button::{Button, ButtonVariants}, input::{Textarea, MoveUp, MoveDown, Escape, IndentInline, OutdentInline}};

impl Palette {
    fn mode_button(&self, mode: Mode, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.mode == mode;
        div().id(if mode == Mode::Actions { "palette-actions" } else { "palette-capture" })
            .h(px(38.)).px_3().flex().items_center().gap_1().cursor_pointer()
            .border_b_2().border_color(if selected { rgb(brand::BLUE).into() } else { transparent_black() })
            .text_color(if selected { cx.theme().foreground } else { cx.theme().muted_foreground })
            .when(mode == Mode::Capture, |row| row.child(Icon::new(IconName::Plus).size(px(14.)).text_color(rgb(brand::RED))))
            .child(if mode == Mode::Actions { "Actions" } else { "Capture" })
            .relative()
            .child(ados_core::hints::mark_overlay())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.set_mode(mode, window, cx);
                this.focus_input(window, cx);
                cx.notify();
            }))
    }

    fn results_list(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.results.is_empty() {
            let query = self.input.read(cx).value().replace('\n', " ");
            return div().id("palette-no-results").h(px(44.)).px_6().flex().items_center()
                .bg(cx.theme().selection).cursor_pointer()
                .child(div().min_w_0().truncate().child(format!("Capture \u{201c}{query}\u{201d}")))
                .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))).into_any_element();
        }
        // Leave room for the multiline input, mode tabs, count and footer.
        let query = self.input.read(cx).value();
        let lines = query.lines().map(|line| (line.chars().count() / 55 + 1) as f32).sum::<f32>().clamp(1., 6.);
        let available = f32::from(window.viewport_size().height) * 0.74 - 190. - (lines - 1.) * 24.;
        let visible_count = self.results.len().min((available / 44.).floor().clamp(1., 8.) as usize);
        let remaining = self.results.len().saturating_sub(visible_count);
        if self.visible_rows != visible_count {
            self.visible_rows = visible_count;
            self.reveal_selected();
        }
        self.first_row = self.first_row.min(remaining);
        // Render a whole-row window instead of a pixel-scrolled uniform list.
        let list = div().id("palette-results").flex().flex_col().flex_shrink_0()
            .children((self.first_row..self.first_row + visible_count).map(|index| {
                let action = &self.results[index];
                let selected = Some(index) == self.selected;
                div().id(("palette-result", index)).h(px(44.)).flex_shrink_0().w_full().px_6()
                    .flex().items_center().gap_3().cursor_pointer()
                    .border_l_2().border_color(if selected { rgb(brand::BLUE).into() } else { transparent_black() })
                    .when(selected, |row| row.bg(cx.theme().selection))
                    .hover(|row| row.bg(cx.theme().list_hover))
                    .child(Icon::new(IconName::ChevronRight).size(px(16.)).text_color(cx.theme().muted_foreground))
                    .child(div().flex_1().min_w_0().truncate().child(action.title.clone()))
                    .children(ados_core::shortcuts::label_for(&action.id, cx).map(SharedString::from).or(action.shortcut.clone())
                        .filter(|s| !s.is_empty()).map(|shortcut| div().text_xs().text_color(cx.theme().muted_foreground).child(shortcut)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.selected = Some(index);
                        this.submit(window, cx);
                    }))
                    .child(ados_core::hints::mark_overlay())
            })).h(px(visible_count as f32 * 44.)).w_full()
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                this.wheel_remainder -= f32::from(event.delta.pixel_delta(px(44.)).y) / 44.;
                let rows = this.wheel_remainder.trunc() as isize;
                this.wheel_remainder -= rows as f32;
                this.first_row = this.first_row.saturating_add_signed(rows)
                    .min(this.results.len().saturating_sub(this.visible_rows));
                if this.selected.is_some_and(|selected| selected < this.first_row
                    || selected >= this.first_row + this.visible_rows) {
                    this.selected = None;
                }
                cx.stop_propagation();
                cx.notify();
            }));
        div().flex().flex_col().w_full().min_h_0().child(list).child(
            div().id("palette-more-hint").h(px(28.)).flex_shrink_0().px_6().flex().items_center()
                .text_xs().text_color(cx.theme().muted_foreground)
                .child(if remaining > 0 {
                    format!("{} actions · {} more offscreen · Scroll or ↑↓", self.results.len(), remaining)
                } else {
                    let count = self.results.len();
                    format!("{count} {}", if count == 1 { "action" } else { "actions" })
                })
        ).into_any_element()
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            ados_core::overlay::set_hole("palette", None, cx);
            return div().into_any_element();
        }
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let panel = cx.theme().popover;
        let query = self.input.read(cx).value();
        let kind = capture::detect(&query).label();
        // Fixed width on desktop; fit narrow windows. The top is near one quarter of
        // the canvas so the compact card's centre sits near the upper third.
        let width = (f32::from(window.viewport_size().width) - 32.).clamp(0., 600.);
        let top = f32::from(window.viewport_size().height) * 0.23;
        // Keep the card inside the viewport, including its count/footer.
        let max_card_height = f32::from(window.viewport_size().height) * 0.74;
        div().id("palette-overlay").absolute().inset_0().size_full().occlude()
            .key_context("AdosPalette")
            // GPUI dispatches bound input actions before raw key events.
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                if this.mode == Mode::Actions { this.move_selection(true, cx); cx.stop_propagation(); }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                if this.mode == Mode::Actions { this.move_selection(false, cx); cx.stop_propagation(); }
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| { this.close(window, cx); cx.stop_propagation(); }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| { this.switch_mode(window, cx); cx.stop_propagation(); }))
            .capture_action(cx.listener(|this, _: &OutdentInline, window, cx| { this.switch_mode(window, cx); cx.stop_propagation(); }))
            .on_action(cx.listener(|this, _: &Next, _, cx| this.move_selection(false, cx)))
            .on_action(cx.listener(|this, _: &Previous, _, cx| this.move_selection(true, cx)))
            .on_action(cx.listener(|this, _: &Thought, window, cx| this.footer(0, window, cx)))
            .on_action(cx.listener(|this, _: &File, window, cx| this.footer(1, window, cx)))
            .on_action(cx.listener(|this, _: &Image, window, cx| this.footer(2, window, cx)))
            .on_action(cx.listener(|this, _: &Link, window, cx| this.footer(3, window, cx)))
            .capture_key_down(cx.listener(Self::key_down))
            .child(div().id("palette-backdrop").absolute().inset_0()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| { this.close(window, cx); cx.stop_propagation(); })))
            .child(div().absolute().top(px(top)).w_full().flex().justify_center()
                .child(div().id("palette-card").w(px(width)).flex().flex_col().overflow_hidden()
                    .max_h(px(max_card_height))
                    .bg(panel).text_color(foreground).text_sm().rounded(px(10.)).border_1().border_color(border)
                    .occlude()
                    .child(ados_core::overlay::hole_tracker("palette"))
                    .child(div().w_full().h(px(1.)).flex_shrink_0().bg(linear_gradient(90.,
                        linear_color_stop(rgb(brand::BLUE), 0.), linear_color_stop(rgb(brand::RED), 1.))))
                    .child(div().flex().items_start().px_6().py_3().gap_3()
                        .child(div().pt_1().child(Icon::new(IconName::Search).size(px(19.)).text_color(muted)))
                        .child(div().flex_1().min_w_0().child(Textarea::new(&self.input).appearance(false).bordered(false)))
                        .child(div().mt_1().px_1().rounded(px(4.)).border_1().border_color(border).text_xs().text_color(muted).child("Esc")))
                    .when(self.mode == Mode::Capture, |card| card.child(div().px_6().pb_2().text_xs().text_color(muted)
                        .child(format!("{} \u{b7} Enter to capture \u{b7} Shift+Enter for a new line", kind))))
                    .child(div().flex().px_3().border_b_1().border_color(border)
                        .child(self.mode_button(Mode::Actions, cx))
                        .child(self.mode_button(Mode::Capture, cx)))
                    .when(self.mode == Mode::Actions, |card| card.child(self.results_list(window, cx)))
                    .when_some(self.error.clone(), |card, error| card.child(div().px_6().py_2().text_color(rgb(brand::RED)).child(error)))
                    .child(div().flex().items_center().gap_3().mx_3().py_2().border_t_1().border_color(border)
                        .children([
                            ("Thought \u{b7} Ctrl+1", 0, false),
                            ("Choose file \u{b7} Ctrl+2", 1, self.picker_in_flight),
                            ("Choose image \u{b7} Ctrl+3", 2, self.picker_in_flight),
                            ("Link \u{b7} Ctrl+4", 3, false),
                        ].into_iter().map(|(tooltip, index, disabled)| {
                            Button::new(("palette-kind", index)).ghost().child(crate::icons::footer(index, foreground)).child(ados_core::hints::mark_overlay()).size(px(44.)).tooltip(tooltip)
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, window, cx| this.footer(index, window, cx)))
                        }))
                        .child(div().flex_1())
                        .child(div().text_xs().text_color(muted).child("Tab to switch")))))
            .into_any_element()
    }
}
