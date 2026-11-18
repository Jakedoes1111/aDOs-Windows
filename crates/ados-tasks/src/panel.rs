//! The Tasks dock panel: state, actions and the changes they make. Rendering is in `view`.
use crate::model::{self, Line, Task, View};
use crate::{parse, store};
use ados_core::{hub::{ContextHub, Snapshot}, ids, paths};
use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    input::{InputEvent, InputState},
};
use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

actions!(ados_tasks, [
    SelectPrevious, SelectNext, ToggleDone, EditTitle, EditDue, ToggleMyDay, DeleteTask,
    UndoDelete, MoveTaskUp, MoveTaskDown, SwitchView, FocusAdd, CancelEdit, ToList, OpenLink,
]);

/// The Done heading's selection key; task ids never start with `#`.
pub(crate) const DONE_KEY: &str = "#done";

#[derive(Default)]
struct TasksTarget(Option<WeakEntity<TasksPanel>>, bool);
impl Global for TasksTarget {}

pub fn init(cx: &mut App) {
    cx.set_global(TasksTarget::default());
    use ados_core::shortcuts::bind;
    bind(cx, "tasks.previous", SelectPrevious);
    bind(cx, "tasks.next", SelectNext);
    bind(cx, "tasks.toggle", ToggleDone);
    bind(cx, "tasks.edit", EditTitle);
    bind(cx, "tasks.due", EditDue);
    bind(cx, "tasks.my_day", ToggleMyDay);
    bind(cx, "tasks.delete", DeleteTask);
    bind(cx, "tasks.undo_delete", UndoDelete);
    bind(cx, "tasks.move_up", MoveTaskUp);
    bind(cx, "tasks.move_down", MoveTaskDown);
    bind(cx, "tasks.switch_view", SwitchView);
    bind(cx, "tasks.add", FocusAdd);
    bind(cx, "tasks.cancel", CancelEdit);
    bind(cx, "tasks.to_list", ToList);
    bind(cx, "tasks.open_link", OpenLink);
    // "Open Tasks" comes from the shell's module list (`shell.focus.tasks`).
    ados_core::actions::register("tasks.add", "Add task", "tasks todo new quick add", None, cx, |window, cx| {
        cx.global_mut::<TasksTarget>().1 = true;
        let open = ados_core::actions::ActionRegistry::all(cx).into_iter().find(|a| a.id.as_ref() == "shell.focus.tasks");
        if let Some(open) = open {
            (open.run)(window, cx);
        }
        // Runs after the shell has opened and focused the panel (and in the standalone example,
        // where there is no shell). Always the field: typed letters must never reach the list.
        window.defer(cx, |window, cx| {
            cx.global_mut::<TasksTarget>().1 = false;
            if let Some(panel) = cx.global::<TasksTarget>().0.clone() {
                let _ = panel.update(cx, |panel, cx| panel.focus_add(window, cx));
            }
        });
    });
}

