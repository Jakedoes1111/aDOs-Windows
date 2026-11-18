//! List rows keep destructive controls visible and scope Delete away from the editor.
use crate::panel::{DeleteSelected, NotesPanel};
use gpui_kit::*;
use gpui_kit::{assets::IconName, prelude::FluentBuilder};
use gpui_kit::component::{ActiveTheme, Icon, Selectable, button::{Button, ButtonVariants}, menu::{ContextMenuExt, PopupMenuItem}};
use std::path::PathBuf;

// Trash is not in gpui-kit's bundled default icon subset. Embed it rather than
// leaving a clickable but invisible square when the host uses Assets::default().
pub(crate) fn trash_icon() -> Icon {
    Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M9 6V4h6v2M5 6l1 14h12l1-14M10 10v6M14 10v6"/></svg>"#)
}

impl NotesPanel {
    pub(crate) fn open_row(&mut self, path: PathBuf, capture: bool, window: &mut Window, cx: &mut Context<Self>) {
        let row_focus = self.row_focus.get(&path).cloned();
        if capture {
            if let Some(item) = self.inbox.iter().find(|item| item.path == path).cloned() {
                self.select_capture(item, window, cx);
            }
        } else {
            self.select_note(path, window, cx);
        }
        self.list_open = false;
        // A note opens with the caret in its editor; a read-only capture keeps the row, so
        // ↑/↓ go on through the list.
        match row_focus {
            Some(focus) if capture && !self.narrow => focus.focus(window, cx),
            _ => self.focus_content(window, cx),
        }
        cx.notify();
    }

    /// Rows in the order the list shows them (search applied).
    pub(crate) fn visible_rows(&self) -> Vec<PathBuf> {
        let query = self.query.to_lowercase();
        self.notes.iter().filter(|n| n.search_text.contains(&query)).map(|n| n.path.clone())
            .chain(self.inbox.iter().filter(|i| i.text.to_lowercase().contains(&query)).map(|i| i.path.clone()))
            .collect()
    }

    /// ↑/↓ in the list: move to the previous or next row (Enter opens it).
    pub(crate) fn step_row(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.visible_rows();
        if rows.is_empty() { return; }
        let focused = rows.iter().position(|p| self.row_focus.get(p).is_some_and(|f| f.is_focused(window)));
        // The focused row was removed (undo, trash, the assistant): go on from where it was,
        // so ↓ reaches the row that took its place, not the open item elsewhere in the list.
        let removed = self.last_row.as_ref().filter(|(_, p)| !rows.contains(p)).map(|(i, _)| *i);
        let next = match (focused, removed) {
            (None, Some(i)) => next_after_removed(i, delta, rows.len()),
            _ => {
                let current = focused.or_else(|| {
                    let open = self.state.selected_capture.as_ref().or(self.state.selected.as_ref());
                    rows.iter().position(|p| Some(p) == open)
                });
                match current {
                    Some(i) => (i as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
                    None if delta < 0 => rows.len() - 1,
                    None => 0,
                }
            }
        };
        if let Some(focus) = self.row_focus.get(&rows[next]) {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn row(&self, path: PathBuf, title: String, tooltip: String, capture: bool, icon: IconName, hit: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = if capture { self.state.selected_capture.as_ref() == Some(&path) }
            else { self.state.selected_capture.is_none() && self.state.selected.as_ref() == Some(&path) };
        let open_path = path.clone();
        let delete_path = path.clone();
        let key_path = path.clone();
        let weak = cx.entity().downgrade();
        div().id(SharedString::from(format!("row-{}", path.display())))
            .flex().items_center().w_full().min_w_0()
            .rounded(px(8.)).border_1()
            .border_color(if hit { cx.theme().ring } else { transparent_black() })
            .key_context("NotesList")
            .on_action(cx.listener(move |this, _: &DeleteSelected, window, cx| {
                this.delete_item(key_path.clone(), window, cx);
                cx.stop_propagation();
            }))
            .child(Button::new(SharedString::from(format!("open-{}", path.display())))
                .child(div().flex_1().min_w_0().text_ellipsis().child(title))
                .child(ados_core::hints::mark())
                .when_some(self.row_focus.get(&path), |button, focus| button.track_focus(focus))
                .icon(icon).ghost().selected(selected)
                .when(selected, |button| button.bg(cx.theme().primary.opacity(0.18)))
                .flex_1().min_w_0().h(px(32.)).justify_start().tooltip(tooltip)
                .on_click(cx.listener(move |this, _, window, cx| this.open_row(open_path.clone(), capture, window, cx))))
            .child(ados_core::hints::marked(Button::new(SharedString::from(format!("trash-{}", path.display())))
                .icon(trash_icon()).ghost().size(px(32.)).flex_shrink_0().tooltip("Move to trash · Delete")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.delete_item(delete_path.clone(), window, cx);
                    this.list_focus.focus(window, cx);
                }))))
            .context_menu(move |menu, _, _| {
                let open = path.clone();
                let delete = path.clone();
                let make = path.clone();
                let a = weak.clone();
                let b = weak.clone();
                let c = weak.clone();
                menu.item(PopupMenuItem::new("Open").on_click(move |_, window, cx| {
                    let _ = a.update(cx, |this, cx| this.open_row(open.clone(), capture, window, cx));
                }))
                .when(capture, |menu| menu.item(PopupMenuItem::new("Make note").on_click(move |_, window, cx| {
                    let _ = c.update(cx, |this, cx| {
                        this.open_row(make.clone(), true, window, cx);
                        if this.state.selected_capture.as_ref() == Some(&make) { this.make_note(window, cx); }
                    });
                })))
                .item(PopupMenuItem::new("Delete").icon(trash_icon()).on_click(move |_, window, cx| {
                    let _ = b.update(cx, |this, cx| this.delete_item(delete.clone(), window, cx));
                }))
            })
    }

