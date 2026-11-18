use crate::{
    file,
    panel::{OpenLink, DeleteSelected, FocusSearch, NewNote, NotesPanel, SelectNext, SelectPrevious, TogglePreview, UndoDelete},
};
use ados_core::capture::CaptureKind;
use gpui_kit::component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
    input::{Editor, Input, MoveDown, MoveUp},
    text::TextView,
};
use gpui_kit::*;
use gpui_kit::{assets::IconName, prelude::FluentBuilder};

pub(crate) const READ_ONLY: &str = "Captures are read-only. Make note to edit.";

/// The one line shown when unsaved edits went to a conflict copy.
pub(crate) const CONFLICT: &str = "Changed outside aDOs: your edits are in a copy";

impl NotesPanel {
    /// One line, never more: the copy's full name is in the tooltip; × or Esc hides it.
    fn conflict_line(&self, name: String, cx: &mut Context<Self>) -> impl IntoElement {
        let tip: SharedString =
            format!("{CONFLICT}: “{name}”. The version changed outside is kept as it was.").into();
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .pl_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_sm()
            .child(div().size(px(6.)).flex_shrink_0().rounded_full().bg(rgb(ados_core::brand::RED)))
            .child(
                div()
                    .id("notes-conflict")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(cx.theme().foreground)
                    .child(CONFLICT)
                    // Wrapped: a copy's name is long, and one line ran off the window.
                    .tooltip(move |w, cx| {
                        let tip = tip.clone();
                        gpui_kit::component::tooltip::Tooltip::element(move |_, _| div().max_w(px(240.)).child(tip.clone()))
                            .build(w, cx)
                    }),
            )
            .child(ados_core::hints::marked(
                Button::new("notes-conflict-hide")
                    .icon(IconName::Close)
                    .ghost()
                    .size(px(32.))
                    .flex_shrink_0()
                    .tooltip("Hide · Esc")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.conflict = None;
                        cx.notify();
                    })),
            ))
    }
}

