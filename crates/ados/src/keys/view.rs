use super::{KeysEditor, Mode, Row};
use ados_core::{brand, shortcuts as sc};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex, input::Input, v_flex, ActiveTheme, Icon, IconName, Selectable,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

fn caps(chord: &str, cx: &App) -> impl IntoElement {
    h_flex().gap_1().children(sc::caps(chord).into_iter().map(|cap| {
        div().px_1p5().min_w(px(22.)).h(px(22.)).flex().items_center().justify_center()
            .rounded(px(6.)).border_1().border_color(cx.theme().border)
            .bg(cx.theme().secondary).text_xs().child(cap)
    }))
}

/// Hint-mode marks: the editor is a modal overlay, so only overlay marks count while it is open.
fn mark_overlay() -> impl IntoElement { ados_core::hints::mark_overlay() }
fn marked_overlay(control: impl IntoElement) -> Div {
    ados_core::hints::marked_in(ados_core::hints::Layer::Overlay, control)
}
fn note(text: String, color: Hsla) -> Div { div().text_xs().text_color(color).child(text) }

/// The risk of a chord in plain words (the core's wording is for developers).
fn warning_text(warning: sc::Warning) -> &'static str {
    match warning {
        sc::Warning::TextField => "Typing in a text field uses this key",
        sc::Warning::WindowManager => "The window manager usually takes this",
    }
}

impl KeysEditor {
    fn row(&mut self, row: &Row, titles: &[(String, String)], hit: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let index = row.index;
        let focus = self.row_focus(&row.spec.id, cx);
        let focused = focus.is_focused(window);
        let muted = cx.theme().muted_foreground;
        let red: Hsla = rgb(brand::RED).into();
        let name = |i: usize| titles.get(i).map(|(t, s)| format!("{t} ({s})")).unwrap_or_default();
        let recording = match &self.mode { Mode::Recording { index: i, chord, error } if *i == index => Some((chord.clone(), error.clone())), _ => None };
        let conflict = match &self.mode { Mode::Conflict { index: i, chord, other } if *i == index => Some((chord.clone(), *other)), _ => None };

        let mut keys = h_flex().gap_2().items_center().flex_shrink_0();
        if let Some((chord, _)) = &recording {
            keys = match chord {
                Some(c) => keys.child(caps(c, cx)),
                None => keys.child(div().text_sm().text_color(rgb(brand::BLUE)).child("Press keys…")),
            };
        } else if let Some((chord, _)) = &conflict {
            keys = keys.child(caps(chord, cx));
        } else if row.chords.is_empty() {
            keys = keys.child(div().text_sm().text_color(muted).child("Unbound"));
        } else {
            for (i, chord) in row.chords.iter().enumerate() {
                if i > 0 { keys = keys.child(div().text_xs().text_color(muted).child("or")); }
                keys = keys.child(caps(chord, cx));
            }
        }

        let mut details = v_flex().gap_1();
        if let Some((chord, error)) = &recording {
            let hint = if chord.is_some() { "Enter saves · Esc cancels · press another chord to change it" } else { "Esc cancels" };
            details = details.child(note(hint.into(), muted));
            if let Some(error) = error { details = details.child(note(error.clone(), red)); }
            if let Some(w) = chord.as_deref().and_then(|c| sc::warning(c, &row.spec.scope)) {
                details = details.child(note(warning_text(w).into(), muted));
            }
        } else if let Some((chord, other)) = &conflict {
            let (chord_s, chord_r, other) = (chord.clone(), chord.clone(), *other);
            details = details
                .child(note(format!("Already used by {}", name(other)), red))
                .child(h_flex().gap_2()
                    .child(Button::new("swap").label("Swap").primary().track_focus(&self.swap_focus).child(mark_overlay())
                        .tooltip("The other shortcut takes this one's old chord")
                        .on_click(cx.listener(move |e, _, w, cx| e.commit(index, &chord_s, Some((other, true)), w, cx))))
                    .child(Button::new("replace").label("Replace").ghost().child(mark_overlay())
                        .tooltip("Unbind the other shortcut")
                        .on_click(cx.listener(move |e, _, w, cx| e.commit(index, &chord_r, Some((other, false)), w, cx))))
                    .child(Button::new("cancel").label("Cancel").ghost().child(mark_overlay())
                        .on_click(cx.listener(|e, _, w, cx| e.cancel(w, cx)))));
        } else {
            for other in &row.conflicts_with {
                details = details.child(note(format!("Conflicts with {}", name(*other)), red));
            }
            for other in &row.overrides {
                details = details.child(note(format!("Intended override: takes {} here", name(*other)), muted));
            }
            for chord in &row.chords {
                if let Some(w) = sc::warning(chord, &row.spec.scope) {
                    details = details.child(note(format!("{}: {}", sc::label(chord), warning_text(w)), muted));
                }
            }
        }

        h_flex()
            .id(SharedString::from(format!("key-row-{}", row.spec.id)))
            .track_focus(&focus)
            .min_h(px(44.)).px_3().py_2().gap_3().items_center().rounded(px(8.))
            .border_1()
            .border_color(if focused || hit || recording.is_some() || conflict.is_some() { rgb(brand::BLUE).into() } else { transparent_black() })
            .when(focused, |el| el.bg(cx.theme().list_hover))
            .hover(|el| el.bg(cx.theme().list_hover))
            .cursor_pointer()
            .relative()
            .child(mark_overlay())
            .on_click(cx.listener(move |e, _, w, cx| if matches!(e.mode, Mode::Idle) { e.record(index, w, cx) }))
            .on_key_down(cx.listener(move |e, ev: &KeyDownEvent, w, cx| {
                if matches!(e.mode, Mode::Idle) && ev.keystroke.key == "enter" && !ev.keystroke.modifiers.modified() {
                    e.record(index, w, cx);
                    cx.stop_propagation();
                }
            }))
            .child(v_flex().flex_1().min_w_0().gap_1()
                .child(h_flex().gap_2().items_center()
                    .child(div().text_sm().truncate().child(row.spec.title.clone()))
                    .when(row.changed, |el| el.child(div().id(SharedString::from(format!("changed-{}", row.spec.id)))
                        .size(px(8.)).rounded_full().bg(rgb(brand::BLUE))
                        .tooltip(|w, cx| gpui_kit::component::tooltip::Tooltip::new("Changed from default").build(w, cx)))))
                .child(details))
            .child(keys)
            .child(div().w(px(32.)).flex_shrink_0().when(row.changed, |el| el.child(marked_overlay(
                Button::new(SharedString::from(format!("reset-{}", row.spec.id))).icon(IconName::Undo2).ghost()
                    .tooltip("Reset to default")
                    .on_click(cx.listener(move |e, _, w, cx| {
                        cx.stop_propagation();
                        e.cancel(w, cx);
                        e.reset(index, cx);
                    }))))))
            .into_any_element()
    }
}

