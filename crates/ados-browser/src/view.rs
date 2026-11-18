//! Browser toolbar and the GPUI focus proxy around the native viewport.
use super::*;
use gpui_kit::component::Disableable;
use gpui_kit::component::{
    button::{Button, ButtonVariants}, input::Input, ActiveTheme, IconName, Sizable,
};
use gpui_kit::prelude::FluentBuilder;
use ados_core::hints::marked;

impl Render for BrowserPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let error = self.error.clone();
        // With an error the pages are hidden: no divider may sit over the message and its Retry.
        let dividers = if error.is_some() { vec![] } else { self.dividers(cx) };
        let root = div()
            .id("browser-panel");
        self.tile_actions(root, cx)
            .capture_any_mouse_down(cx.listener(|s, _, w, cx| s.focus_primary(w, cx)))
            .key_context("BrowserPanel")
            .track_focus(&self.focus)
            .size_full().flex().flex_col().overflow_hidden()
            .bg(cx.theme().background)
            .on_action(cx.listener(|this, _: &FocusAddress, window, cx| this.focus_address(window, cx)))
            .on_action(cx.listener(|this, _: &Reload, _, _| this.reload()))
            .on_action(cx.listener(|this, _: &Back, _, _| this.go(-1)))
            .on_action(cx.listener(|this, _: &Forward, _, _| this.go(1)))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.new_tab(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| this.close_tab(None, window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle_tab(1, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| this.cycle_tab(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ReopenTab, window, cx| this.reopen_tab(window, cx)))
            .on_action(cx.listener(|this, _: &comment::CommentMode, window, cx| this.toggle_comment(window, cx)))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| this.comment_key(event, window, cx)))
            // Tab from the address bar: Back, Forward, Reload, engine, then the page.
            .tab_group()
            .child(self.tab_strip(cx))
            .child(
                div().flex().items_center().gap_1().px_1().h(px(36.)).flex_shrink_0()
                    .bg(cx.theme().secondary).border_b_1().border_color(cx.theme().border)
                    .child(marked(Button::new("browser-back").tab_index(1).icon(IconName::ArrowLeft).ghost().size(px(32.))
                        .tooltip(ados_core::shortcuts::tooltip("Back", "browser.back", cx))
                        .disabled(!self.nav.can_back())
                        .on_click(cx.listener(|this, _, _, _| this.go(-1)))))
                    .child(marked(Button::new("browser-forward").tab_index(1).icon(IconName::ArrowRight).ghost().size(px(32.))
                        .tooltip(ados_core::shortcuts::tooltip("Forward", "browser.forward", cx))
                        .disabled(!self.nav.can_forward())
                        .on_click(cx.listener(|this, _, _, _| this.go(1)))))
                    .child(marked(Button::new("browser-reload").tab_index(1).icon(IconName::RotateCw).ghost().size(px(32.))
                        .tooltip(ados_core::shortcuts::tooltip("Reload", "browser.reload", cx))
                        .on_click(cx.listener(|this, _, _, _| this.reload()))))
                    .children(self.comment_buttons(cx))
                    .child(div().flex_1().min_w_0().relative()
                        .child(Input::new(&self.input).small().w_full().tab_index(0))
                        .child(ados_core::hints::mark()))
                    .child(marked(Button::new("browser-engine").tab_index(2).icon(IconName::Globe)
                        .when(!self.narrow, |button| button.label(self.state.engine.label()))
                        .ghost().h(px(32.))
                        .tooltip(format!("{}: switch to {}", self.state.engine.label(), self.state.engine.other().label()))
                        .on_click(cx.listener(|this, _, window, cx| this.switch_engine(window, cx))))),
            )
            .children(self.notice_line(cx))
            .children(self.comment_strip(cx))
            .child(div().relative().flex_1().min_h_0().w_full().flex().flex_col().child(
                div().id("browser-page-focus").track_focus(&self.page_focus).tab_index(3)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| this.typed_on_proxy(event)))
                    .relative().flex_1().min_h_0().w_full().border_2()
                    .border_color(cx.theme().background) // owner: no blue outline on the focused page
                    .child(canvas(
                        |_, _, _| {},
                        move |bounds, _, window, cx| {
                            let _ = weak.update(cx, |panel, cx| {
                                let bounds = bounds.intersect(&window.content_mask().bounds);
                                panel.painted(bounds, window.scale_factor(), window, cx);
                                let runtime = panel.runtime.clone();
                                // G4: a module tab being dragged needs its drop zones and ghost
                                // visible, so the page hides for the length of the drag.
                                let drag_panel = weak.clone();
                                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                    if phase == DispatchPhase::Capture {
                                        sync_drag(&drag_panel, cx.has_active_drag(), cx);
                                        // A tile divider held: it follows the pointer.
                                        let _ = drag_panel.update(cx, |this, cx| if this.divider_drag.is_some() && event.pressed_button == Some(MouseButton::Left) {
                                            this.drag_divider(event.position, cx);
                                        } else { this.end_divider_drag(cx); });
                                    }
                                });
                                let drag_panel = weak.clone();
                                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                                    if phase == DispatchPhase::Capture {
                                        sync_drag(&drag_panel, false, cx);
                                        let _ = drag_panel.update(cx, |this, cx| this.end_divider_drag(cx));
                                    }
                                });
                                // Runs before a GPUI input receives a pointer click.
                                window.on_mouse_event(move |event: &MouseDownEvent, phase, _, _| {
                                    if phase == DispatchPhase::Capture && !bounds.contains(&event.position) {
                                        if let Some(runtime) = &runtime { runtime.focus_address(); }
                                    }
                                });
                            });
                        },
                    ).size_full())
                    .when(!self.ready && error.is_none(), |el| el.child(
                        div().absolute().top_4().left_4().text_color(cx.theme().muted_foreground)
                            .child("Starting browser…"),
                    ))
                    .when_some(error, |el, error| el.child(
                        div().absolute().top_4().left_4().right_4().text_color(cx.theme().foreground)
                            .child(error)
                            .child(Button::new("browser-retry").label("Retry").on_click(
                                cx.listener(|this, _, window, cx| { this.restarted = false; this.start(window, cx); cx.notify(); }),
                            )),
                    )),
            ).children(dividers))
    }
}

/// A mouse-up ends the drag even though GPUI clears its drag state only after the event.
fn sync_drag(panel: &WeakEntity<BrowserPanel>, dragging: bool, cx: &mut App) {
    let _ = panel.update(cx, |this, cx| {
        if this.dragging != dragging {
            this.dragging = dragging;
            this.sync_visibility(cx);
        }
    });
}
