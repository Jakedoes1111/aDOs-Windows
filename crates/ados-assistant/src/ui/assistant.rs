use super::markdown::Row;
use super::state::Saved;
use super::agent::Conn;
use ados_agents::{BackendConfig, Lease, SessionSummary, SharedBackend, Sink};
use ados_core::{
    actions::{ActionRegistry, PaletteAction},
    overlay::OverlayGuard,
};
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Default)]
struct Views(Vec<(WindowId, WeakEntity<Assistant>)>);
impl Global for Views {}

pub fn init(cx: &mut App) {
    cx.set_global(Views::default());
    for (id, title, new_chat) in [
        ("assistant.toggle", "Ask the assistant", false),
        ("assistant.new_chat", "New assistant chat", true),
    ] {
        ActionRegistry::register(
            PaletteAction {
                id: id.into(),
                title: title.into(),
                keywords: "assistant chat agent pi claude codex".into(),
                shortcut: None,
                run: Rc::new(move |window, cx| {
                    let target = cx
                        .global::<Views>()
                        .0
                        .iter()
                        .find(|(id, _)| *id == window.window_handle().window_id())
                        .map(|(_, v)| v.clone());
                    if let Some(target) = target {
                        let _ = target.update(cx, |v, cx| {
                            if new_chat {
                                v.new_chat(window, cx);
                            } else {
                                v.toggle(window, cx);
                            }
                        });
                    }
                }),
            },
            cx,
        );
    }
}

/// "Assistant: use <backend>" for each backend in `backends.toml`: picks it and opens the card.
fn register_backend_actions(entity: &Entity<Assistant>, cx: &mut App) {
    let backends: Vec<(String, String)> =
        entity.read(cx).backends.iter().map(|b| (b.id.clone(), b.name.clone())).collect();
    for (id, name) in backends {
        let target = entity.downgrade();
        let backend = id.clone();
        ActionRegistry::register(
            PaletteAction {
                id: format!("assistant.use.{id}").into(),
                title: format!("Assistant: use {name}").into(),
                keywords: format!("assistant agent backend switch {id} {name}").into(),
                shortcut: None,
                run: Rc::new(move |window, cx| {
                    let _ = target.update(cx, |v, cx| {
                        v.switch_backend(backend.clone(), window, cx);
                        v.open(window, cx);
                    });
                }),
            },
            cx,
        );
    }
}

