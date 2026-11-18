//! The assistant's agent backend: a shared handle (`ados_agents::SharedBackend`, one
//! process per backend for all of aDOs), started lazily, its chat's events read from GPUI
//! tasks through a subscription to that chat's session id. A backend switch only lets go
//! of it; the registry stops the process once nobody uses it (idle delay). The open card
//! holds a lease, so the agent stays up while the card is shown.
use super::Assistant;
use crate::{newest_first, prompt_text, ContextItem};
use ados_agents::{AgentEvent, BackendConfig, Permission, SharedBackend, Sink, ToolStatus};
use gpui_kit::*;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Conn {
    /// Not started yet (it starts on the first send, chats list or chat restore).
    Idle,
    Starting,
    Ready,
    /// Start failed or the agent stopped; the sentence is shown with a Retry button.
    Failed(String),
}

/// Work to run once the backend is ready.
pub(crate) type Then = Box<dyn FnOnce(&mut Assistant, SharedBackend, &mut Window, &mut Context<Assistant>)>;

pub(super) fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

impl Assistant {
    pub(super) fn backend_config(&self) -> Option<&BackendConfig> {
        crate::pick_backend(self.saved.backend.as_deref(), &self.backends)
    }
    /// The chosen backend's display name, for labels and tooltips.
    pub(super) fn backend_name(&self) -> String {
        self.backend_config().map_or_else(|| "the assistant".into(), |b| b.name.clone())
    }

    /// Hold the chosen backend while the card is open; let go when it closes.
    pub(super) fn hold_backend(&mut self) {
        let id = self.backend_config().map(|b| b.id.clone());
        self.lease = match (self.saved.open, id) {
            (true, Some(id)) => Some(ados_agents::shared(&id).lease()),
            _ => None,
        };
    }

    /// Send the chat's events and questions to this card (once per session).
    fn subscribe(&mut self, session: &str) {
        if let (Some(backend), Some(sink)) = (&self.backend, &self.sink) {
            self.subscription = Some((session.to_string(), backend.subscribe(session, sink.clone())));
        }
    }

    /// Run `then` with the running backend, starting it first if needed.
    pub(super) fn with_backend(&mut self, then: Then, window: &mut Window, cx: &mut Context<Self>) {
        match (&self.conn, self.backend.clone()) {
            (Conn::Ready, Some(backend)) => then(self, backend, window, cx),
            (Conn::Starting, _) => self.waiting.push(then),
            _ => {
                self.waiting.push(then);
                self.start_backend(window, cx);
            }
        }
    }

