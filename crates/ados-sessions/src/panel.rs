//! The Sessions panel: struct, construction, persistence, context publishing and the
//! dock/panel traits. Backend lifecycle is in `backends`, rendering in `list` and `view`.
use crate::backends::Slot;
use crate::model::{self, Entry, Transcript};
use crate::questions::PendingQuestion;
use crate::state::Saved;
use ados_core::{
    actions::ActionRegistry,
    hub::{ContextHub, Snapshot},
    ids,
};
use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    input::{InputEvent, InputState, TextareaState},
};
use gpui_kit::*;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Default)]
struct Target(Option<WeakEntity<SessionsPanel>>);
impl Global for Target {}

actions!(ados_sessions, [FocusFilter, NewSession, ToggleList]);

/// Below this panel width the list and the chat cannot both fit (list 240 + chat 400):
/// the list collapses by itself and "show" opens it over the chat.
const NARROW: f32 = 640.;

pub fn init(cx: &mut App) {
    cx.set_global(Target::default());
    ados_core::shortcuts::bind(cx, "sessions.filter", FocusFilter);
    ados_core::shortcuts::bind(cx, "sessions.new", NewSession);
    ados_core::shortcuts::bind(cx, "sessions.toggle_list", ToggleList);
    ados_core::actions::register(
        "sessions.new",
        "New session",
        "sessions agent chat backend claude pi codex start",
        None,
        cx,
        |window, cx| {
            // Bring the module up first (the shell's own action), then show the backend picker.
            if let Some(open) = ActionRegistry::all(cx).into_iter().find(|a| a.id.as_ref() == "shell.focus.sessions") {
                (open.run)(window, cx);
            }
            if let Some(panel) = cx.global::<Target>().0.clone() {
                let _ = panel.update(cx, |panel, cx| panel.show_picker(window, cx));
            }
        },
    );
}