pub(super) struct Drag {
    pub start: Point<Pixels>,
    pub origin: (f32, f32),
    pub moved: bool,
    pub bubble: bool,
}
pub struct Assistant {
    pub(super) saved: Saved,
    pub(super) error: Option<String>,
    /// A muted line from the agent that is not part of its reply (e.g. pi's update notice).
    pub(super) notice: Option<String>,
    /// Backends from `backends.toml`, in file order.
    pub(super) backends: Vec<BackendConfig>,
    pub(super) conn: Conn,
    /// The chosen backend once started (shared with Sessions and MCP).
    pub(super) backend: Option<SharedBackend>,
    /// Where the chat's events and permission questions arrive, and its subscription.
    pub(super) sink: Option<Sink>,
    pub(super) subscription: Option<(String, ados_agents::Subscription)>,
    /// Held while the card is open, so the agent is not idle-stopped under it.
    pub(super) lease: Option<Lease>,
    /// Bumped on every backend switch so late results from the old agent are ignored.
    pub(super) generation: u64,
    /// The session the running agent has open (new or loaded), if any.
    pub(super) live_session: Option<String>,
    /// Work waiting for the backend to finish starting.
    pub(super) waiting: Vec<super::agent::Then>,
    pub(super) backend_tasks: Vec<Task<()>>,
    pub(super) turn: Option<Task<()>>,
    /// The prompt being sent while the backend starts; given back to the composer if it fails.
    pub(super) unsent: Option<String>,
    /// A send made while the saved chat was still opening; sent once it is open.
    pub(super) queued: Option<(String, Vec<crate::ContextItem>)>,
    /// True while `session/load` replays a chat (user chunks become user rows).
    pub(super) replaying: bool,
    /// The last replayed row is a user message still receiving chunks.
    pub(super) replaying_user: bool,
    pub(super) running: bool,
    pub(super) ready: bool,
    pub(super) drag: Option<Drag>,
    pub(super) input: Entity<TextareaState>,
    pub(super) chats: Vec<SessionSummary>,
    pub(super) chats_loading: bool,
    /// The highlighted row in the chats list (keyboard).
    pub(super) chat_cursor: usize,
    pub(super) chats_scroll: ScrollHandle,
    pub(super) switcher: bool,
    pub(super) backend_menu: bool,
    /// The highlighted row of the backend menu (keyboard).
    pub(super) backend_cursor: usize,
    pub(super) focus: FocusHandle,
    /// The first option of the first open question: Tab from the composer lands here.
    pub(super) question_focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    pub(super) rows: Vec<Row>,
    pub(super) reading_scroll: ScrollHandle,
    /// Last browser native-window rectangle seen, so a repeated publish does not
    /// schedule a redraw every frame.
    pub(super) last_native: Option<Bounds<Pixels>>,
    pub(super) partial: BTreeMap<usize, String>,
    pub(super) streaming_row: Option<usize>,
    pub(super) attached: Vec<SharedString>,
    pub(super) dismissed_selection: Option<String>,
    pub(super) picker: bool,
    pub(super) steps: Vec<super::steps_strip::Step>,
    pub(super) questions: Vec<super::questions::PendingQuestion>,
    subscriptions: Vec<Subscription>,
}
impl Assistant {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|cx: &mut Context<Self>| {
            let (mut saved, mut error) = Saved::load();
            let backends = match ados_agents::config::load() {
                Ok(backends) => backends,
                Err(e) => {
                    error = Some(e.to_string());
                    vec![]
                }
            };
            saved.backend = crate::pick_backend(saved.backend.as_deref(), &backends).map(|b| b.id.clone());
            let draft = saved
                .drafts
                .get(saved.chat.as_deref().unwrap_or("new"))
                .cloned()
                .unwrap_or_default();
            let input = cx.new(|cx| {
                let mut input = TextareaState::new(window, cx)
                    .auto_grow(1, 6)
                    .submit_on_enter(true)
                    .placeholder("Ask… · @ attaches a module");
                let end = draft.len();
                input.set_value(draft, window, cx);
                // Caret after the draft, so typing continues it (multi-line inputs reset to the start).
                input.set_selected_range(end..end, cx);
                input
            });
            let mut this = Self {
                saved,
                error,
                notice: None,
                backends,
                conn: Conn::Idle,
                backend: None,
                sink: None,
                subscription: None,
                lease: None,
                generation: 0,
                live_session: None,
                waiting: vec![],
                backend_tasks: vec![],
                turn: None,
                unsent: None,
                queued: None,
                replaying: false,
                replaying_user: false,
                running: false,
                ready: false,
                drag: None,
                input: input.clone(),
                chats: vec![],
                chats_loading: false,
                chat_cursor: 0,
                chats_scroll: ScrollHandle::new(),
                switcher: false,
                backend_menu: false,
                backend_cursor: 0,
                focus: cx.focus_handle(),
                question_focus: cx.focus_handle(),
                previous_focus: None,
                rows: vec![],
                reading_scroll: ScrollHandle::new(),
                last_native: None,
                partial: BTreeMap::new(),
                streaming_row: None,
                attached: vec![],
                dismissed_selection: None,
                picker: false,
                steps: vec![],
                questions: vec![],
                subscriptions: vec![],
            };
            this.subscriptions.push(cx.subscribe_in(
                &input,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::Change => {
                        this.picker = this.input.read(cx).value().ends_with('@');
                        this.save_draft(cx);
                        cx.notify();
                    }
                    InputEvent::PressEnter { shift: false, .. } => this.send_prompt(window, cx),
                    _ => {}
                },
            ));
            this.clamp(window);
            if this.saved.open {
                this.restore_chat(window, cx);
            }
            this.subscriptions
                .push(cx.observe_window_bounds(window, |this, window, cx| {
                    this.clamp(window);
                    this.persist();
                    cx.notify();
                }));
            this.subscriptions
                .push(cx.observe_global::<ados_core::hub::ContextHub>(|_, cx| cx.notify()));
            this.subscriptions.push(cx.observe_global::<OverlayGuard>(|_, cx| cx.notify()));
            // A panel opened a choice: an open card the user is not typing in steps aside.
            // Not while it asks a question, since closing the card answers "deny".
            this.subscriptions.push(cx.observe_global_in::<ados_core::overlay::MakeRoom>(window, |this, window, cx| {
                if this.is_open() && this.questions.is_empty() && !this.focus.contains_focused(window, cx) {
                    this.collapse(window, cx);
                }
            }));
            this.subscriptions
                .push(
                    cx.observe_global::<ados_core::overlay::NativeRect>(|this, cx| {
                        let current = ados_core::overlay::native_rect(cx);
                        if this.last_native != current {
                            this.last_native = current;
                            cx.notify();
                        }
                    }),
                );
            this.subscriptions.push(cx.on_release(|this, cx| {
                this.deny_all();
                ados_core::overlay::set_cutout(None, cx);
            }));
            this
        });
        if !cx.has_global::<Views>() {
            init(cx);
        }
        cx.global_mut::<Views>()
            .0
            .push((window.window_handle().window_id(), entity.downgrade()));
        register_backend_actions(&entity, cx);
        super::inbox::register(&entity, cx);
        entity
    }
    pub fn is_open(&self) -> bool {
        self.saved.open
    }
    pub(super) fn has_draft(&self) -> bool {
        self.saved
            .drafts
            .get(self.saved.chat.as_deref().unwrap_or("new"))
            .is_some_and(|draft| !draft.trim().is_empty())
    }
    /// Alt+Space: open the card; on an open card that does not have the keyboard, focus its
    /// composer; close it only from inside the card. Closing from outside would send the
    /// next typed sentence to whatever list is behind it.
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_open() && !self.focus.contains_focused(window, cx) {
            self.previous_focus = window.focused(cx);
            self.input.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        } else if self.is_open() {
            self.collapse(window, cx);
        } else {
            self.open(window, cx);
        }
    }
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clamp(window);
        if !self.saved.open {
            self.previous_focus = window.focused(cx);
            self.saved.open = true;
            self.persist();
            self.hold_backend();
        }
        self.ready = false;
        self.restore_chat(window, cx);
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    pub fn collapse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Closing the card answers every open permission question with "deny".
        self.deny_all();
        if self.saved.open {
            if self.focus.contains_focused(window, cx) {
                if let Some(focus) = self.previous_focus.take() {
                    window.focus(&focus, cx);
                } else {
                    window.blur(cx);
                }
            }
            self.saved.open = false;
            self.persist();
            self.hold_backend();
        }
        cx.notify();
    }
    pub(super) fn persist(&mut self) {
        if let Err(e) = self.saved.save() {
            self.error = Some(format!("Couldn't save assistant state: {e}"));
        }
    }
    pub(super) fn available_size(window: &Window) -> Size<Pixels> {
        // The overlay lives inside Root's client-decoration shadow padding.
        let viewport = window.viewport_size();
        let padding = gpui_kit::component::window_paddings(window);
        size(
            viewport.width - padding.left - padding.right,
            viewport.height - padding.top - padding.bottom,
        )
    }
    pub(super) fn clamp(&mut self, window: &Window) {
        let before = (self.saved.x, self.saved.anchor_x, self.saved.anchored);
        self.ensure_anchored(window);
        if before != (self.saved.x, self.saved.anchor_x, self.saved.anchored) {
            self.persist();
        }
    }
}

impl Render for Assistant {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.clamp(window);
        // The card is not an `OverlayGuard` overlay: the browser cuts the card's
        // rectangle out of its page (see `card`) instead of hiding the page. Any
        // other overlay (palette, menus) still hides the card; the bubble stays in the top bar.
        let clear = OverlayGuard::count(cx) == 0;
        let card_shown = clear && self.saved.open;
        if !card_shown {
            ados_core::overlay::set_cutout(None, cx);
        }
        div()
            .absolute()
            .inset_0()
            .child(self.drag_events(cx))
            .child(self.bubble(window, cx))
            .when(card_shown, |el| el.child(self.card(window, cx)))
    }
}