impl Render for NotesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let paths: std::collections::HashSet<_> = self.notes.iter().map(|n| n.path.clone())
            .chain(self.inbox.iter().map(|i| i.path.clone())).collect();
        self.row_focus.retain(|path, _| paths.contains(path));
        for path in paths {
            self.row_focus.entry(path).or_insert_with(|| cx.focus_handle());
        }
        let rows = self.visible_rows();
        if let Some(i) = rows.iter().position(|p| self.row_focus.get(p).is_some_and(|f| f.is_focused(window))) {
            self.last_row = Some((i, rows[i].clone()));
        }
        let is_capture = self.state.selected_capture.is_some();
        let has_item = is_capture || self.state.selected.is_some();
        let preview = is_capture || self.state.mode == "preview";
        let query = self.query.to_lowercase();
        let notes: Vec<_> = self
            .notes
            .iter()
            .filter(|n| n.search_text.contains(&query))
            .cloned()
            .collect();
        let inbox: Vec<_> = self
            .inbox
            .iter()
            .filter(|i| i.text.to_lowercase().contains(&query))
            .cloned()
            .collect();
        let no_matches = !query.is_empty() && notes.is_empty() && inbox.is_empty();
        let hit = self.search_hit(window, cx);
        let sidebar = div()
            .w(px(220.))
            .when(self.narrow, |list| list.w_full())
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                div().p_2().relative()
                    // ↑/↓ leave the search box for the list (a one-line input ignores them).
                    .capture_action(cx.listener(|this, _: &MoveDown, window, cx| { this.search_step(1, window, cx); cx.stop_propagation(); }))
                    .capture_action(cx.listener(|this, _: &MoveUp, window, cx| { this.search_step(-1, window, cx); cx.stop_propagation(); }))
                    .child(
                    Input::new(&self.search)
                        .prefix(IconName::Search)
                        .cleanable(true),
                ).child(ados_core::hints::mark()),
            )
            .child(
                div()
                    .id("notes-list")
                    .key_context("NotesList")
                    .track_focus(&self.list_focus)
                    // A press on blank list space is not a choice of row: keep the primary
                    // input the panel's capture handler just focused. Without this, the
                    // tracked focus (bubble phase, after capture) took the keyboard and typed
                    // text went nowhere. Rows still focus the list explicitly on open.
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_action(cx.listener(|this, _: &DeleteSelected, window, cx| this.delete_selected(window, cx)))
                    .on_action(cx.listener(|this, _: &SelectPrevious, window, cx| this.step_row(-1, window, cx)))
                    .on_action(cx.listener(|this, _: &SelectNext, window, cx| this.step_row(1, window, cx)))
                    // Typing on a read-only capture says how to edit it; on a note row it goes
                    // into the search box. Never nowhere.
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        let m = event.keystroke.modifiers;
                        let Some(text) = event.keystroke.key_char.clone().filter(|c| !c.trim().is_empty() && !c.chars().any(char::is_control)) else { return };
                        if m.control || m.alt || m.platform { return; }
                        if this.state.selected_capture.is_some() {
                            this.notice = Some(READ_ONLY.into());
                            cx.notify();
                        } else {
                            this.type_into_search(&text, window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .on_action(cx.listener(|this, _: &UndoDelete, window, cx| {
                        this.undo_last(window, cx);
                        this.list_focus.focus(window, cx);
                    }))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mb_1()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Notes"),
                            )
                            .child(
                                ados_core::hints::marked(Button::new("refresh-notes")
                                    .icon(IconName::RefreshCw)
                                    .ghost()
                                    .size(px(32.))
                                    .tooltip("Refresh notes and inbox")
                                    .on_click(cx.listener(|this, _, window, cx| { this.refresh(cx); this.leave_missing(window, cx); this.reload_changed(window, cx); }))),
                            ),
                    )
                    .when(no_matches, |list| list.child(div().p_2().text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("No notes match “{}”. Enter starts a note with it.", self.query))))
                    .children(notes.into_iter().map(|note| {
                        let is_hit = hit.as_ref() == Some(&note.path);
                        self.row(note.path, note.title.clone(), note.title, false, IconName::FileText, is_hit, cx)
                    }))
                    .child(
                        div()
                            .mt_4()
                            .mb_2()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Inbox"),
                    )
                    .children(inbox.into_iter().map(|item| {
                        let icon = match item.kind {
                            CaptureKind::File => IconName::FileText,
                            CaptureKind::Url => IconName::Link,
                            CaptureKind::Image => IconName::Image,
                            CaptureKind::Thought => IconName::Lightbulb,
                        };
                        let is_hit = hit.as_ref() == Some(&item.path);
                        self.row(item.path, file::title_from_text(&item.text), item.text, true, icon, is_hit, cx)
                    })),
            );
        let mut sidebar = Some(sidebar.into_any_element());
        let toolbar = div()
            .flex()
            .items_center()
            .gap_1()
            .p_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .when(self.narrow, |bar| bar.child(ados_core::hints::marked(Button::new("toggle-note-list")
                .icon(IconName::PanelLeft).ghost().size(px(32.))
                .tooltip(if self.list_open { "Show editor" } else { "Show notes and search" })
                .on_click(cx.listener(|this, _, window, cx| {
                    this.list_open = !this.list_open;
                    if this.list_open { this.search.read(cx).focus_handle(cx).focus(window, cx); }
                    else { this.focus_content(window, cx); }
                    cx.notify();
                })))))
            .child(
                ados_core::hints::marked(Button::new("new-note")
                    .icon(IconName::Plus)
                    .ghost()
                    .size(px(32.))
                    .tooltip("New note · Ctrl+N")
                    .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx)))),
            )
            .child(div().flex_1().min_w_0().text_ellipsis().child(if has_item {
                file::title_from_text(&self.text)
            } else {
                "Notes".into()
            }))
            .when(is_capture, |bar| {
                bar.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Read-only"),
                )
                .child(
                    ados_core::hints::marked(Button::new("make-note")
                        .label("Make note")
                        .on_click(cx.listener(|this, _, window, cx| this.make_note(window, cx)))),
                )
            })
            .when(has_item && !is_capture, |bar| {
                bar.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(if self.dirty { "Saving…" } else { "Saved" }),
                )
                .child(
                    ados_core::hints::marked(Button::new("preview-note")
                        .icon(if preview {
                            IconName::EyeOff
                        } else {
                            IconName::Eye
                        })
                        .ghost()
                        .size(px(32.))
                        .tooltip(if preview {
                            "Edit · Ctrl+Shift+P"
                        } else {
                            "Preview · Ctrl+Shift+P"
                        })
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_preview(window, cx)),
                        )),
                )
            })
            .when(!self.undo.is_empty(), |bar| {
                bar.child(
                    ados_core::hints::marked(Button::new("undo-note")
                        .icon(IconName::Undo)
                        .ghost()
                        .size(px(32.))
                        .tooltip("Undo deletion")
                        .on_click(cx.listener(|this, _, window, cx| this.undo_last(window, cx)))),
                )
            })
            .when(has_item, |bar| {
                bar.child(
                    ados_core::hints::marked(Button::new("delete-note")
                        .icon(crate::rows::trash_icon())
                        .ghost()
                        .size(px(32.))
                        .tooltip("Move to trash")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.delete_selected(window, cx)),
                        )),
                )
            });
        let content = if !has_item {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .size_full()
                .gap_2()
                .child("Select a note, or type a new title and press Enter")
                .child(
                    ados_core::hints::marked(Button::new("empty-new-note")
                        .label("New note")
                        .icon(IconName::Plus)
                        .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx)))),
                )
                .into_any_element()
        } else if preview {
            div()
                .id("note-preview-scroll")
                .size_full()
                .overflow_y_scroll()
                .p_4()
                .child(TextView::new(&self.preview).on_link_click(|url, _, window, cx| ados_core::links::open_url(url, window, cx)))
                .into_any_element()
        } else {
            Editor::new(&self.editor).size_full().into_any_element()
        };
        div()
            .id("notes-panel")
            .track_focus(&self.root_focus)
            .capture_any_mouse_down(cx.listener(|s, _, w, cx| s.focus_primary(w, cx)))
            .key_context("NotesPanel")
            // Bubble phase: editor popups, multi-cursor and the find field handle Esc first.
            .on_action(cx.listener(|this, _: &gpui_kit::component::input::Escape, window, cx| {
                this.escape_to_list(window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewNote, window, cx| this.new_note(window, cx)))
            .on_action(
                cx.listener(|this, _: &TogglePreview, window, cx| this.toggle_preview(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenLink, window, cx| this.open_link(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.focus_search(window, cx)
            }))
            .on_key_up(cx.listener(|this, _, _, cx| this.publish(cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.publish(cx)),
            )
            .relative()
            .flex()
            .size_full()
            .min_w_0()
            .child({
                let weak = cx.entity().downgrade();
                canvas(move |bounds, window, cx| {
                    let _ = weak.update(cx, |this, cx| this.panel_bounds(bounds.size.width, window, cx));
                }, |_, _, _, _| {}).absolute().size_full()
            })
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .when(!self.narrow, |body| body.children(sidebar.take()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(toolbar)
                    .when_some(self.conflict.clone(), |body, name| body.child(self.conflict_line(name, cx)))
                    .when_some(self.notice.clone(), |body, notice| {
                        body.child(div().p_2().text_sm().text_color(cx.theme().muted_foreground).child(notice))
                    })
                    .when_some(self.error.clone(), |body, error| {
                        body.child(
                            div()
                                .p_2()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(error)
                                .child(
                                    Button::new("retry-notes-save")
                                        .label("Retry save")
                                        .ghost()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.flush_visible(cx);
                                        })),
                                ),
                        )
                    })
                    .child(div().flex_1().min_h_0().min_w_0()
                        .when(self.narrow && self.list_open, |body| body.children(sidebar.take()))
                        .when(!(self.narrow && self.list_open), |body| body.child(content))),
            )
    }
}