pub struct SessionsPanel {
    pub(crate) saved: Saved,
    /// A plain sentence about the panel itself (state file, backends.toml).
    pub(crate) error: Option<String>,
    pub(crate) slots: Vec<Slot>,
    pub(crate) transcripts: HashMap<String, Transcript>,
    /// Sessions with a prompt turn in flight, by key.
    pub(crate) running: HashMap<String, Task<()>>,
    pub(crate) questions: Vec<PendingQuestion>,
    pub(crate) focus: FocusHandle,
    pub(crate) list_focus: FocusHandle,
    pub(crate) filter: Entity<InputState>,
    pub(crate) composer: Entity<TextareaState>,
    pub(crate) list_scroll: ScrollHandle,
    pub(crate) transcript_scroll: ScrollHandle,
    /// Keyboard highlight in the list.
    pub(crate) cursor: usize,
    /// The new-session backend picker is showing on the right.
    pub(crate) picking: bool,
    /// Backend a new session is being started on.
    pub(crate) creating: Option<usize>,
    /// Highlighted backend in the picker.
    pub(crate) pick_cursor: usize,
    pub(crate) picker_focus: FocusHandle,
    /// The open session's first permission option; Tab from the composer lands here.
    pub(crate) question_focus: FocusHandle,
    /// Thought items the user expanded: (session key, item index).
    pub(crate) expanded: HashSet<(String, usize)>,
    /// Backends were started (lazily, on first render).
    pub(crate) started: bool,
    pub(crate) scroll_restored: bool,
    /// Set while the composer is being refilled, so that is not taken as typing.
    pub(crate) loading_draft: bool,
    /// Panel width from the last paint (0 until measured).
    pub(crate) width: Rc<Cell<f32>>,
    /// Narrow panel: the list is open over the chat. Not saved.
    pub(crate) overlay: bool,
    /// The list was drawn last frame (to restore its scroll when it comes back).
    pub(crate) list_was_shown: bool,
    save_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl SessionsPanel {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (saved, mut error) = Saved::load();
        let configs = ados_agents::config::load().unwrap_or_else(|e| {
            error = Some(format!("{e:#}"));
            vec![]
        });
        let filter = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Filter sessions");
            input.set_value(saved.filter.clone(), window, cx);
            input
        });
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("Continue this session… Enter sends, Esc stops")
        });
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.subscribe_in(&filter, window, |this, _, event, window, cx| match event {
                InputEvent::Change => {
                    this.saved.filter = this.filter.read(cx).value().to_string();
                    this.cursor = this.rows().iter().position(|r| matches!(r, crate::list::ListRow::Session(_))).unwrap_or(0);
                    this.persist_soon(cx);
                    cx.notify();
                }
                // Enter in the filter opens the highlighted match and puts the keyboard in
                // its composer; with nothing to open the keyboard stays in the filter.
                InputEvent::PressEnter { .. } => {
                    this.list_key("enter", window, cx);
                }
                _ => {}
            }),
            cx.subscribe_in(&composer, window, |this, _, event, window, cx| match event {
                InputEvent::Change if !this.loading_draft => this.draft_changed(cx),
                InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                _ => {}
            }),
            cx.on_focus_in(&focus, window, |this, _, cx| {
                ContextHub::set_active(ids::SESSIONS, cx);
                this.publish(cx);
            }),
            cx.on_release(|this, _| {
                this.deny_all();
                let _ = this.saved.save();
            }),
            cx.on_app_quit(|this, _| {
                let _ = this.saved.save();
                async {}
            }),
        ];
        cx.global_mut::<Target>().0 = Some(cx.entity().downgrade());
        let mut panel = Self {
            slots: configs.into_iter().map(Slot::new).collect(),
            saved,
            error,
            transcripts: HashMap::new(),
            running: HashMap::new(),
            questions: vec![],
            focus,
            list_focus: cx.focus_handle(),
            filter,
            composer,
            list_scroll: ScrollHandle::new(),
            transcript_scroll: ScrollHandle::new(),
            cursor: 0,
            picking: false,
            creating: None,
            pick_cursor: 0,
            picker_focus: cx.focus_handle(),
            question_focus: cx.focus_handle(),
            expanded: HashSet::new(),
            started: false,
            scroll_restored: false,
            loading_draft: false,
            width: Rc::new(Cell::new(0.)),
            overlay: false,
            list_was_shown: false,
            save_task: None,
            _subscriptions: subscriptions,
        };
        if let Some(backend) = panel.saved.backend.clone() {
            if !panel.slots.iter().any(|s| s.config.id == backend) {
                panel.saved.backend = None;
            }
        }
        panel.load_draft(window, cx);
        panel.publish(cx);
        panel
    }

    /// `Ctrl+6`, the rail and the palette land here: the list, so arrows work at once.
    /// Stable handle on the panel root: focus anywhere in the panel is inside it.
    pub fn root_focus(&self) -> FocusHandle {
        self.focus.clone()
    }
    pub fn focus_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cursor_to_selected();
        if !self.list_shown() && (self.picking || self.saved.selected.is_none()) {
            // Nothing else to land on: bring the list back, then focus it.
            if self.picking {
                self.picker_focus.focus(window, cx);
            } else {
                // Shown for this visit only: the saved collapsed state is the owner's choice and
                // must survive (this also runs at launch, before the panel width is known).
                self.overlay = true;
                cx.defer_in(window, |this, window, cx| this.list_focus.focus(window, cx));
            }
            cx.notify();
            return;
        }
        // Coming back to an open session: the keyboard is in its composer, so the first
        // sentence typed is not lost on the list.
        if !self.picking && self.saved.selected.is_some() {
            self.composer.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.list_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn narrow(&self) -> bool {
        let w = self.width.get();
        w > 0. && w < NARROW
    }

    /// Is the list drawn? Wide panel: unless collapsed (or shown for this visit, `focus_primary`).
    /// Narrow panel: only while opened over the chat.
    pub(crate) fn list_shown(&self) -> bool {
        if self.narrow() { self.overlay } else { !self.saved.collapsed || self.overlay }
    }

    /// Show or hide the list. Hiding while the filter or list has the keyboard moves it to the
    /// composer (or the panel root with no session open); showing leaves the keyboard where it is.
    pub(crate) fn set_list_shown(&mut self, show: bool, window: &mut Window, cx: &mut Context<Self>) {
        if show == self.list_shown() {
            return;
        }
        if show {
            if self.narrow() { self.overlay = true } else { self.saved.collapsed = false }
        } else {
            let in_list = self.list_focus.is_focused(window) || self.filter.read(cx).focus_handle(cx).is_focused(window);
            // Keep the scroll position the list had, for when it returns.
            self.saved.scroll = f32::from(self.list_scroll.offset().y);
            self.overlay = false;
            if !self.narrow() { self.saved.collapsed = true }
            if in_list {
                if self.picking {
                    self.picker_focus.focus(window, cx);
                } else if self.saved.selected.is_some() {
                    self.composer.update(cx, |input, cx| input.focus(window, cx));
                } else {
                    self.focus.focus(window, cx);
                }
            }
        }
        self.persist();
        cx.notify();
    }

    pub(crate) fn toggle_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_list_shown(!self.list_shown(), window, cx);
    }

    /// Put the keyboard highlight on the open session's row (rows reorder as sessions change).
    pub(crate) fn cursor_to_selected(&mut self) {
        let Some(key) = self.saved.selected.clone() else { return };
        if let Some(ix) = self.rows().iter().position(|r| matches!(r, crate::list::ListRow::Session(e) if e.key() == key)) {
            self.cursor = ix;
        }
    }

    pub(crate) fn backend_name(&self, id: &str) -> String {
        self.slots
            .iter()
            .find(|s| s.config.id == id)
            .map_or_else(|| id.to_string(), |s| s.config.name.clone())
    }

    pub(crate) fn entry(&self, key: &str) -> Option<&Entry> {
        let (backend, id) = model::split_key(key)?;
        self.slots
            .iter()
            .find(|s| s.config.id == backend)?
            .entries
            .iter()
            .find(|e| e.id == id)
    }

    /// The open session's title, from the list or from what was saved.
    pub(crate) fn selected_title(&self) -> String {
        match self.saved.selected.as_deref().and_then(|k| self.entry(k)) {
            Some(entry) => model::label(entry),
            None => self.saved.selected_title.clone().unwrap_or_else(|| "Session".into()),
        }
    }

    pub(crate) fn persist(&mut self) {
        if self.scroll_restored && self.list_shown() {
            self.saved.scroll = f32::from(self.list_scroll.offset().y);
        }
        if let Err(e) = self.saved.save() {
            self.error = Some(format!("Couldn't save the Sessions state: {e:#}"));
        }
    }

    /// Save shortly after the last change (typing, scrolling), not on every key.
    pub(crate) fn persist_soon(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let _ = this.update(cx, |this, cx| {
                this.persist();
                cx.notify();
            });
        }));
    }

    fn draft_changed(&mut self, cx: &mut Context<Self>) {
        let key = self.saved.selected.clone().unwrap_or_else(|| "new".into());
        let text = self.composer.read(cx).value().to_string();
        if text.is_empty() {
            self.saved.drafts.remove(&key);
        } else {
            self.saved.drafts.insert(key, text);
        }
        self.persist_soon(cx);
    }

    /// Put the selected session's unsent text in the composer.
    pub(crate) fn load_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.saved.selected.clone().unwrap_or_else(|| "new".into());
        let text = self.saved.drafts.get(&key).cloned().unwrap_or_default();
        self.loading_draft = true;
        self.composer.update(cx, |input, cx| {
            let end = text.len();
            input.set_value(text, window, cx);
            // Caret after the draft, so typing continues it.
            input.set_selected_range(end..end, cx);
        });
        self.loading_draft = false;
    }

    /// Open `key` on the right: replay it if this run has not shown it yet.
    pub(crate) fn select(&mut self, key: String, cwd: PathBuf, title: String, window: &mut Window, cx: &mut Context<Self>) {
        self.picking = false;
        self.overlay = false;
        if self.saved.selected.as_deref() != Some(key.as_str()) {
            self.saved.selected = Some(key.clone());
            self.saved.selected_cwd = Some(cwd.clone());
            self.saved.selected_title = Some(title);
            self.persist();
            self.load_draft(window, cx);
        }
        let needs_load = self
            .transcripts
            .get(&key)
            .is_none_or(|t| !t.live && !t.replaying);
        if needs_load {
            self.load(key, cwd, window, cx);
        }
        self.transcript_scroll.scroll_to_bottom();
        self.cursor_to_selected();
        self.publish(cx);
        cx.notify();
    }

    pub(crate) fn show_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picking = true;
        // Start on the open session's backend, else the first one.
        self.pick_cursor = self
            .saved
            .selected
            .as_deref()
            .and_then(model::split_key)
            .and_then(|(b, _)| self.slot_ix(b))
            .unwrap_or(0);
        self.picker_focus.focus(window, cx);
        ados_core::overlay::make_room(cx);
        cx.notify();
    }

    /// Tell the assistant what is open: the session title and its last visible messages.
    pub(crate) fn publish(&self, cx: &mut Context<Self>) {
        let (title, location, text) = match self.saved.selected.as_deref() {
            Some(key) => {
                let backend = model::split_key(key).map(|(b, _)| self.backend_name(b)).unwrap_or_default();
                let cwd = self.saved.selected_cwd.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                let text = self.transcripts.get(key).map(|t| t.tail_text(60_000)).unwrap_or_default();
                (self.selected_title(), Some(format!("{backend} session in {cwd}")), text)
            }
            None => ("Sessions".to_string(), None, String::new()),
        };
        ContextHub::publish(
            Snapshot {
                module: ids::SESSIONS.into(),
                title: title.into(),
                location: location.map(Into::into),
                text: text.into(),
                selection: None,
            },
            cx,
        );
    }
}

impl EventEmitter<PanelEvent> for SessionsPanel {}
impl Focusable for SessionsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        // A collapsed list is not drawn: focusing its handle would leave the keyboard outside
        // the panel, where Ctrl+B (`SessionsPanel` scope) cannot reach it.
        if self.list_shown() { self.list_focus.clone() } else { self.focus.clone() }
    }
}
impl Panel for SessionsPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Sessions"
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}
impl BasePanel for SessionsPanel {
    fn panel_name(&self) -> &'static str {
        ids::SESSIONS
    }
    fn on_removed(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.persist();
    }
}
