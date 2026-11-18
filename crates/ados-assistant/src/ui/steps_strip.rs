use super::Assistant;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    ActiveTheme, Sizable,
};
use gpui_kit::*;

pub(super) struct Step {
    pub id: String,
    pub summary: String,
    pub done: Option<bool>,
}
impl Assistant {
    pub(super) fn tool_start(&mut self, id: String, name: String, summary: String) {
        let summary = format!("{name} · {}", summary.lines().next().unwrap_or_default());
        if let Some(step) = self.steps.iter_mut().find(|s| s.id == id) {
            step.summary = summary;
            step.done = None;
        } else {
            self.steps.push(Step {
                id,
                summary,
                done: None,
            });
        }
    }
    pub(super) fn tool_title(&mut self, id: &str, title: &str) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.id == id) {
            let kind = step.summary.split(" · ").next().unwrap_or_default().to_owned();
            step.summary = format!("{kind} · {}", title.lines().next().unwrap_or_default());
        }
    }
    pub(super) fn tool_end(&mut self, id: String, ok: bool, summary: String) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.id == id) {
            step.done = Some(ok);
            if !summary.is_empty() {
                step.summary = summary.lines().next().unwrap_or_default().to_owned();
            }
        } else {
            self.steps.push(Step {
                id,
                summary,
                done: Some(ok),
            });
        }
    }
    pub(super) fn steps_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut strip = div()
            .flex()
            .flex_col()
            .px_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .flex_shrink_0();
        if self.steps.is_empty() {
            return strip.into_any_element();
        }
        let active = self.steps.iter().filter(|s| s.done.is_none()).count();
        let summary = if active > 0 {
            format!("{active} running · {} steps", self.steps.len())
        } else {
            format!("{} steps", self.steps.len())
        };
        strip = strip.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(summary),
                )
                .child(
                    Button::new("minimize-steps")
                        .ghost()
                        .small()
                        .h(px(32.))
                        .label(if self.saved.minimized {
                            "Expand"
                        } else {
                            "Minimize"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.saved.minimized = !this.saved.minimized;
                            this.persist();
                            cx.notify();
                        })),
                ),
        );
        if !self.saved.minimized {
            strip = strip.child(
                div()
                    .id("tool-steps")
                    .max_h(px(112.))
                    .overflow_y_scroll()
                    .children(self.steps.iter().map(|step| {
                        let (glyph, color) = match step.done {
                            None => ("◌", cx.theme().primary),
                            Some(true) => ("✓", cx.theme().muted_foreground),
                            Some(false) => ("×", cx.theme().danger),
                        };
                        div()
                            .flex()
                            .gap_2()
                            .py_1()
                            .text_xs()
                            .child(div().text_color(color).child(glyph))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(step.summary.clone()),
                            )
                    })),
            );
        }
        strip.into_any_element()
    }
}
