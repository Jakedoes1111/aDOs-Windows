//! One `Slot` per backend in `backends.toml`: its shared backend handle (one process per
//! backend for all of aDOs, see `ados_agents::registry`), its listed sessions and
//! `session/list` cursor, and the GPUI tasks that read the events and permission requests
//! of the sessions this panel has open (subscribed by session id).
//!
//! The panel holds no lease of its own: each call does, and a turn or an open question
//! keeps the backend alive. So a backend the panel only listed stops after the idle delay,
//! and the next send starts it again (the registry reopens the session).
use crate::model::{self, Entry, Transcript};
use crate::panel::SessionsPanel;
use crate::questions::PendingQuestion;
use ados_agents::{AgentEvent, BackendConfig, Permission, PermissionRequest, SharedBackend, Sink, Subscription};
use futures::channel::oneshot;
use gpui_kit::*;
use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) enum Conn {
    Idle,
    Starting,
    Ready(SharedBackend),
    /// The start failed or the agent stopped: the sentence is shown with Retry.
    Failed(String),
}

type Then = Box<dyn FnOnce(&mut SessionsPanel, SharedBackend, &mut Window, &mut Context<SessionsPanel>)>;

pub(crate) struct Slot {
    pub config: BackendConfig,
    pub handle: SharedBackend,
    pub conn: Conn,
    /// Where this panel's sessions on the backend send events and questions.
    sink: Option<Sink>,
    subs: HashMap<String, Subscription>,
    /// The first page was asked for (only once the backend's rows are visible).
    pub requested: bool,
    pub entries: Vec<Entry>,
    /// The next `session/list` page, if the agent said there is one.
    pub cursor: Option<String>,
    pub listing: bool,
    /// Listing failed (the agent runs, but cannot list): shown on the backend's row.
    pub list_error: Option<String>,
    /// Bumped whenever the agent is replaced, so late results from the old one are dropped.
    generation: u64,
    waiting: Vec<Then>,
    tasks: Vec<Task<()>>,
}

impl Slot {
    pub fn new(config: BackendConfig) -> Self {
        Self {
            handle: ados_agents::shared(&config.id),
            config,
            conn: Conn::Idle,
            sink: None,
            subs: HashMap::new(),
            requested: false,
            entries: vec![],
            cursor: None,
            listing: false,
            list_error: None,
            generation: 0,
            waiting: vec![],
            tasks: vec![],
        }
    }
}

pub(crate) fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

impl SessionsPanel {
    /// Backends whose rows are visible (the backend filter allows them) and not listed yet.
    pub(crate) fn unlisted_visible(&self) -> Vec<usize> {
        let only = self.saved.backend.as_deref();
        (0..self.slots.len())
            .filter(|&ix| !self.slots[ix].requested && only.is_none_or(|b| b == self.slots[ix].config.id))
            .collect()
    }

