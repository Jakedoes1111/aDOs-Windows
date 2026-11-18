//! What each key does to the list. Every change goes through `TasksPanel::change` (load, edit, save).
use crate::model::{self, Status, View};
use crate::panel::{today, EditKind, TasksPanel, DONE_KEY};
use crate::{parse, store};
use gpui_kit::component::{button::{Button, ButtonVariants}, notification::Notification, WindowExt};
use gpui_kit::*;

impl TasksPanel {
    pub(crate) fn add_from_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.add.read(cx).value().to_string();
        let parsed = parse::quick_add(&text, today());
        if parsed.title.is_empty() {
            return;
        }
        let in_my_day = self.ui.view == View::MyDay;
        let added = self.change(cx, |tasks| {
            let mut task = store::new_task(tasks, &parsed.title);
            task.due = parsed.due;
            task.project = parsed.project.clone();
            // Added while looking at My Day: it belongs there.
            if in_my_day {
                task.my_day = Some(today());
            }
            tasks.push(task.clone());
            Ok(task)
        });
        if let Some(task) = added {
            self.add.update(cx, |input, cx| input.set_value("", window, cx));
            self.preview = None;
            self.ui.selected = Some(task.id);
            self.after_change(cx);
        }
    }

    pub(crate) fn toggle_done(&mut self, cx: &mut Context<Self>) {
        if self.ui.selected.as_deref() == Some(DONE_KEY) {
            self.ui.done_open = !self.ui.done_open;
            self.after_change(cx);
            return;
        }
        let Some(task) = self.selected_task() else { return };
        let status = if task.is_done() { Status::Open } else { Status::Done };
        self.change(cx, |tasks| store::edit(tasks, &task.id, |t| t.status = status));
    }

    pub(crate) fn toggle_task(&mut self, id: String, cx: &mut Context<Self>) {
        self.ui.selected = Some(id);
        self.toggle_done(cx);
    }

    pub(crate) fn toggle_my_day(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.selected_task() else { return };
        let mark = if task.my_day == Some(today()) { None } else { Some(today()) };
        self.change(cx, |tasks| store::edit(tasks, &task.id, |t| t.my_day = mark));
    }

    pub(crate) fn start_edit(&mut self, kind: EditKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.selected.as_deref() == Some(DONE_KEY) {
            if kind == EditKind::Title {
                self.toggle_done(cx);
            }
            return;
        }
        let Some(task) = self.selected_task() else { return };
        let value = match kind {
            EditKind::Title => task.title.clone(),
            EditKind::Due => task.due.map(|d| d.to_string()).unwrap_or_default(),
        };
        let placeholder = match kind {
            EditKind::Title => "Title",
            EditKind::Due => "today, tomorrow, fri or 2026-11-20; empty clears",
        };
        self.editing = Some((task.id, kind));
        self.edit.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            input.set_value(value, window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    /// Enter saves; leaving the field saves too, unless what was typed is not a date.
    pub(crate) fn commit_edit(&mut self, enter: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, kind)) = self.editing.clone() else { return };
        let text = self.edit.read(cx).value().trim().to_string();
        match kind {
            EditKind::Title if text.is_empty() => {}
            EditKind::Title => {
                self.change(cx, |tasks| store::edit(tasks, &id, |t| t.title = text));
            }
            EditKind::Due => {
                let due = if text.is_empty() { None } else { parse::date(&text, today()) };
                if due.is_none() && !text.is_empty() {
                    if enter {
                        self.error = Some(format!("“{text}” is not a date. Try today, tomorrow, fri or 2026-11-20."));
                        cx.notify();
                        return;
                    }
                } else {
                    self.change(cx, |tasks| store::edit(tasks, &id, |t| t.due = due));
                }
            }
        }
        self.end_edit(enter, window, cx);
    }

    pub(crate) fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_edit(true, window, cx);
    }

    fn end_edit(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.take().is_some() && refocus {
            self.list_focus.focus(window, cx);
        }
        if self.error.as_deref().is_some_and(|e| e.contains("is not a date")) {
            self.error = None;
        }
        self.after_change(cx);
    }

    pub(crate) fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.selected_task() else { return };
        let removed = self.change(cx, |tasks| {
            let ix = tasks.iter().position(|t| t.id == task.id).ok_or_else(|| anyhow::anyhow!("That task no longer exists"))?;
            Ok(tasks.remove(ix))
        });
        let Some(removed) = removed else { return };
        let title = removed.title.clone();
        self.undo.push(removed);
        let weak = cx.entity().downgrade();
        let shell_undo = weak.clone();
        ados_core::notifications::remember_undo(move |window, cx| {
            let _ = shell_undo.update(cx, |panel, cx| panel.undo_delete(window, cx));
        }, cx);
        window.push_notification(
            Notification::new().content(|_, _, _| ados_core::hints::cover_padded(ados_core::hints::Layer::Float, px(16.)).into_any_element())
                .message(format!("Deleted “{title}”."))
                .id::<TasksPanel>()
                .autohide(false)
                .action(move |_, _, _| {
                    let weak = weak.clone();
                    Button::new("undo-deleted-task").label("Undo").ghost().child(ados_core::hints::mark_in(ados_core::hints::Layer::Float)).on_click(move |_, window, cx| {
                        let _ = weak.update(cx, |panel, cx| panel.undo_delete(window, cx));
                    })
                }),
            cx,
        );
    }

    pub(crate) fn delete_task(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.ui.selected = Some(id);
        self.delete_selected(window, cx);
    }

    pub(crate) fn undo_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.undo.last().cloned() else { return };
        let restored = self.change(cx, |tasks| {
            if !tasks.iter().any(|t| t.id == task.id) {
                tasks.push(task.clone());
                tasks.sort_by_key(|t| t.order);
            }
            Ok(())
        });
        if restored.is_some() {
            self.undo.pop();
            if self.undo.is_empty() {
                window.remove_notification::<TasksPanel>(cx);
            }
            self.select(task.id, window, cx);
        }
    }

    /// Alt+Up/Down: swap with the neighbour in the same group.
    pub(crate) fn reorder(&mut self, up: bool, cx: &mut Context<Self>) {
        let Some(task) = self.selected_task() else { return };
        let Some(other) = model::swap_partner(&self.lines(), &task.id, up) else { return };
        self.change(cx, |tasks| {
            let a = tasks.iter().position(|t| t.id == task.id);
            let b = tasks.iter().position(|t| t.id == other);
            if let (Some(a), Some(b)) = (a, b) {
                let order = tasks[a].order;
                tasks[a].order = tasks[b].order;
                tasks[b].order = order;
                tasks.sort_by_key(|t| t.order);
            }
            Ok(())
        });
    }
}
