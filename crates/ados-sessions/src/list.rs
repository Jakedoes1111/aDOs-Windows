//! The left side: filter box, backend filter, and one list of every backend's sessions
//! (newest first), with a row per backend that is starting or failed (Retry) and a
//! "More" row per backend with another `session/list` page.
use crate::backends::{home, Conn};
use crate::model::{self, Entry};
use crate::panel::SessionsPanel;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, MoveDown, MoveUp},
    ActiveTheme, IconName, Selectable, Sizable,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

pub(crate) enum ListRow {
    /// The backend is starting or listing its first page.
    Status { text: String },
    /// The backend failed: one short line naming it (full sentence in the tooltip), with Retry.
    Problem { ix: usize, line: String, text: String },
    Session(Entry),
    More { ix: usize },
}

impl SessionsPanel {
    pub(crate) fn rows(&self) -> Vec<ListRow> {
        let only = self.saved.backend.as_deref();
        let shown = |id: &str| only.is_none_or(|b| b == id);
        let mut rows = vec![];
        // Backends that failed go below the sessions, one short line each, so a
        // logged-out backend never pushes real sessions out of view.
        let mut problems = vec![];
        let query = self.saved.filter.trim();
        let mut problem = |ix: usize, name: &str, raw: &str| {
            let line = model::problem_line(name, raw);
            if model::problem_matches(&line, query) {
                problems.push(ListRow::Problem { ix, line, text: raw.to_string() });
            }
        };
        for (ix, slot) in self.slots.iter().enumerate() {
            if !shown(&slot.config.id) {
                continue;
            }
            let name = &slot.config.name;
            match &slot.conn {
                Conn::Failed(text) => problem(ix, name, text),
                Conn::Starting => rows.push(ListRow::Status { text: format!("Starting {name}…") }),
                Conn::Ready(_) if slot.listing && slot.entries.is_empty() => {
                    rows.push(ListRow::Status { text: format!("Listing {name} sessions…") })
                }
                _ => {}
            }
            if let Some(text) = &slot.list_error {
                problem(ix, name, text);
            }
        }
        let mut entries: Vec<Entry> = self
            .slots
            .iter()
            .flat_map(|s| s.entries.iter())
            .filter(|e| model::matches(e, &self.saved.filter, only, &self.backend_name(&e.backend)))
            .cloned()
            .collect();
        model::sort_newest(&mut entries);
        rows.extend(entries.into_iter().map(ListRow::Session));
        for (ix, slot) in self.slots.iter().enumerate() {
            if shown(&slot.config.id) && slot.cursor.is_some() {
                rows.push(ListRow::More { ix });
            }
        }
        rows.extend(problems);
        rows
    }

    /// Enter or click on a row. Opening a session puts the keyboard in its composer.
    fn activate(&mut self, row: &ListRow, window: &mut Window, cx: &mut Context<Self>) {
        match row {
            ListRow::Session(entry) => {
                let (key, cwd, title) = (entry.key(), entry.cwd.clone(), model::label(entry));
                self.select(key, cwd, title, window, cx);
                self.composer.update(cx, |input, cx| input.focus(window, cx));
            }
            ListRow::Problem { ix, .. } => self.retry(*ix, window, cx),
            ListRow::More { ix } => self.list_page(*ix, window, cx),
            ListRow::Status { .. } => {}
        }
    }

    /// A character typed while the list has the keyboard: into the open session's composer
    /// when the highlight is on that session, otherwise into the filter. Never dropped.
    fn list_typed(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows();
        let on_open = matches!(rows.get(self.cursor), Some(ListRow::Session(e)) if self.saved.selected.as_deref() == Some(e.key().as_str()));
        if on_open && !self.picking {
            self.composer.update(cx, |input, cx| {
                input.focus(window, cx);
                input.insert(text.to_string(), window, cx);
            });
        } else {
            self.filter.update(cx, |input, cx| {
                input.focus(window, cx);
                input.insert(text.to_string(), window, cx);
            });
        }
        cx.notify();
    }

