use super::Assistant;
use crate::ContextItem;
use ados_core::hub::{ContextHub, Snapshot};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    ActiveTheme, Sizable,
};
use gpui_kit::*;

pub(super) fn capped(text: &str) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(60_000).collect();
    if chars.next().is_some() {
        result.push_str("\n[truncated]");
    }
    result
}
/// A context chip whose label ellipsises inside the available width while the
/// close mark stays visible at the end.
fn chip(id: impl Into<ElementId>, label: String, tooltip: String) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .h(px(32.))
        .max_w_full()
        .min_w_0()
        // Button defaults to flex-shrink 0; let the chip give way so the row stays one line.
        .flex_shrink(1.)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .w_full()
                .min_w_0()
                .gap_1()
                .child(div().flex_1().min_w_0().truncate().child(label))
                .child(div().flex_shrink_0().child("×")),
        )
        .child(super::header::float_mark())
        .tooltip(tooltip)
}

fn item(snapshot: &Snapshot, selection: bool) -> ContextItem {
    ContextItem {
        label: format!(
            "{}: {} {}",
            if selection { "selection" } else { "module" },
            snapshot.title,
            snapshot.location.as_deref().unwrap_or_default()
        )
        .trim_end()
        .to_owned(),
        text: capped(if selection {
            snapshot.selection.as_deref().unwrap_or_default()
        } else {
            &snapshot.text
        }),
    }
}
impl Assistant {
    pub(super) fn context(&self, cx: &App) -> Vec<ContextItem> {
        let mut result = Vec::new();
        let active = ContextHub::active(cx);
        if let Some(active) = &active {
            if self.saved.this_module {
                result.push(item(active, false));
            }
            if let Some(selection) = &active.selection {
                if !selection.is_empty() && self.dismissed_selection.as_deref() != Some(selection) {
                    result.push(item(active, true));
                }
            }
        }
        for module in &self.attached {
            if self.saved.this_module && active.as_ref().is_some_and(|s| &s.module == module) {
                continue;
            }
            if let Some(snapshot) = ContextHub::get(module, cx) {
                result.push(item(&snapshot, false));
            }
        }
        result
    }
    pub(super) fn context_chips(&self, cx: &mut Context<Self>) -> AnyElement {
        // One row, never wrapped: the card reserves height for a single chip row, so a
        // wrapped "@" pushed the composer and Send out of the card's bottom edge.
        let mut row = div()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .flex_shrink_0()
            .w_full()
            .min_w_0();
        if self.saved.this_module {
            let title = ContextHub::active(cx)
                .map(|s| s.title.to_string())
                .unwrap_or_else(|| "No active module".into());
            row = row.child(
                chip(
                    "this-module",
                    format!("This module · {title}"),
                    format!("This module · {title}\nRemove active module context"),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.saved.this_module = false;
                    this.persist();
                    this.input.update(cx, |input, cx| input.focus(window, cx));
                    cx.notify();
                })),
            );
        }
        if let Some(selection) = ContextHub::active(cx).and_then(|s| s.selection) {
            if !selection.is_empty() && self.dismissed_selection.as_deref() != Some(&selection) {
                row = row.child(
                    chip(
                        "selection-context",
                        "Selection".to_owned(),
                        "Remove selection context".to_owned(),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dismissed_selection = Some(selection.to_string());
                        this.input.update(cx, |input, cx| input.focus(window, cx));
                        cx.notify();
                    })),
                );
            }
        }
        for (index, module) in self.attached.iter().enumerate() {
            let title = ContextHub::get(module, cx)
                .map(|s| s.title.to_string())
                .unwrap_or_else(|| module.to_string());
            let module = module.clone();
            row = row.child(
                chip(
                    ("attached-context", index),
                    title.clone(),
                    format!("{title}\nRemove module context"),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.attached.retain(|m| m != &module);
                    this.input.update(cx, |input, cx| input.focus(window, cx));
                    cx.notify();
                })),
            );
        }
        row.child(
            Button::new("add-context")
                .ghost()
                .small()
                .h(px(32.))
                .min_w(px(32.))
                .label("@")
                .child(super::header::float_mark())
                .tooltip("Attach context")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.picker = !this.picker;
                    cx.notify();
                })),
        )
        .text_color(cx.theme().muted_foreground)
        .into_any_element()
    }
    pub(super) fn context_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div()
            .id("context-picker")
            .max_h(px(160.))
            .overflow_y_scroll()
            .p_2()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap_1();
        if !self.saved.this_module {
            list = list.child(
                Button::new("restore-this-module")
                    .ghost()
                    .small()
                    .h(px(32.))
                    .label("This module (follows focus)")
                    .child(super::header::float_mark())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.saved.this_module = true;
                        this.persist();
                        this.finish_pick(window, cx);
                    })),
            );
        }
        for (index, snapshot) in ContextHub::all(cx).iter().enumerate() {
            if self.attached.contains(&snapshot.module)
                || (self.saved.this_module
                    && ContextHub::active(cx).is_some_and(|s| s.module == snapshot.module))
            {
                continue;
            }
            let id = snapshot.module.clone();
            list = list.child(
                Button::new(("context-option", index))
                    .ghost()
                    .small()
                    .h(px(32.))
                    .label(snapshot.title.clone())
                    .child(super::header::float_mark())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.attached.push(id.clone());
                        this.finish_pick(window, cx);
                    })),
            );
        }
        list.into_any_element()
    }
    fn finish_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        if value.ends_with('@') {
            let value = value.trim_end_matches('@').to_owned();
            self.input
                .update(cx, |input, cx| input.set_value(value, window, cx));
            self.save_draft(cx);
        }
        self.picker = false;
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::capped;
    #[test]
    fn unicode_context_cap() {
        assert_eq!(capped(&"é".repeat(60_000)).chars().count(), 60_000);
        let text = capped(&"é".repeat(60_001));
        assert!(text.ends_with("[truncated]"));
        assert_eq!(text.matches('é').count(), 60_000);
    }
}
