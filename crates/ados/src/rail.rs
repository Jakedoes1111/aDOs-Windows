use crate::{actions, modules, shell::Shell};
use gpui_kit::component::{button::{Button, ButtonVariants}, *};
use gpui_kit::prelude::*;
use gpui_kit::*;

impl Shell {
    pub(crate) fn rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(crate::settings_ui::zoomed(72., cx))
            .flex_shrink_0()
            .h_full()
            .items_center()
            .py_3()
            .gap_3()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(div().text_color(rgb(ados_core::brand::BLUE)).child("aD"))
                    .child(div().text_color(rgb(ados_core::brand::RED)).child("Os")),
            )
            .child(
                v_flex().id("rail-modules").flex_1().min_h_0().w_full().items_center().gap_3().overflow_y_scroll()
                .children(modules::MODULES.iter().map(|m| {
                let id = m.id.to_string();
                ados_core::hints::marked(Button::new(m.id)
                    .icon(m.icon.clone())
                    .ghost()
                    .selected(self.active == m.id)
                    .tooltip(ados_core::shortcuts::tooltip(m.title, &format!("shell.focus.{}", m.id), cx))
                    .on_click(cx.listener(move |s, _, w, cx| s.focus_module(&id, w, cx))))
            }))
                .child(ados_core::hints::marked(
                Button::new("assistant-rail")
                    .icon(IconName::Bot)
                    .ghost()
                    .tooltip(ados_core::shortcuts::tooltip("Assistant", "assistant.toggle", cx))
                    .on_click(|_, w, cx| w.dispatch_action(Box::new(actions::ToggleAssistant), cx)),
            ))
            )
            .child(ados_core::hints::marked(
                Button::new("command")
                    .icon(IconName::Search)
                    .ghost()
                    .tooltip(ados_core::shortcuts::tooltip("Command palette", "shell.palette", cx))
                    .on_click(|_, w, cx| w.dispatch_action(Box::new(actions::OpenPalette), cx)),
            ))
            .child(ados_core::hints::marked(
                Button::new("settings")
                    .icon(IconName::Settings)
                    .ghost()
                    .tooltip(ados_core::shortcuts::tooltip("Settings", "shell.settings", cx))
                    .on_click(|_, w, cx| w.dispatch_action(Box::new(crate::settings_ui::OpenSettings), cx)),
            ))
    }
}