    /// The row Enter in the search box opens: the first match, while there is a query.
    pub(crate) fn search_hit(&self, window: &Window, cx: &App) -> Option<PathBuf> {
        if self.query.trim().is_empty() || !self.search.read(cx).focus_handle(cx).is_focused(window) {
            return None;
        }
        self.visible_rows().into_iter().next()
    }

    /// Enter in the search box: open the first match, or with no match start a note whose
    /// first line is what was typed (the empty state says so).
    pub(crate) fn open_first_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.visible_rows().into_iter().next() else {
            let title = self.query.trim().to_string();
            if title.is_empty() { return; }
            self.new_note(window, cx);
            if self.state.selected.is_some() && self.query.is_empty() {
                self.editor.update(cx, |editor, cx| editor.insert(title, window, cx));
            }
            return;
        };
        let capture = self.inbox.iter().any(|item| item.path == path);
        self.open_row(path, capture, window, cx);
    }

    /// ↓/↑ in the search box: into the list, at its first or last row.
    pub(crate) fn search_step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.visible_rows();
        let target = if delta > 0 { rows.first() } else { rows.last() };
        if let Some(focus) = target.and_then(|path| self.row_focus.get(path)) {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    /// A character typed on a list row goes into the search box rather than nowhere.
    pub(crate) fn type_into_search(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.list_open = true;
        self.search.update(cx, |input, cx| {
            input.focus(window, cx);
            input.insert(text.to_string(), window, cx);
        });
        cx.notify();
    }

    pub(crate) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list_open = true;
        self.search.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub(crate) fn panel_bounds(&mut self, width: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        let narrow = width < px(420.);
        if self.narrow == narrow { return; }
        self.narrow = narrow;
        self.list_open = false;
        let weak = cx.entity().downgrade();
        window.defer(cx, move |_, cx| { let _ = weak.update(cx, |_, cx| cx.notify()); });
    }
}

/// The row ↑/↓ lands on when the focused row at `index` has just been removed from a list
/// now `len` long: ↓ takes the row that moved into its place, ↑ the one above it.
fn next_after_removed(index: usize, delta: isize, len: usize) -> usize {
    let target = if delta > 0 { index } else { index.saturating_sub(1) };
    target.min(len.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::next_after_removed;
    #[test]
    fn steps_from_a_removed_row() {
        // Top row of 3 removed, 2 left: ↓ goes to the new top row, ↑ stays on it.
        assert_eq!(next_after_removed(0, 1, 2), 0);
        assert_eq!(next_after_removed(0, -1, 2), 0);
        // Middle row removed: ↓ to the row below it (now at its index), ↑ to the one above.
        assert_eq!(next_after_removed(2, 1, 4), 2);
        assert_eq!(next_after_removed(2, -1, 4), 1);
        // Last row removed: ↓ clamps to the new last row.
        assert_eq!(next_after_removed(3, 1, 3), 2);
    }
}