impl Render for KeysEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        let specs = sc::specs(cx);
        let titles: Vec<(String, String)> = specs.iter().map(|s| (s.title.clone(), sc::scope_title(&s.scope))).collect();
        let conflicts = sc::conflicts(cx).len();
        let changed = !sc::user(cx).is_empty();
        let mut list = v_flex().gap_1();
        let mut group = String::new();
        // While the search box has the keyboard and holds a query, Enter acts on the first
        // match: ring it so that is visible.
        let hit = (self.search.focus_handle(cx).is_focused(window) && !self.search.read(cx).value().trim().is_empty())
            .then(|| rows.first().map(|r| r.index)).flatten();
        for row in &rows {
            let scope = sc::scope_title(&row.spec.scope);
            if scope != group {
                list = list.child(div().pt_3().pb_1().px_3().text_xs().text_color(cx.theme().muted_foreground).child(scope.clone()));
                group = scope;
            }
            list = list.child(self.row(row, &titles, hit == Some(row.index), window, cx));
        }
        if rows.is_empty() {
            list = list.child(div().p_6().text_sm().text_color(cx.theme().muted_foreground).child("No shortcuts match."));
        }
        let viewport = window.viewport_size();
        let path = sc::file_path().display().to_string();
        div()
            .absolute().inset_0().flex().items_center().justify_center()
            .bg(cx.theme().background.opacity(0.9))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                v_flex()
                    .id("shortcuts-editor")
                    .key_context("ShortcutsEditor")
                    .track_focus(&self.focus)
                    .w((viewport.width - px(48.)).min(px(720.)))
                    .h((viewport.height - px(48.)).min(px(720.)))
                    .rounded(px(10.)).border_1().border_color(cx.theme().border)
                    .bg(cx.theme().popover).shadow_md()
                    // Escape closes and Down/Enter leave the search box for the list, even
                        // while the search box (focused on open) has the keyboard.
                    .capture_key_down(cx.listener(|e, ev: &KeyDownEvent, w, cx| {
                        if ev.keystroke.modifiers.modified() || !matches!(e.mode, Mode::Idle) { return; }
                        let in_search = e.search.focus_handle(cx).is_focused(w);
                        match ev.keystroke.key.as_str() {
                            "escape" => { e.close(cx); cx.stop_propagation(); }
                            "down" if in_search => { e.step(1, w, cx); cx.stop_propagation(); }
                            // Enter in the search box acts on the first match, like Enter on its row.
                            "enter" if in_search => {
                                if let Some(first) = e.rows(cx).first().map(|r| r.index) {
                                    e.step(1, w, cx);
                                    e.record(first, w, cx);
                                }
                                cx.stop_propagation();
                            }
                            _ => {}
                        }
                    }))
                    .on_key_down(cx.listener(|e, ev: &KeyDownEvent, w, cx| {
                        let plain = !ev.keystroke.modifiers.modified();
                        match ev.keystroke.key.as_str() {
                            "escape" if plain => {
                                if matches!(e.mode, Mode::Idle) { e.close(cx) } else { e.cancel(w, cx) }
                                cx.stop_propagation();
                            }
                            "down" if plain && matches!(e.mode, Mode::Idle) => { e.step(1, w, cx); cx.stop_propagation(); }
                            "up" if plain && matches!(e.mode, Mode::Idle) => { e.step(-1, w, cx); cx.stop_propagation(); }
                            // Typing on a row goes into the search box, never nowhere.
                            _ if matches!(e.mode, Mode::Idle) && !e.search.focus_handle(cx).is_focused(w) && !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.alt && !ev.keystroke.modifiers.platform => {
                                let Some(text) = ev.keystroke.key_char.clone().filter(|c| !c.trim().is_empty() && !c.chars().any(char::is_control)) else { return };
                                e.search.update(cx, |input, cx| {
                                    input.focus(w, cx);
                                    input.insert(text, w, cx);
                                });
                                cx.stop_propagation();
                            }
                            _ => {}
                        }
                    }))
                    .child(h_flex().h(px(56.)).px_4().gap_2().items_center().flex_shrink_0()
                        .border_b_1().border_color(cx.theme().border)
                        .child(Icon::new(IconName::Search).text_color(cx.theme().muted_foreground))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Shortcuts"))
                        .child(div().flex_1())
                        .when(conflicts > 0, |el| el.child(
                            Button::new("conflicts")
                                .label(format!("{conflicts} conflict{}", if conflicts == 1 { "" } else { "s" }))
                                .ghost().selected(self.only_conflicts).child(mark_overlay())
                                .text_color(rgb(brand::RED))
                                .tooltip(if self.only_conflicts { "Show all shortcuts" } else { "Show only conflicts" })
                                .on_click(cx.listener(|e, _, _, cx| { e.only_conflicts = !e.only_conflicts; cx.notify(); }))))
                        .when(changed, |el| el.child(Button::new("reset-all").label("Reset all").ghost().child(mark_overlay())
                            .tooltip("Return every shortcut to its default")
                            .on_click(cx.listener(|e, _, w, cx| { e.cancel(w, cx); e.reset_all(cx) }))))
                        .child(marked_overlay(Button::new("close-shortcuts").icon(IconName::Close).ghost()
                            .tooltip("Close · Esc")
                            .on_click(cx.listener(|e, _, _, cx| e.close(cx))))))
                    .child(div().px_4().pt_3().flex_shrink_0().relative()
                        .child(Input::new(&self.search).prefix(Icon::new(IconName::Search)))
                        .child(mark_overlay()))
                    .when_some(self.error.clone(), |el, error| el.child(
                        div().px_4().pt_2().text_sm().text_color(rgb(brand::RED)).child(error)))
                    .child(div().id("shortcuts-list").flex_1().min_h_0().overflow_y_scroll().px_2().pb_2().child(list))
                    .child(h_flex().px_4().h(px(36.)).gap_1().flex_shrink_0().items_center()
                        .border_t_1().border_color(cx.theme().border)
                        .text_xs().text_color(cx.theme().muted_foreground)
                        .child(div().flex_shrink_0().whitespace_nowrap().child("Enter or click to change · ↑↓ move · Saved to"))
                        // A long path loses its start, never its file name, and stays inside the dialog.
                        .child(div().id("shortcuts-path").flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis_start()
                            .child(path.clone())
                            .tooltip(move |w, cx| gpui_kit::component::tooltip::Tooltip::new(path.clone()).build(w, cx)))),
            )
    }
}
