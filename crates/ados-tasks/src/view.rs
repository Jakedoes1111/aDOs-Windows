//! Drawing the panel: view switch, quick add, the list.
use crate::model::{self, Line, Task, View};
use crate::panel::*;
use ados_core::shortcuts::tooltip;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex, ActiveTheme, Icon, IconName, Selectable, Sizable,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

// Trash is not in gpui-kit's bundled default icon subset, so it is embedded (as Notes does).
fn trash_icon() -> Icon {
    Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M9 6V4h6v2M5 6l1 14h12l1-14M10 10v6M14 10v6"/></svg>"#)
}

impl TasksPanel {
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view_button = |id: &'static str, label: &'static str, view: View, icon: Option<IconName>, cx: &mut Context<Self>| {
            ados_core::hints::marked(Button::new(id)
                .label(label)
                .when_some(icon, |b, icon| b.icon(icon))
                .ghost()
                .h(px(32.))
                .selected(self.ui.view == view)
                .tooltip(tooltip(label, "tasks.switch_view", cx))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_view(view, cx);
                    this.list_focus.focus(window, cx);
                })))
        };
        h_flex()
            .px_3()
            .pt_3()
            .gap_1()
            .child(view_button("tasks-my-day", "My Day", View::MyDay, Some(IconName::Sun), cx))
            .child(view_button("tasks-all", "All", View::All, None, cx))
            .child(div().flex_1())
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(today().format("%a %-d %b").to_string()))
    }

    fn quick_add(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .px_3()
            .pt_2()
            .gap_1()
            .key_context("TasksAdd")
            .on_action(cx.listener(|this, _: &ToList, window, cx| {
                this.list_focus.focus(window, cx);
                this.step(0, cx);
            }))
            .child(div().relative().child(Input::new(&self.add).prefix(Icon::new(IconName::Plus))).child(ados_core::hints::mark()))
            .child(
                div()
                    .h(px(18.))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .children(self.preview.clone()),
            )
    }

    fn task_row(&self, task: &Task, list_focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let selected = self.ui.selected.as_deref() == Some(task.id.as_str());
        let editing = self.editing.as_ref().filter(|(id, _)| *id == task.id).map(|(_, kind)| *kind);
        let today = today();
        let my_day = task.my_day == Some(today);
        let (id_select, id_toggle, id_delete) = (task.id.clone(), task.id.clone(), task.id.clone());
        let edit_field = || {
            div()
                .flex_1()
                .min_w_0()
                .key_context("TasksEdit")
                .on_action(cx.listener(|this, _: &CancelEdit, window, cx| this.cancel_edit(window, cx)))
                .child(Input::new(&self.edit).small())
        };
        // The title keeps its full width first: it grows and shrinks from its own size,
        // while the date and project chips shrink (to nothing) many times faster.
        let title = div()
            .flex_auto()
            .min_w_0()
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .when(task.is_done(), |d| d.line_through().text_color(theme.muted_foreground))
            .child(task.title.clone());
        let chip = || div().flex_shrink(40.).min_w_0().overflow_hidden().whitespace_nowrap().text_xs();
        let due = task.due.map(|due| {
            chip()
                .text_color(if due < today && !task.is_done() { theme.danger } else { theme.muted_foreground })
                .child(model::due_label(due, today))
        });
        let project = task.project.clone().filter(|_| self.ui.view == View::MyDay).map(|p| {
            chip().text_color(theme.muted_foreground).child(format!("#{p}"))
        });
        h_flex()
            .id(SharedString::from(format!("task-{}", task.id)))
            .h(px(40.))
            .px_1()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(if selected && list_focused { theme.primary } else { transparent_black() })
            .when(selected, |row| row.bg(theme.primary.opacity(0.18)))
            .on_click(cx.listener(move |this, _, window, cx| this.select(id_select.clone(), window, cx)))
            .child(ados_core::hints::mark())
            .child(
                Button::new(SharedString::from(format!("done-{}", task.id)))
                    .icon(if task.is_done() { IconName::CircleCheck } else { IconName::Square })
                    .ghost()
                    .size(px(32.))
                    .flex_shrink_0()
                    .tooltip(tooltip(if task.is_done() { "Mark open" } else { "Mark done" }, "tasks.toggle", cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_task(id_toggle.clone(), cx))),
            )
            .map(|row| match editing {
                Some(EditKind::Title) => row.child(edit_field()),
                Some(EditKind::Due) => row.child(title).child(div().w(px(220.)).flex_shrink_0().child(edit_field())),
                None => row.child(title).children(project).children(due),
            })
            .children(crate::links::task_url(task).map(|url| {
                ados_core::hints::marked(Button::new(SharedString::from(format!("link-{}", task.id)))
                    .icon(gpui_kit::assets::IconName::ExternalLink).ghost().size(px(32.)).flex_shrink_0()
                    .tooltip(format!("Open {url} in the Browser"))
                    .on_click(move |_, window, cx| ados_core::links::open_url(&url, window, cx)))
            }))
            .child(
                ados_core::hints::marked(Button::new(SharedString::from(format!("myday-{}", task.id)))
                    .icon(Icon::new(IconName::Sun).when(my_day, |i| i.text_color(theme.primary)))
                    .ghost()
                    .size(px(32.))
                    .flex_shrink_0()
                    .tooltip(tooltip(if my_day { "Remove from My Day" } else { "Add to My Day" }, "tasks.my_day", cx))
                    .on_click(cx.listener({
                        let id = task.id.clone();
                        move |this, _, _, cx| {
                            this.ui.selected = Some(id.clone());
                            this.toggle_my_day(cx);
                        }
                    }))),
            )
            .child(
                ados_core::hints::marked(Button::new(SharedString::from(format!("delete-{}", task.id)))
                    .icon(trash_icon())
                    .ghost()
                    .size(px(32.))
                    .flex_shrink_0()
                    .tooltip(tooltip("Delete task", "tasks.delete", cx))
                    .on_click(cx.listener(move |this, _, window, cx| this.delete_task(id_delete.clone(), window, cx)))),
            )
            .into_any_element()
    }

    fn line(&self, line: &Line, list_focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        match line {
            Line::Task(task) => self.task_row(task, list_focused, cx),
            Line::Group(name) => div()
                .px_2()
                .pt_3()
                .pb_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(name.clone())
                .into_any_element(),
            Line::DoneToggle { count, open } => {
                let selected = self.ui.selected.as_deref() == Some(DONE_KEY);
                h_flex()
                    .id("tasks-done-toggle")
                    .mt_2()
                    .h(px(36.))
                    .px_2()
                    .gap_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if selected && list_focused { theme.primary } else { transparent_black() })
                    .when(selected, |row| row.bg(theme.primary.opacity(0.18)))
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.select(DONE_KEY.into(), window, cx);
                        this.toggle_done(cx);
                    }))
                    .child(Icon::new(if *open { IconName::ChevronDown } else { IconName::ChevronRight }))
                    .child(format!("Done ({count})"))
                    .child(ados_core::hints::mark())
                    .into_any_element()
            }
        }
    }

    fn empty_text(&self) -> &'static str {
        if self.error.as_deref().is_some_and(|e| e.contains("could not be read")) {
            return "";
        }
        match self.ui.view {
            View::MyDay => "Nothing due today. Tasks you add here, or mark with the sun, show up here.",
            View::All => "No tasks yet. Type one above and press Enter.",
        }
    }
}

