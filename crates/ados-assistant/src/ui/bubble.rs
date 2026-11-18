use super::assistant::{Assistant, Drag};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    ActiveTheme,
};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder;
use std::time::Duration;

impl Assistant {
    pub(super) fn drag_events(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.weak_entity();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let view = weak.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        if phase != DispatchPhase::Capture {
                            return;
                        }
                        let mut target = None;
                        if let Some(drag) = &mut this.drag {
                            let dx = f32::from(event.position.x - drag.start.x);
                            let dy = f32::from(event.position.y - drag.start.y);
                            if dx * dx + dy * dy >= 16. {
                                drag.moved = true;
                            }
                            if drag.moved {
                                target = Some(drag.origin.0 + dx);
                            }
                        }
                        if let Some(x) = target {
                            this.set_bubble_x(x, window);
                            cx.notify();
                        }
                    });
                });
                let view = weak.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                            return;
                        }
                        if let Some(drag) = this.drag.take() {
                            if !drag.moved && drag.bubble {
                                this.toggle(window, cx);
                            }
                            this.persist();
                            cx.notify();
                        }
                    });
                });
            },
        )
        .absolute()
        .size_full()
        .into_any_element()
    }
    pub(super) fn start_drag(&mut self, event: &MouseDownEvent, bubble: bool, origin: (f32, f32)) {
        self.drag = Some(Drag {
            start: event.position,
            origin,
            moved: false,
            bubble,
        });
    }
    /// Left/Right move the bubble (and the card under it) along the top bar.
    pub(super) fn move_with_keys(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dx = match event.keystroke.key.as_str() {
            "left" => -10.,
            "right" => 10.,
            _ => return,
        };
        let (x, _) = self.bubble_position(window, cx);
        self.set_bubble_x(x + dx, window);
        self.persist();
        cx.stop_propagation();
        cx.notify();
    }
    pub(super) fn bubble(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let color = if !matches!(self.conn, super::agent::Conn::Failed(_)) {
            cx.theme().primary
        } else {
            cx.theme().muted_foreground
        };
        let ring = div()
            .flex_shrink_0()
            .size(px(24.))
            .rounded_full()
            .border_2()
            .border_color(color)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(if self.ready { 8. } else { 4. }))
                    .rounded_full()
                    .bg(color),
            );
        let ring = if self.running {
            ring.with_animation(
                "assistant-ring",
                Animation::new(Duration::from_millis(1200)).repeat(),
                move |el, delta| {
                    el.border_color(
                        color.opacity(0.3 + (delta * std::f32::consts::TAU).sin().abs() * 0.7),
                    )
                },
            )
            .into_any_element()
        } else {
            ring.into_any_element()
        };
        let (x, y) = self.bubble_position(window, cx);
        let draft = self.has_draft();
        let tooltip = if draft {
            format!("Ask {} · Alt+Space · Draft kept", self.backend_name())
        } else {
            format!("Ask {} · Alt+Space", self.backend_name())
        };
        let content = div()
            .relative()
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .child(ring)
            .when(draft, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(0.))
                        .right(px(0.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(cx.theme().muted_foreground),
                )
            });
        div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .size(px(32.))
            .child(ados_core::hints::mark_in(ados_core::hints::Layer::Float))
            .child(
                Button::new("assistant-bubble")
                    .ghost()
                    .w(px(32.))
                    .h(px(32.))
                    .p_0()
                    .tooltip(tooltip)
                    .child(content)
                    .on_key_down(cx.listener(Self::move_with_keys))
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        if event.is_keyboard() {
                            this.toggle(window, cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev, window, cx| {
                            let origin = this.bubble_position(window, cx);
                            this.start_drag(ev, true, origin);
                            cx.stop_propagation();
                        }),
                    ),
            )
            .into_any_element()
    }
}