    /// While the panel is drawn: list the first page of each backend whose rows are visible
    /// (a backend filtered out is not started), and the first time, reopen the session that
    /// was open last time.
    pub(crate) fn start_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for ix in self.unlisted_visible() {
            self.slots[ix].requested = true;
            self.list_page(ix, window, cx);
        }
        if self.started {
            return;
        }
        self.started = true;
        if let (Some(key), Some(cwd)) = (self.saved.selected.clone(), self.saved.selected_cwd.clone()) {
            let title = self.selected_title();
            self.select(key, cwd, title, window, cx);
        }
    }

    /// Send `session`'s events and questions to this panel.
    fn subscribe(&mut self, ix: usize, session: &str) {
        let slot = &mut self.slots[ix];
        if let (Some(sink), false) = (&slot.sink, slot.subs.contains_key(session)) {
            let sub = slot.handle.subscribe(session, sink.clone());
            slot.subs.insert(session.to_string(), sub);
        }
    }

    pub(crate) fn slot_ix(&self, backend: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.config.id == backend)
    }

    /// Run `then` with the slot's running agent, starting it first if needed.
    fn with_backend(&mut self, ix: usize, then: Then, window: &mut Window, cx: &mut Context<Self>) {
        let slot = &mut self.slots[ix];
        match &slot.conn {
            Conn::Ready(backend) => {
                let backend = backend.clone();
                then(self, backend, window, cx)
            }
            Conn::Starting => slot.waiting.push(then),
            Conn::Idle | Conn::Failed(_) => {
                slot.waiting.push(then);
                self.start(ix, window, cx);
            }
        }
    }

    fn start(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let slot = &mut self.slots[ix];
        slot.conn = Conn::Starting;
        slot.generation += 1;
        let generation = slot.generation;
        let config = slot.config.clone();
        let handle = slot.handle.clone();
        let (sink, events, permissions) = Sink::new();
        slot.sink = Some(sink);
        slot.subs.clear();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = handle
                .start()
                .await
                .map(|_| handle)
                .map_err(|e| model::start_error(&config.name, &config.command, &e.to_string()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.started(ix, generation, result, events, permissions, window, cx)
            });
        });
        slot.tasks.push(task);
        cx.notify();
    }

    fn started(
        &mut self,
        ix: usize,
        generation: u64,
        result: Result<SharedBackend, String>,
        events: async_channel::Receiver<AgentEvent>,
        permissions: async_channel::Receiver<Permission>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.slots[ix].generation != generation {
            return; // Replaced meanwhile.
        }
        let backend = match result {
            Ok(backend) => backend,
            Err(message) => return self.fail(ix, message, cx),
        };
        let event_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this.update_in(cx, |this, _, cx| this.agent_event(ix, event, cx)).is_err() {
                    break;
                }
            }
        });
        let permission_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok((request, reply)) = permissions.recv().await {
                if this.update(cx, |this, cx| this.ask(ix, request, reply, cx)).is_err() {
                    break;
                }
            }
        });
        let slot = &mut self.slots[ix];
        slot.conn = Conn::Ready(backend.clone());
        slot.tasks.push(event_task);
        slot.tasks.push(permission_task);
        for then in std::mem::take(&mut slot.waiting) {
            then(self, backend.clone(), window, cx);
        }
        cx.notify();
    }

    /// The backend failed to start or stopped: show why, forget what was tied to it.
    fn fail(&mut self, ix: usize, message: String, cx: &mut Context<Self>) {
        let id = self.slots[ix].config.id.clone();
        let prefix = format!("{id}/");
        let slot = &mut self.slots[ix];
        slot.conn = Conn::Failed(message.clone());
        slot.waiting.clear();
        slot.listing = false;
        slot.generation += 1;
        slot.subs.clear();
        self.running.retain(|key, _| !key.starts_with(&prefix));
        self.deny_where(|key| key.starts_with(&prefix));
        for (key, t) in self.transcripts.iter_mut() {
            if key.starts_with(&prefix) {
                t.live = false;
                if t.replaying {
                    t.replaying = false;
                    t.error = Some(message.clone());
                }
            }
        }
        cx.notify();
    }

    /// Retry: drop whatever is left of the agent and start again from the first page.
    pub(crate) fn retry(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let slot = &mut self.slots[ix];
        slot.handle.retry();
        slot.tasks.clear();
        slot.subs.clear();
        slot.conn = Conn::Idle;
        slot.entries.clear();
        slot.cursor = None;
        slot.list_error = None;
        self.list_page(ix, window, cx);
        if let (Some(key), Some(cwd)) = (self.saved.selected.clone(), self.saved.selected_cwd.clone()) {
            if key.starts_with(&format!("{}/", self.slots[ix].config.id)) {
                let title = self.selected_title();
                self.transcripts.remove(&key);
                self.select(key, cwd, title, window, cx);
            }
        }
    }

    /// Fetch the slot's next `session/list` page (the first one when there is no cursor).
    pub(crate) fn list_page(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.slots[ix].listing {
            return;
        }
        self.slots[ix].listing = true;
        let cursor = self.slots[ix].cursor.clone();
        self.with_backend(
            ix,
            Box::new(move |this, backend, window, cx| {
                let generation = this.slots[ix].generation;
                let task = cx.spawn_in(window, async move |view, cx| {
                    let result = backend.list_sessions(None, cursor).await;
                    let _ = view.update(cx, |this, cx| {
                        let slot = &mut this.slots[ix];
                        if slot.generation != generation {
                            return;
                        }
                        slot.listing = false;
                        match result {
                            Ok((page, next)) => {
                                let backend = slot.config.id.clone();
                                for s in page {
                                    if slot.entries.iter().any(|e| e.id == s.id) {
                                        continue;
                                    }
                                    slot.entries.push(Entry {
                                        backend: backend.clone(),
                                        id: s.id,
                                        cwd: s.cwd,
                                        title: s.title,
                                        updated_at: s.updated_at,
                                    });
                                }
                                slot.cursor = next;
                            }
                            Err(e) => slot.list_error = Some(e.to_string()),
                        }
                        cx.notify();
                    });
                });
                this.slots[ix].tasks.push(task);
            }),
            window,
            cx,
        );
    }

    /// `session/load` replays `key` into its transcript.
    pub(crate) fn load(&mut self, key: String, cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some((backend_id, id)) = model::split_key(&key).map(|(b, i)| (b.to_string(), i.to_string())) else {
            return;
        };
        let Some(ix) = self.slot_ix(&backend_id) else {
            let t = self.transcripts.entry(key).or_default();
            t.error = Some(format!("The backend “{backend_id}” is no longer in backends.toml."));
            return;
        };
        self.transcripts.insert(key.clone(), Transcript { replaying: true, ..Default::default() });
        let name = self.slots[ix].config.name.clone();
        self.with_backend(
            ix,
            Box::new(move |this, backend, window, cx| {
                // Subscribe first: the replay arrives before the load resolves.
                this.subscribe(ix, &id);
                let generation = this.slots[ix].generation;
                let task = cx.spawn_in(window, async move |view, cx| {
                    let result = backend.load_session(&id, &cwd, &ados_agents::ados_servers()).await;
                    let _ = view.update(cx, |this, cx| {
                        if this.slots[ix].generation != generation {
                            return;
                        }
                        let t = this.transcripts.entry(key.clone()).or_default();
                        t.replaying = false;
                        match result {
                            Ok(()) => t.live = true,
                            Err(e) => t.error = Some(model::open_error(&name, &e.to_string())),
                        }
                        this.transcript_scroll.scroll_to_bottom();
                        this.publish(cx);
                        cx.notify();
                    });
                });
                this.slots[ix].tasks.push(task);
            }),
            window,
            cx,
        );
        cx.notify();
    }

    /// Send the composer text to the open session.
    pub(crate) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.saved.selected.clone() else { return };
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() || self.running.contains_key(&key) {
            return;
        }
        let ready = self.transcripts.get(&key).is_some_and(|t| t.live);
        let Some((backend_id, id)) = model::split_key(&key).map(|(b, i)| (b.to_string(), i.to_string())) else {
            return;
        };
        let Some(ix) = self.slot_ix(&backend_id) else { return };
        let Conn::Ready(backend) = &self.slots[ix].conn else { return };
        if !ready {
            return;
        }
        let backend = backend.clone();
        self.loading_draft = true;
        self.composer.update(cx, |input, cx| input.set_value("", window, cx));
        self.loading_draft = false;
        self.saved.drafts.remove(&key);
        self.name_untitled(&key, &text);
        self.persist();
        let t = self.transcripts.entry(key.clone()).or_default();
        t.push_user(&text);
        t.error = None;
        self.touch(&key);
        self.transcript_scroll.scroll_to_bottom();
        let generation = self.slots[ix].generation;
        let turn_key = key.clone();
        let task = cx.spawn_in(window, async move |view, cx| {
            let result = backend.prompt(&id, &text).await;
            let _ = view.update(cx, |this, cx| {
                if this.slots[ix].generation != generation {
                    return;
                }
                this.running.remove(&turn_key);
                this.deny_where(|k| k == turn_key);
                if let Err(e) = result {
                    this.transcripts.entry(turn_key.clone()).or_default().error = Some(e.to_string());
                }
                this.touch(&turn_key);
                this.publish(cx);
                cx.notify();
            });
        });
        self.running.insert(key, task);
        self.publish(cx);
        cx.notify();
    }

    /// Stop (or Esc while a turn runs): `session/cancel`; the turn then ends as cancelled.
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.saved.selected.clone() else { return };
        if !self.running.contains_key(&key) {
            return;
        }
        let Some((backend_id, id)) = model::split_key(&key) else { return };
        if let Some(Conn::Ready(backend)) = self.slot_ix(backend_id).map(|ix| &self.slots[ix].conn) {
            if let Err(e) = backend.cancel(id) {
                self.transcripts.entry(key.clone()).or_default().error = Some(e.to_string());
            }
        }
        // A cancelled turn answers its open questions too (ACP: the client replies
        // "cancelled"), or an agent waiting on one never ends the turn.
        self.deny_where(|k| k == key);
        cx.notify();
    }

    /// New session on backend `ix`, in the home folder; it opens at once.
    pub(crate) fn new_session(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.picking = false;
        self.creating = Some(ix);
        self.with_backend(
            ix,
            Box::new(move |this, backend, window, cx| {
                let generation = this.slots[ix].generation;
                let task = cx.spawn_in(window, async move |view, cx| {
                    let cwd = home();
                    let result = backend.new_session(&cwd, &ados_agents::ados_servers()).await;
                    let _ = view.update_in(cx, |this, window, cx| {
                        if this.slots[ix].generation != generation {
                            return;
                        }
                        this.creating = None;
                        match result {
                            Ok(id) => {
                                this.subscribe(ix, &id);
                                let slot = &mut this.slots[ix];
                                let entry = Entry {
                                    backend: slot.config.id.clone(),
                                    id,
                                    cwd: cwd.clone(),
                                    title: None,
                                    updated_at: Some(model::now_rfc3339()),
                                };
                                let key = entry.key();
                                let title = model::label(&entry);
                                slot.entries.push(entry);
                                this.transcripts.insert(key.clone(), Transcript { live: true, ..Default::default() });
                                this.select(key, cwd, title, window, cx);
                                this.composer.update(cx, |input, cx| input.focus(window, cx));
                            }
                            Err(e) => this.error = Some(e.to_string()),
                        }
                        cx.notify();
                    });
                });
                this.slots[ix].tasks.push(task);
            }),
            window,
            cx,
        );
        cx.notify();
    }

    /// A session without a title (a new one) is named after its first message.
    fn name_untitled(&mut self, key: &str, text: &str) {
        let Some((backend, id)) = model::split_key(key) else { return };
        let Some(ix) = self.slot_ix(backend) else { return };
        let Some(entry) = self.slots[ix].entries.iter_mut().find(|e| e.id == id) else { return };
        if entry.title.as_deref().and_then(model::first_meaningful_line).is_some() {
            return;
        }
        if let Some(title) = model::title_from_message(text) {
            entry.title = Some(title.clone());
            if self.saved.selected.as_deref() == Some(key) {
                self.saved.selected_title = Some(title);
            }
        }
    }

    /// The session changed now: it moves to the top of the list.
    fn touch(&mut self, key: &str) {
        let Some((backend, id)) = model::split_key(key) else { return };
        if let Some(ix) = self.slot_ix(backend) {
            if let Some(entry) = self.slots[ix].entries.iter_mut().find(|e| e.id == id) {
                entry.updated_at = Some(model::now_rfc3339());
            }
        }
    }

    fn agent_event(&mut self, ix: usize, event: AgentEvent, cx: &mut Context<Self>) {
        let backend = self.slots[ix].config.id.clone();
        let session = match &event {
            AgentEvent::Text { session_id, .. }
            | AgentEvent::Thought { session_id, .. }
            | AgentEvent::UserText { session_id, .. }
            | AgentEvent::ToolCall { session_id, .. }
            | AgentEvent::ToolCallUpdate { session_id, .. } => session_id.clone(),
            AgentEvent::Crashed { message } => return self.fail(ix, message.clone(), cx),
        };
        let key = model::key(&backend, &session);
        if let Some(t) = self.transcripts.get_mut(&key) {
            t.apply(&event);
            if self.saved.selected.as_deref() == Some(key.as_str()) {
                self.transcript_scroll.scroll_to_bottom();
                self.publish(cx);
            }
            cx.notify();
        }
    }

    pub(crate) fn ask(&mut self, ix: usize, request: PermissionRequest, reply: oneshot::Sender<Option<String>>, cx: &mut Context<Self>) {
        let key = model::key(&self.slots[ix].config.id, &request.session_id);
        self.questions.push(PendingQuestion { key, request, reply });
        cx.notify();
    }
}