impl Render for TasksPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list_focused = self.list_focus.is_focused(window);
        let lines = self.lines();
        let rows: Vec<AnyElement> = lines.iter().map(|line| self.line(line, list_focused, cx)).collect();
        let empty = rows.is_empty().then(|| self.empty_text());
        let list = div()
            .id("tasks-list")
            .track_focus(&self.list_focus)
            // While a title or date is being typed, single-letter keys must reach the field.
            .when(self.editing.is_none(), |list| list.key_context("TasksList"))
            // ↑/↓ from a row's checkbox (reached with Tab) take focus back to the list, so one
            // marker shows and Space acts on the selected row, not the checkbox left behind.
            .on_action(cx.listener(|this, _: &SelectPrevious, window, cx| { this.list_focus.focus(window, cx); this.step(-1, cx); }))
            .on_action(cx.listener(|this, _: &SelectNext, window, cx| { this.list_focus.focus(window, cx); this.step(1, cx); }))
            .on_action(cx.listener(|this, _: &ToggleDone, _, cx| this.toggle_done(cx)))
            .on_action(cx.listener(|this, _: &EditTitle, window, cx| this.start_edit(EditKind::Title, window, cx)))
            .on_action(cx.listener(|this, _: &EditDue, window, cx| this.start_edit(EditKind::Due, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleMyDay, _, cx| this.toggle_my_day(cx)))
            .on_action(cx.listener(|this, _: &OpenLink, window, cx| this.open_link(window, cx)))
            .on_action(cx.listener(|this, _: &DeleteTask, window, cx| this.delete_selected(window, cx)))
            .on_action(cx.listener(|this, _: &UndoDelete, window, cx| this.undo_delete(window, cx)))
            .on_action(cx.listener(|this, _: &MoveTaskUp, _, cx| this.reorder(true, cx)))
            .on_action(cx.listener(|this, _: &MoveTaskDown, _, cx| this.reorder(false, cx)))
            .on_action(cx.listener(|this, _: &SwitchView, _, cx| {
                let next = if this.ui.view == View::MyDay { View::All } else { View::MyDay };
                this.set_view(next, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusAdd, window, cx| this.focus_add(window, cx)))
            // A letter that is not a list key (those run as actions first) starts a new task
            // in the add field instead of going nowhere.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let k = &event.keystroke;
                if this.editing.is_some() || k.modifiers.control || k.modifiers.alt || k.modifiers.platform { return; }
                let Some(text) = k.key_char.clone().filter(|c| !c.trim().is_empty() && !c.chars().any(char::is_control)) else { return };
                this.add.update(cx, |input, cx| {
                    input.focus(window, cx);
                    input.insert(text, window, cx);
                });
                cx.stop_propagation();
            }))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px_2()
            .py_2()
            .children(rows)
            .when_some(empty, |list, text| {
                list.child(div().p_3().text_sm().text_color(cx.theme().muted_foreground).child(text))
            });
        v_flex()
            .id("tasks-panel")
            .key_context("TasksPanel")
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.header(cx))
            .child(self.quick_add(cx))
            .when_some(self.error.clone(), |panel, error| {
                panel.child(div().px_3().py_1().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(list)
    }
}