/// Show the file again in an open Tasks panel, without moving focus (another crate,
/// e.g. aDOs's MCP server, changed it).
pub fn refresh(cx: &mut App) {
    if let Some(panel) = cx.try_global::<TasksTarget>().and_then(|t| t.0.clone()) {
        let _ = panel.update(cx, |panel, cx| {
            panel.reload(cx);
            cx.notify();
        });
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct UiState {
    pub view: View,
    pub done_open: bool,
    pub selected: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum EditKind { Title, Due }

pub struct TasksPanel {
    pub(crate) focus: FocusHandle,
    pub(crate) list_focus: FocusHandle,
    pub(crate) add: Entity<InputState>,
    pub(crate) edit: Entity<InputState>,
    pub(crate) editing: Option<(String, EditKind)>,
    pub(crate) tasks: Vec<Task>,
    pub(crate) ui: UiState,
    /// A plain sentence about the last thing that went wrong.
    pub(crate) error: Option<String>,
    /// What Enter in the quick-add field will do.
    pub(crate) preview: Option<String>,
    pub(crate) undo: Vec<Task>,
    pub(crate) scroll: ScrollHandle,
    path: PathBuf,
    _subscriptions: Vec<Subscription>,
}

pub(crate) fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

impl TasksPanel {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let add = cx.new(|cx| InputState::new(window, cx).placeholder("Add a task: Call bank fri #home"));
        let edit = cx.new(|cx| InputState::new(window, cx));
        let focus = cx.focus_handle();
        let mut error = None;
        let ui = match std::fs::read_to_string(paths::state_file("tasks")) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                error = Some(format!("Could not restore the Tasks view ({e}); showing My Day."));
                UiState::default()
            }),
            Err(_) => UiState::default(),
        };
        let subscriptions = vec![
            cx.subscribe_in(&add, window, |this, _, event, window, cx| match event {
                InputEvent::Change => {
                    let text = this.add.read(cx).value().to_string();
                    this.preview = parse::describe(&parse::quick_add(&text, today()), today());
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.add_from_field(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&edit, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.commit_edit(true, window, cx),
                InputEvent::Blur => this.commit_edit(false, window, cx),
                _ => {}
            }),
            cx.on_focus_in(&focus, window, |this, _, cx| {
                ContextHub::set_active(ids::TASKS, cx);
                this.reload(cx);
            }),
        ];
        cx.global_mut::<TasksTarget>().0 = Some(cx.entity().downgrade());
        let mut panel = Self {
            focus,
            list_focus: cx.focus_handle(),
            add,
            edit,
            editing: None,
            tasks: Vec::new(),
            ui,
            error,
            preview: None,
            undo: Vec::new(),
            scroll: ScrollHandle::new(),
            path: store::default_path(),
            _subscriptions: subscriptions,
        };
        panel.reload(cx);
        panel
    }

    /// Ctrl+7, the rail and "Open Tasks": the list, or the quick-add field when asked or empty.
    /// Stable handle on the panel root: focus anywhere in the panel is inside it.
    pub fn root_focus(&self) -> FocusHandle {
        self.focus.clone()
    }
    pub fn focus_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wants_add = std::mem::take(&mut cx.global_mut::<TasksTarget>().1);
        self.reload(cx);
        if wants_add || self.selectable().is_empty() {
            self.focus_add(window, cx);
        } else {
            self.list_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn focus_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Read the file again (another caller may have changed it) and show it.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        match store::load(&self.path) {
            Ok(tasks) => {
                self.tasks = tasks;
                if self.error.as_deref().is_some_and(|e| e.contains("could not be read")) {
                    self.error = None;
                }
            }
            Err(e) => {
                self.tasks.clear();
                self.error = Some(format!("{e:#}"));
            }
        }
        self.publish(cx);
        cx.notify();
    }

    pub(crate) fn lines(&self) -> Vec<Line> {
        model::lines(&self.tasks, self.ui.view, today(), self.ui.done_open)
    }

    /// (line index, selection key) for every line the keyboard can land on.
    pub(crate) fn selectable(&self) -> Vec<(usize, String)> {
        self.lines().iter().enumerate().filter_map(|(ix, line)| match line {
            Line::Task(t) => Some((ix, t.id.clone())),
            Line::DoneToggle { .. } => Some((ix, DONE_KEY.to_string())),
            Line::Group(_) => None,
        }).collect()
    }

    pub(crate) fn selected_task(&self) -> Option<Task> {
        let id = self.ui.selected.as_deref()?;
        self.tasks.iter().find(|t| t.id == id).cloned()
    }

    /// Keep a visible selection; when the selected line went away, take the one now at its place.
    fn fix_selection(&mut self, old_index: Option<usize>) {
        let items = self.selectable();
        if items.iter().any(|(_, key)| Some(key) == self.ui.selected.as_ref()) {
            return;
        }
        let ix = old_index.unwrap_or(0).min(items.len().saturating_sub(1));
        self.ui.selected = items.get(ix).map(|(_, key)| key.clone());
    }

    fn selected_index(&self) -> Option<usize> {
        self.selectable().iter().position(|(_, key)| Some(key) == self.ui.selected.as_ref())
    }

    pub(crate) fn select(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        self.ui.selected = Some(key);
        self.list_focus.focus(window, cx);
        self.after_change(cx);
    }

    pub(crate) fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items = self.selectable();
        if items.is_empty() {
            return;
        }
        let next = match self.selected_index() {
            Some(ix) => (ix as isize + delta).clamp(0, items.len() as isize - 1) as usize,
            None => 0,
        };
        self.ui.selected = Some(items[next].1.clone());
        self.after_change(cx);
    }

    /// Save the view state, scroll the selection into view, tell the assistant, repaint.
    pub(crate) fn after_change(&mut self, cx: &mut Context<Self>) {
        let line = self.selectable().into_iter().find(|(_, key)| Some(key) == self.ui.selected.as_ref());
        if let Some((ix, _)) = line {
            self.scroll.scroll_to_item(ix);
        }
        let saved = serde_json::to_string_pretty(&self.ui).map_err(anyhow::Error::from)
            .and_then(|text| std::fs::write(paths::state_file("tasks"), text).map_err(Into::into));
        if let Err(e) = saved {
            self.error = Some(format!("Could not remember the Tasks view: {e}"));
        }
        self.publish(cx);
        cx.notify();
    }

    /// Apply one change to the file, then show the result.
    pub(crate) fn change<R>(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Vec<Task>) -> anyhow::Result<R>) -> Option<R> {
        let old_index = self.selected_index();
        let result = store::update(&self.path, f);
        let out = match result {
            Ok(r) => {
                self.error = None;
                Some(r)
            }
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                None
            }
        };
        if let Ok(tasks) = store::load(&self.path) {
            self.tasks = tasks;
        }
        self.fix_selection(old_index);
        self.after_change(cx);
        out
    }

    pub(crate) fn publish(&self, cx: &mut App) {
        let lines = self.lines();
        let heading = match self.ui.view { View::MyDay => "My Day", View::All => "All tasks" };
        let mut text = format!("{heading}\n\n{}", model::as_text(&lines, today()));
        if let Some(error) = &self.error {
            text = format!("{error}\n\n{text}");
        }
        let selection = self.selected_task().map(|t| t.title.into());
        ContextHub::publish(Snapshot {
            module: ids::TASKS.into(),
            title: "Tasks".into(),
            location: Some(self.path.to_string_lossy().into_owned().into()),
            text: text.into(),
            selection,
        }, cx);
    }

    pub(crate) fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.ui.view = view;
        self.fix_selection(None);
        self.after_change(cx);
    }
}

impl EventEmitter<PanelEvent> for TasksPanel {}
impl Focusable for TasksPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.list_focus.clone()
    }
}
impl Panel for TasksPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Tasks"
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}
impl BasePanel for TasksPanel {
    fn panel_name(&self) -> &'static str {
        ids::TASKS
    }
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if active {
            self.reload(cx);
        }
    }
}