    pub(crate) fn list_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let rows = self.rows();
        match key {
            "down" => self.cursor = (self.cursor + 1).min(rows.len().saturating_sub(1)),
            "up" => self.cursor = self.cursor.saturating_sub(1),
            "home" => self.cursor = 0,
            "end" => self.cursor = rows.len().saturating_sub(1),
            "enter" => {
                if let Some(row) = rows.get(self.cursor) {
                    self.activate(row, window, cx);
                }
            }
            _ => return false,
        }
        self.list_scroll.scroll_to_item(self.cursor + 1); // +1: the backend filter row is item 0.
        cx.notify();
        true
    }

    /// Scrolled near the end: fetch every backend's next page.
    fn near_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (offset, max) = (self.list_scroll.offset().y, self.list_scroll.max_offset().y);
        if f32::from(-offset) + 120. >= f32::from(max) {
            for ix in 0..self.slots.len() {
                if self.slots[ix].cursor.is_some() {
                    self.list_page(ix, window, cx);
                }
            }
        }
    }

    pub(crate) fn list_view(&mut self, overlay: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.rows();
        self.cursor = self.cursor.min(rows.len().saturating_sub(1));
        let theme = cx.theme();
        let fg = theme.foreground;
        let (muted, border, active, hover, ring, primary, danger) = (
            theme.muted_foreground,
            theme.border,
            theme.list_active,
            theme.list_hover,
            theme.ring,
            theme.primary,
            theme.danger,
        );
        // The ring shows wherever ↑/↓ and Enter act on it: the list and the filter box.
        let list_focused = self.list_focus.is_focused(window) || self.filter.read(cx).focus_handle(cx).is_focused(window);
        let home = home();

        let mut chips = div().flex().flex_wrap().gap_1().px_2().pb_2();
        let all = self.saved.backend.is_none();
        chips = chips.child(
            ados_core::hints::marked(Button::new("sessions-backend-all")
                .ghost()
                .small()
                .h(px(32.))
                .selected(all)
                .label("All")
                .tooltip("Sessions from every backend")
                .on_click(cx.listener(|this, _, _, cx| this.set_backend_filter(None, cx)))),
        );
        for (ix, slot) in self.slots.iter().enumerate() {
            let id = slot.config.id.clone();
            chips = chips.child(
                ados_core::hints::marked(Button::new(("sessions-backend", ix))
                    .ghost()
                    .small()
                    .h(px(32.))
                    .selected(self.saved.backend.as_deref() == Some(id.as_str()))
                    .label(slot.config.name.clone())
                    .tooltip(format!("Only {} sessions", slot.config.name))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_backend_filter(Some(id.clone()), cx)))),
            );
        }

        let mut list = div()
            .id("sessions-list")
            .key_context("SessionsList")
            .track_focus(&self.list_focus)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.list_scroll)
            .p_1()
            .flex()
            .flex_col()
            .gap_1()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let k = &event.keystroke;
                if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
                    return;
                }
                if this.list_key(&k.key, window, cx) {
                    cx.stop_propagation();
                } else if let Some(text) = k.key_char.as_deref().filter(|t| !t.is_empty() && !t.chars().any(char::is_control)) {
                    this.list_typed(text, window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_scroll(cx.listener(|this, _, window, cx| {
                this.near_end(window, cx);
                this.persist_soon(cx);
            }))
            .child(chips);
        if rows.is_empty() {
            let text = if self.slots.is_empty() {
                "No backends in backends.toml."
            } else if self.saved.filter.trim().is_empty() {
                "No sessions yet."
            } else {
                "No sessions match the filter."
            };
            list = list.child(div().px_2().py_2().text_sm().text_color(muted).child(text));
        }
        for (row_ix, row) in rows.into_iter().enumerate() {
            let current = list_focused && row_ix == self.cursor;
            let base = div()
                .id(("session-row", row_ix))
                .flex_shrink_0()
                .min_h(px(36.))
                .px_2()
                .py_1()
                .rounded(px(8.))
                .border_1()
                .border_color(if !current { transparent_black() } else if matches!(&row, ListRow::Session(e) if self.saved.selected.as_deref() == Some(e.key().as_str())) { fg } else { ring })
                .text_sm();
            let element = match &row {
                ListRow::Session(entry) => {
                    let key = entry.key();
                    let selected = self.saved.selected.as_deref() == Some(key.as_str());
                    let running = self.running.contains_key(&key);
                    let asking = self.has_question(&key);
                    base.when(selected, |r| r.bg(active))
                        .hover(move |s| s.bg(hover))
                        .cursor_pointer()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().min_w_0().truncate().child(model::label(entry)))
                                .when(running || asking, |r| {
                                    r.child(
                                        div()
                                            .id(("session-dot", row_ix))
                                            .size(px(8.))
                                            .rounded_full()
                                            .flex_shrink_0()
                                            .bg(if asking { danger } else if selected { fg } else { primary })
                                            .tooltip(move |window, cx| {
                                                gpui_kit::component::tooltip::Tooltip::new(if asking {
                                                    "Waiting for your answer"
                                                } else {
                                                    "Running"
                                                })
                                                .build(window, cx)
                                            }),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .text_color(if selected { fg } else { muted })
                                .child(
                                    div()
                                        .px_1()
                                        .rounded(px(4.))
                                        .border_1()
                                        .border_color(if selected { fg.opacity(0.6) } else { border })
                                        .flex_shrink_0()
                                        .child(self.backend_name(&entry.backend)),
                                )
                                .child(div().flex_1().min_w_0().truncate().child(model::folder_label(&entry.cwd, &home)))
                                .children(entry.updated_at.as_deref().map(|t| div().flex_shrink_0().child(model::time_label(t)))),
                        )
                }
                ListRow::Status { text, .. } => base.flex().items_center().text_color(muted).child(text.clone()),
                ListRow::Problem { ix, line, text } => {
                    let ix = *ix;
                    let full = text.clone();
                    base.flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id(("sessions-problem", ix))
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(danger)
                                .child(line.clone())
                                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(full.clone()).build(window, cx)),
                        )
                        .child(
                            ados_core::hints::marked(Button::new(("sessions-retry", ix))
                                .ghost()
                                .icon(IconName::RefreshCw)
                                .size(px(32.))
                                .flex_shrink_0()
                                .tooltip(format!("Retry {}", self.slots[ix].config.name))
                                .on_click(cx.listener(move |this, _, window, cx| this.retry(ix, window, cx)))),
                        )
                }
                ListRow::More { ix } => {
                    let name = self.slots[*ix].config.name.clone();
                    let loading = self.slots[*ix].listing;
                    base.flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .text_color(muted)
                        .child(if loading { format!("Loading more {name} sessions…") } else { format!("More {name} sessions") })
                }
            };
            list = list.child(element.child(ados_core::hints::mark()).on_click(cx.listener(move |this, _, window, cx| {
                this.cursor = row_ix;
                this.list_focus.focus(window, cx);
                let rows = this.rows();
                if let Some(row) = rows.get(row_ix) {
                    if !matches!(row, ListRow::Problem { .. }) {
                        this.activate(row, window, cx);
                    }
                }
            })));
        }

        let width = self.width.get();
        div()
            .id("sessions-list-pane")
            .when(!overlay, |d| d.w(relative(0.38)).min_w(px(240.)).max_w(px(360.)))
            .when(overlay, |d| d.absolute().top_0().left_0().w(px(width.clamp(200., 360.))).occlude().shadow_md())
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            // Esc closes the list while it is open over the chat (narrow panel), filter or list.
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && this.overlay && this.narrow() {
                    this.set_list_shown(false, window, cx);
                    cx.stop_propagation();
                }
            }))
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .p_2()
                    .child(
                        div()
                            .flex_1()
                            .relative()
                            // ↑/↓ in the filter move the highlight (a one-line input ignores them).
                            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                                this.list_key("up", window, cx);
                                cx.stop_propagation();
                            }))
                            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                                this.list_key("down", window, cx);
                                cx.stop_propagation();
                            }))
                            .child(Input::new(&self.filter).prefix(IconName::Search).cleanable(true))
                            .child(ados_core::hints::mark()),
                    )
                    .child(
                        ados_core::hints::marked(Button::new("sessions-hide-list")
                            .icon(IconName::PanelLeftClose)
                            .ghost()
                            .size(px(32.))
                            .flex_shrink_0()
                            .tooltip("Hide sessions list (Ctrl+B)")
                            .on_click(cx.listener(|this, _, window, cx| this.set_list_shown(false, window, cx)))),
                    )
                    .child(
                        ados_core::hints::marked(Button::new("sessions-new")
                            .icon(IconName::Plus)
                            .ghost()
                            .size(px(32.))
                            .tooltip("New session")
                            .on_click(cx.listener(|this, _, window, cx| this.show_picker(window, cx)))),
                    ),
            )
            .child(list)
            .into_any_element()
    }

    fn set_backend_filter(&mut self, backend: Option<String>, cx: &mut Context<Self>) {
        self.saved.backend = backend;
        self.cursor = 0;
        self.persist();
        cx.notify();
    }
}
