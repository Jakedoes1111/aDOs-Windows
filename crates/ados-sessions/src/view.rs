//! The right side: the open session's transcript (text, collapsed thinking, tool steps),
//! its permission questions and the composer; or the new-session backend picker.
use crate::backends::Conn;
use crate::model::{self, Item};
use crate::panel::SessionsPanel;
use ados_agents::ToolStatus;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{IndentInline, Textarea},
    ActiveTheme, Disableable, IconName, Sizable,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

impl Render for SessionsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.started || !self.unlisted_visible().is_empty() {
            // A backend starts (to list) when its rows are first drawn, not when the app starts.
            cx.defer_in(window, |this, window, cx| this.start_all(window, cx));
        }
        if !self.scroll_restored && self.started && self.slots.iter().all(|s| !s.listing) {
            self.scroll_restored = true;
            self.list_scroll.set_offset(point(px(0.), px(self.saved.scroll)));
        }
        let shown = self.list_shown();
        if shown && !self.list_was_shown && self.scroll_restored {
            self.list_scroll.set_offset(point(px(0.), px(self.saved.scroll)));
        }
        self.list_was_shown = shown;
        let overlay = shown && self.narrow();
        let list = shown.then(|| self.list_view(overlay, window, cx));
        let (side, over) = if overlay { (None, list) } else { (list, None) };
        let right = self.right_view(!shown, window, cx);
        let width = self.width.clone();
        let view = cx.weak_entity();
        // Measure the panel: below NARROW the list collapses by itself. A changed width redraws once.
        let measure = canvas(
            move |bounds, window, cx| {
                let w = f32::from(bounds.size.width);
                if (width.get() - w).abs() > 0.5 {
                    width.set(w);
                    window.defer(cx, move |_, cx| {
                        let _ = view.update(cx, |_, cx| cx.notify());
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div()
            .id("sessions-panel")
            .relative()
            .key_context("SessionsPanel")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &crate::panel::FocusFilter, window, cx| {
                if this.list_shown() {
                    this.filter.update(cx, |input, cx| input.focus(window, cx));
                } else {
                    // The filter is not drawn yet: show the list, then focus it next frame.
                    this.set_list_shown(true, window, cx);
                    cx.defer_in(window, |this, window, cx| this.filter.update(cx, |input, cx| input.focus(window, cx)));
                }
            }))
            .on_action(cx.listener(|this, _: &crate::panel::ToggleList, window, cx| this.toggle_list(window, cx)))
            .on_action(cx.listener(|this, _: &crate::panel::NewSession, window, cx| this.show_picker(window, cx)))
            .size_full()
            .flex()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(measure)
            .children(side)
            .child(right)
            .children(over)
    }
}

impl SessionsPanel {
    /// Keys while the backend picker shows. Returns true when the key was the picker's.
    fn picker_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let count = self.slots.len();
        match key {
            "left" | "up" => self.pick_cursor = self.pick_cursor.saturating_sub(1),
            "right" | "down" | "tab" if count > 0 => self.pick_cursor = (self.pick_cursor + 1).min(count - 1),
            "enter" if self.pick_cursor < count => self.new_session(self.pick_cursor, window, cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    /// Tab in the composer while the open session asks a question: go to its first
    /// option (the card's "Tab to choose"), not to Stop further down. Tab reaches the
    /// composer as its `IndentInline` action, which runs before any key listener.
    fn tab_to_question(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let key = self.saved.selected.clone().unwrap_or_default();
        if !self.has_question(&key) {
            return false;
        }
        if !self.composer.focus_handle(cx).is_focused(window) {
            return false;
        }
        self.question_focus.focus(window, cx);
        cx.notify();
        true
    }

    fn show_list_button(&self, cx: &mut Context<Self>) -> AnyElement {
        ados_core::hints::marked(
            Button::new("sessions-show-list")
                .icon(IconName::PanelLeftOpen)
                .ghost()
                .size(px(32.))
                .flex_shrink_0()
                .tooltip("Show sessions list (Ctrl+B)")
                .on_click(cx.listener(|this, _, window, cx| this.set_list_shown(true, window, cx))),
        )
        .into_any_element()
    }

    fn right_view(&mut self, list_hidden: bool, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let toggle = list_hidden.then(|| self.show_list_button(cx));
        let bar = |toggle: Option<AnyElement>| div().flex().items_center().p_2().flex_shrink_0().children(toggle);
        let body = if self.picking {
            div().size_full().flex().flex_col().child(bar(toggle)).child(div().flex_1().min_h_0().child(self.picker_view(cx))).into_any_element()
        } else if let Some(key) = self.saved.selected.clone() {
            self.session_view(&key, toggle, cx)
        } else {
            div().size_full().flex().flex_col().child(bar(toggle)).child(div().flex_1().min_h_0().child(self.empty_view(cx))).into_any_element()
        };
        div()
            .id("sessions-right")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .tab_group()
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if this.tab_to_question(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.picking && this.picker_key(&event.keystroke.key, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key != "escape" {
                    return;
                }
                let key = this.saved.selected.clone().unwrap_or_default();
                if this.has_question(&key) {
                    this.answer(0, None, window, cx);
                } else if this.running.contains_key(&key) {
                    this.cancel(cx);
                } else if this.picking {
                    this.picking = false;
                    this.list_focus.focus(window, cx);
                    cx.notify();
                } else {
                    return;
                }
                cx.stop_propagation();
            }))
            .child(body)
            .into_any_element()
    }

    fn empty_view(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .text_color(cx.theme().muted_foreground)
            .child("Open a session from the list, or start a new one.")
            .child(
                Button::new("sessions-empty-new")
                    .icon(IconName::Plus)
                    .label("New session")
                    .child(ados_core::hints::mark())
                    .h(px(32.))
                    .on_click(cx.listener(|this, _, window, cx| this.show_picker(window, cx))),
            )
            .into_any_element()
    }

    fn picker_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut buttons = div().flex().flex_wrap().gap_2();
        for (ix, slot) in self.slots.iter().enumerate() {
            let label = match &slot.conn {
                Conn::Starting if self.creating == Some(ix) => format!("{} (starting…)", slot.config.name),
                _ => slot.config.name.clone(),
            };
            buttons = buttons.child(
                Button::new(("sessions-pick", ix))
                    .label(label)
                    .child(ados_core::hints::mark())
                    .h(px(32.))
                    .when(ix == self.pick_cursor, |b| b.primary())
                    .on_click(cx.listener(move |this, _, window, cx| this.new_session(ix, window, cx))),
            );
        }
        div()
            .track_focus(&self.picker_focus)
            .size_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_lg().child("New session"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Pick a backend. The session starts in your home folder. Arrows move, Enter picks, Esc closes."),
            )
            .child(buttons)
            .children(self.error.clone().map(|e| div().text_sm().text_color(cx.theme().danger).child(e)))
            .into_any_element()
    }

    fn session_view(&self, key: &str, toggle: Option<AnyElement>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, danger, primary) = (theme.muted_foreground, theme.border, theme.danger, theme.primary);
        let backend_id = model::split_key(key).map(|(b, _)| b.to_string()).unwrap_or_default();
        let backend = self.backend_name(&backend_id);
        let transcript = self.transcripts.get(key);
        let running = self.running.contains_key(key);
        let folder = self
            .saved
            .selected_cwd
            .as_ref()
            .map(|p| model::folder_label(p, &crate::backends::home()))
            .unwrap_or_default();
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .h(px(48.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(border)
            .children(toggle)
            .child(div().flex_1().min_w_0().truncate().child(self.selected_title()))
            .when(running, |h| h.child(div().text_xs().text_color(primary).child("Running")))
            .child(div().text_xs().text_color(muted).px_1().rounded(px(4.)).border_1().border_color(border).child(backend.clone()))
            .child(div().text_xs().text_color(muted).child(folder));

        let mut items = div()
            .id("session-transcript")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.transcript_scroll)
            .px_4()
            .py_3()
            .flex()
            .flex_col()
            .gap_3()
            .text_sm();
        if let Some(t) = transcript {
            for (ix, item) in t.items.iter().enumerate() {
                items = items.child(self.item_view(key, ix, item, &backend, cx));
            }
            if t.replaying {
                items = items.child(div().text_color(muted).child("Opening session…"));
            }
            if let Some(error) = &t.error {
                items = items.child(div().text_color(danger).whitespace_normal().child(error.clone()));
            }
            if !t.replaying && t.items.is_empty() && t.error.is_none() {
                items = items.child(div().text_color(muted).child("Nothing here yet. Write below to start."));
            }
        }
        let live = transcript.is_some_and(|t| t.live);
        let has_text = !self.composer.read(cx).value().trim().is_empty();
        let composer = div()
            .p_2()
            .flex_shrink_0()
            .border_t_1()
            .border_color(border)
            .child(div().relative().child(Textarea::new(&self.composer).w_full()).child(ados_core::hints::mark()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt_1()
                    .gap_2()
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(muted).child(if self.has_question(key) {
                        "Answer the question above · Esc denies".to_string()
                    } else if running {
                        "Esc stops the reply".to_string()
                    } else if !live {
                        format!("Waiting for {backend}…")
                    } else {
                        String::new()
                    }))
                    .child(
                        Button::new("session-send")
                            .small()
                            .h(px(32.))
                            .min_w(px(56.))
                            .label(if running { "Stop" } else { "Send" })
                            .child(ados_core::hints::mark())
                            .when(!running, |b| b.primary())
                            .disabled(!(running || (live && has_text)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let key = this.saved.selected.clone().unwrap_or_default();
                                if this.running.contains_key(&key) {
                                    this.cancel(cx);
                                } else {
                                    this.send(window, cx);
                                }
                            })),
                    ),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(items)
            .children(self.questions_view(key, cx))
            .child(composer)
            .into_any_element()
    }

    fn item_view(&self, key: &str, ix: usize, item: &Item, backend: &str, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let speaker = |who: String, text: &str| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(muted).child(who))
                .child(ados_core::links::linked_text(&format!("session-{key}-{ix}"), text, cx))
                .into_any_element()
        };
        match item {
            Item::User(text) => speaker("You".into(), text),
            Item::Agent(text) => speaker(backend.to_string(), text),
            Item::Thought(text) => {
                let open = self.expanded.contains(&(key.to_string(), ix));
                let toggle_key = (key.to_string(), ix);
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_1()
                    .child(
                        Button::new(("session-thought", ix))
                            .ghost()
                            .small()
                            .h(px(32.))
                            .icon(if open { IconName::ChevronDown } else { IconName::ChevronRight })
                            .label("Thinking")
                            .child(ados_core::hints::mark())
                            .tooltip(if open { "Hide thinking" } else { "Show thinking" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded.remove(&toggle_key) {
                                    this.expanded.insert(toggle_key.clone());
                                }
                                cx.notify();
                            })),
                    )
                    .when(open, |d| d.child(div().pl_3().text_color(muted).whitespace_normal().child(text.trim().to_string())))
                    .into_any_element()
            }
            Item::Tool { kind, title, status, .. } => {
                let (icon, color) = match status {
                    ToolStatus::Completed => (IconName::Check, muted),
                    ToolStatus::Failed => (IconName::Close, cx.theme().danger),
                    _ => (IconName::Loader, cx.theme().primary),
                };
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(muted)
                    .child(gpui_kit::component::Icon::new(icon).small().text_color(color))
                    .child(div().flex_1().min_w_0().truncate().child(format!("{kind} · {title}")))
                    .child(div().text_xs().child(model::status_word(*status)))
                    .into_any_element()
            }
        }
    }
}