    /// Retry after a start error: start the backend again and send the prompt it gave back.
    pub(super) fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(config) = self.backend_config() {
            ados_agents::shared(&config.id).retry();
        }
        self.conn = Conn::Idle;
        self.restore_chat(window, cx);
        if !self.input.read(cx).value().trim().is_empty() {
            self.send_prompt(window, cx);
        } else if matches!(self.conn, Conn::Idle) {
            self.start_backend(window, cx);
        }
        cx.notify();
    }

    pub(super) fn start_backend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.backend_config().cloned() else {
            self.fail("No agent backends are listed in backends.toml.".into(), window, cx);
            return;
        };
        let handle = ados_agents::shared(&config.id);
        if matches!(self.conn, Conn::Failed(_)) {
            handle.retry(); // The owner asked again (Retry or a send): start it once more.
        }
        self.conn = Conn::Starting;
        self.error = None;
        let generation = self.generation;
        let (sink, events, permissions) = Sink::new();
        self.sink = Some(sink);
        self.subscription = None;
        let task = cx.spawn_in(window, async move |this, cx| {
            // A missing command reads as "not installed"; other failures keep their own words.
            let result = handle
                .start()
                .await
                .map(|_| handle)
                .map_err(|e| anyhow::anyhow!(crate::start_error(&config.name, &config.command, &e.to_string())));
            let _ = this.update_in(cx, |this, window, cx| {
                this.started(generation, result, events, permissions, window, cx)
            });
        });
        self.backend_tasks.push(task);
        cx.notify();
    }

    fn started(
        &mut self,
        generation: u64,
        result: anyhow::Result<SharedBackend>,
        events: async_channel::Receiver<AgentEvent>,
        permissions: async_channel::Receiver<Permission>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return; // The user switched backends meanwhile.
        }
        let backend = match result {
            Ok(backend) => backend,
            Err(e) => return self.fail(e.to_string(), window, cx),
        };
        self.backend = Some(backend.clone());
        self.conn = Conn::Ready;
        self.hold_backend();
        self.backend_tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this.update_in(cx, |this, window, cx| this.agent_event(event, window, cx)).is_err() {
                    break;
                }
            }
        }));
        self.backend_tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Ok((request, reply)) = permissions.recv().await {
                if this.update_in(cx, |this, _, cx| this.permission(request, reply, cx)).is_err() {
                    break;
                }
            }
        }));
        for then in std::mem::take(&mut self.waiting) {
            then(self, backend.clone(), window, cx);
        }
        if backend.info().is_some_and(|i| i.can_list_sessions) {
            self.list_chats(window, cx); // Titles for the header and the chats menu.
        }
        cx.notify();
    }

    /// Show `message` with Retry, drop waiting work and give an unsent prompt back to the composer.
    fn fail(&mut self, message: String, window: &mut Window, cx: &mut Context<Self>) {
        self.conn = Conn::Failed(message);
        self.waiting.clear();
        self.queued = None;
        self.replaying = false;
        self.backend = None;
        self.live_session = None;
        self.subscription = None;
        self.give_back_prompt(window, cx);
        cx.notify();
    }

    fn give_back_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.running = false;
        if let Some(text) = self.unsent.take() {
            if self.rows.last().is_some_and(|row| row.user && row.text == text) {
                self.rows.pop();
            }
            if self.input.read(cx).value().trim().is_empty() {
                self.input.update(cx, |input, cx| input.set_value(text, window, cx));
                self.save_draft(cx);
            }
        }
    }

    /// Let go of the backend (the registry stops it once nobody uses it) and forget
    /// everything tied to it here.
    pub(super) fn drop_backend(&mut self) {
        self.subscription = None;
        self.sink = None;
        self.lease = None;
        self.deny_all();
        self.generation += 1;
        self.turn = None;
        self.backend_tasks.clear();
        self.waiting.clear();
        self.queued = None;
        self.backend = None;
        self.live_session = None;
        self.conn = Conn::Idle;
        self.running = false;
        self.replaying = false;
    }

    /// Make sure the saved chat is the agent's open session (load it, or create a new
    /// one in the home folder), then run `then` with its id.
    /// A saved chat the agent cannot load is forgotten (logged, not shown); with
    /// `fresh_if_lost` a new chat is started for `then`, otherwise `then` is dropped.
    fn with_session<F>(&mut self, then: F, fresh_if_lost: bool, window: &mut Window, cx: &mut Context<Self>)
    where
        F: FnOnce(&mut Self, SharedBackend, String, &mut Window, &mut Context<Self>) + 'static,
    {
        self.with_session_inner(then, fresh_if_lost, window, cx)
    }

    fn with_session_inner<F>(
        &mut self,
        then: F,
        fresh_if_lost: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(&mut Self, SharedBackend, String, &mut Window, &mut Context<Self>) + 'static,
    {
        self.with_backend(
            Box::new(move |this, backend, window, cx| {
                let wanted = this.saved.chat.clone();
                if wanted.is_some() && wanted == this.live_session {
                    let id = wanted.unwrap_or_default();
                    this.replaying = false;
                    return then(this, backend, id, window, cx);
                }
                let generation = this.generation;
                let cwd = this.saved.chat_cwd.clone().unwrap_or_else(home);
                if let Some(id) = &wanted {
                    this.reset_reading(cx);
                    this.replaying = true;
                    // Subscribe first: the replay arrives before the load resolves.
                    this.subscribe(id);
                }
                let task = cx.spawn_in(window, async move |view, cx| {
                    let result = match &wanted {
                        Some(id) => backend.load_session(id, &cwd, &ados_agents::ados_servers()).await.map(|()| id.clone()),
                        None => backend.new_session(&cwd, &ados_agents::ados_servers()).await,
                    };
                    let _ = view.update_in(cx, |this, window, cx| {
                        if generation != this.generation {
                            return;
                        }
                        this.replaying = false;
                        this.streaming_row = None;
                        match (result, wanted) {
                            (Ok(id), _) => {
                                this.session_opened(id.clone(), cwd);
                                then(this, backend, id, window, cx);
                            }
                            (Err(e), Some(lost)) => {
                                // Typically a chat saved by an older client (pi-mobile) that the
                                // agent does not know. Not the user's problem: log and move on.
                                eprintln!("aDOs assistant: forgetting saved chat {lost}: {e}");
                                this.forget_chat(&lost);
                                if fresh_if_lost {
                                    this.with_session_inner(then, false, window, cx);
                                }
                            }
                            (Err(e), None) => {
                                // Same layout as a backend start failure: a sentence with Retry.
                                eprintln!("aDOs assistant: session/new failed: {e:#}");
                                let name = this.backend_name();
                                this.fail(crate::chat_start_error(&name, &e.to_string()), window, cx);
                            }
                        }
                        // A send made while the chat was opening goes out now.
                        if let Some((text, context)) = this.queued.take() {
                            this.send_text(text, context, window, cx);
                        }
                        cx.notify();
                    });
                });
                this.backend_tasks.push(task);
            }),
            window,
            cx,
        );
    }

    /// Drop a saved chat the agent cannot open; its draft becomes the new chat's draft.
    fn forget_chat(&mut self, id: &str) {
        if let Some(draft) = self.saved.drafts.remove(id) {
            self.saved.drafts.entry("new".into()).or_insert(draft);
        }
        if self.saved.chat.as_deref() == Some(id) {
            self.saved.chat = None;
            self.saved.chat_cwd = None;
        }
        self.persist();
    }

    fn session_opened(&mut self, id: String, cwd: PathBuf) {
        if self.subscription.as_ref().is_none_or(|(s, _)| *s != id) {
            self.subscribe(&id);
        }
        if self.saved.chat.is_none() {
            if let Some(draft) = self.saved.drafts.remove("new") {
                self.saved.drafts.insert(id.clone(), draft);
            }
        }
        self.saved.chat = Some(id.clone());
        self.saved.chat_cwd = Some(cwd);
        self.live_session = Some(id);
        self.persist();
    }

    /// Reopen the saved chat when the card opens with nothing shown yet.
    pub(super) fn restore_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saved.chat.is_some()
            && self.rows.is_empty()
            && self.live_session != self.saved.chat
            && !self.replaying
            && !matches!(self.conn, Conn::Failed(_))
        {
            // Set now, not when the load starts, so reopening the card meanwhile does not queue it twice.
            self.replaying = true;
            self.with_session(|_, _, _, _, _| {}, false, window, cx);
        }
    }

    pub(super) fn send_text(&mut self, text: String, context: Vec<ContextItem>, window: &mut Window, cx: &mut Context<Self>) {
        self.unsent = Some(text.clone());
        self.running = true;
        let prompt = prompt_text(&text, &context);
        self.with_session(
            move |this, backend, session, window, cx| {
                // A new chat's first prompt carries the aDOs preamble (tools, preferences).
                // The row for this send is already there, so a new chat has exactly one user row.
                let first = this.rows.iter().filter(|row| row.user).count() <= 1;
                let prompt = if first { crate::with_preamble(&prompt) } else { prompt };
                // Reopening the chat replayed its history over the row added on send.
                if !this.rows.last().is_some_and(|row| row.user && row.text == text) {
                    this.append_row(true, text.clone(), cx);
                }
                this.unsent = None;
                this.running = true;
                let generation = this.generation;
                this.turn = Some(cx.spawn_in(window, async move |view, cx| {
                    // The transcript (sessions_read) gets what the owner typed, not the preamble.
                    let result = backend.prompt_as(&session, &prompt, &text).await;
                    let _ = view.update_in(cx, |this, _, cx| {
                        if generation != this.generation {
                            return;
                        }
                        this.running = false;
                        this.streaming_row = None;
                        this.ready = !this.saved.open;
                        if let Err(e) = result {
                            this.error = Some(e.to_string());
                        }
                        cx.notify();
                    });
                }));
            },
            true,
            window,
            cx,
        );
    }

    pub(super) fn stop_turn(&mut self) {
        if let (Some(backend), Some(session)) = (&self.backend, &self.live_session) {
            if let Err(e) = backend.cancel(session) {
                self.error = Some(e.to_string());
            }
        }
    }

    /// Fetch the chosen backend's sessions (first page, newest first).
    pub(super) fn list_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chats_loading = true;
        self.with_backend(
            Box::new(|this, backend, window, cx| {
                let generation = this.generation;
                let task = cx.spawn_in(window, async move |view, cx| {
                    let result = backend.list_sessions(None, None).await;
                    let _ = view.update_in(cx, |this, _, cx| {
                        if generation != this.generation {
                            return;
                        }
                        this.chats_loading = false;
                        match result {
                            Ok((page, _)) => this.chats = newest_first(page),
                            Err(e) => this.error = Some(e.to_string()),
                        }
                        cx.notify();
                    });
                });
                this.backend_tasks.push(task);
            }),
            window,
            cx,
        );
    }

    pub(super) fn switch_backend(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.backend_menu = false;
        if self.saved.backend.as_deref() == Some(id.as_str()) {
            cx.notify();
            return;
        }
        self.drop_backend();
        self.saved.backend = Some(id);
        self.hold_backend();
        self.chats.clear();
        self.error = None;
        self.notice = None;
        self.new_chat(window, cx);
    }

    fn agent_event(&mut self, event: AgentEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ours = |session: &str| self.saved.chat.as_deref() == Some(session);
        match event {
            AgentEvent::Text { session_id, text } if ours(&session_id) => self.text_delta(0, &text, cx),
            AgentEvent::Thought { session_id, text } if ours(&session_id) => self.thought_delta(&text, cx),
            AgentEvent::UserText { session_id, text } if ours(&session_id) && self.replaying => {
                self.user_chunk(&text, cx)
            }
            AgentEvent::ToolCall { session_id, id, title, kind, status } if ours(&session_id) => {
                self.tool_start(id.clone(), kind, title);
                if let Some(ok) = finished(status) {
                    self.tool_end(id, ok, String::new());
                }
            }
            AgentEvent::ToolCallUpdate { session_id, id, title, status } if ours(&session_id) => {
                if let Some(title) = title {
                    self.tool_title(&id, &title);
                }
                if let Some(ok) = status.and_then(finished) {
                    self.tool_end(id, ok, String::new());
                }
            }
            AgentEvent::Crashed { message } => {
                // Not `drop_backend`: this runs inside one of the tasks it would drop.
                self.subscription = None;
                self.deny_all();
                self.generation += 1;
                self.turn = None;
                self.backend = None;
                self.live_session = None;
                self.running = false;
                self.replaying = false;
                self.conn = Conn::Failed(message);
            }
            _ => {}
        }
        cx.notify();
    }
}

fn finished(status: ToolStatus) -> Option<bool> {
    match status {
        ToolStatus::Completed => Some(true),
        ToolStatus::Failed => Some(false),
        ToolStatus::Pending | ToolStatus::InProgress => None,
    }
}
