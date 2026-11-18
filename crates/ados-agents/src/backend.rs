//! One running agent backend: its process, its ACP connection and the event stream.
//!
//! The connection lives on its own thread (`smol::block_on`), so the handle works
//! from any executor, GPUI's included. Requests go in over a channel; every
//! `session/update` comes out as an [`AgentEvent`].
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::{
    CancelNotification, ClientCapabilities, ContentBlock, FileSystemCapabilities,
    Implementation, InitializeRequest, ListSessionsRequest, LoadSessionRequest,
    NewSessionRequest, PermissionOptionKind, PromptRequest, ReadTextFileRequest,
    ReadTextFileResponse, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionNotification,
    WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client};
use anyhow::{anyhow, Result};
use futures::channel::oneshot;
use futures::FutureExt;

use crate::config::BackendConfig;
use crate::event::{self, AgentEvent, PermissionCallback, StopReason};
use crate::files::{self, SessionDirs};
use crate::mcp::{self, McpEntry};
use crate::started;

/// Where events go when a backend is started with a hook ([`Backend::start_with`]):
/// called on the backend's own thread, in wire order, before the request that
/// caused them resolves.
pub type EventHook = std::sync::Arc<dyn Fn(AgentEvent) + Send + Sync>;

/// The agent answered with ACP's `auth_required` (or said it is not logged in).
/// Callers can find it with `err.downcast_ref::<NotLoggedIn>()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotLoggedIn(pub String);

impl std::fmt::Display for NotLoggedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for NotLoggedIn {}

/// True when `err` means the agent wants its user to log in first.
pub fn is_not_logged_in(err: &anyhow::Error) -> bool {
    err.downcast_ref::<NotLoggedIn>().is_some()
}

/// Wrap an ACP error with `what` ("Could not list sessions"), keeping auth errors findable.
fn acp_error(what: &str, e: agent_client_protocol::Error) -> anyhow::Error {
    let text = format!("{what}: {e}");
    let lower = e.to_string().to_lowercase();
    let auth = e.code == agent_client_protocol::ErrorCode::AUTH_REQUIRED
        || lower.contains("not logged in")
        || lower.contains("authentication required")
        || lower.contains("please log in")
        || lower.contains("login required");
    if auth {
        anyhow::Error::new(NotLoggedIn(text))
    } else {
        anyhow!(text)
    }
}

/// What the agent said about itself in `initialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendInfo {
    /// The `backends.toml` id.
    pub id: String,
    /// The `backends.toml` display name.
    pub name: String,
    /// The agent's own name and version, when it reports them.
    pub agent: Option<String>,
    /// The agent advertises `sessionCapabilities.list`.
    pub can_list_sessions: bool,
    /// The agent advertises `loadSession`.
    pub can_load_sessions: bool,
}

/// One entry of `session/list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub cwd: PathBuf,
    pub title: Option<String>,
    pub updated_at: Option<String>,
}

type Reply<T> = oneshot::Sender<Result<T>>;

pub(crate) enum Cmd {
    New { cwd: PathBuf, mcp: Vec<McpEntry>, reply: Reply<String> },
    Prompt { session: String, text: String, reply: Reply<StopReason> },
    Cancel { session: String },
    List { cwd: Option<PathBuf>, cursor: Option<String>, reply: Reply<(Vec<SessionSummary>, Option<String>)> },
    Load { session: String, cwd: PathBuf, mcp: Vec<McpEntry>, reply: Reply<()> },
}

/// A running agent. Dropping it kills the agent's process group.
pub struct Backend {
    info: BackendInfo,
    cmds: async_channel::Sender<Cmd>,
    events: async_channel::Receiver<AgentEvent>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    /// Start the backend's process and run `initialize`. Permission requests go to
    /// `on_permission` (use [`event::deny`](crate::deny) to refuse everything).
    pub async fn start(config: &BackendConfig, on_permission: PermissionCallback) -> Result<Self> {
        Self::start_with(config, on_permission, None).await
    }

    /// Like [`start`](Self::start), but every event (crashes included) goes to `on_event`
    /// instead of [`events`](Self::events). The shared [`Registry`](crate::Registry) uses this.
    pub async fn start_with(
        config: &BackendConfig,
        on_permission: PermissionCallback,
        on_event: Option<EventHook>,
    ) -> Result<Self> {
        let agent = AcpAgent::new(
            AcpAgentConfig::new(crate::launch::program(&config.command))
                .args(config.args.iter().cloned())
                .envs(config.env.iter().map(|(k, v)| (k.clone(), v.clone()))),
        );
        let (cmd_tx, cmd_rx) = async_channel::unbounded();
        let (event_tx, event_rx) = async_channel::unbounded();
        let sink = Sink { channel: event_tx, hook: on_event };
        let (ready_tx, ready_rx) = oneshot::channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let (name, command) = (config.name.clone(), config.command.clone());
        let thread = std::thread::Builder::new()
            .name(format!("ados-agent-{}", config.id))
            .spawn(move || {
                smol::block_on(supervise(agent, (name, command), on_permission, sink, cmd_rx, ready_tx, stop_rx))
            })
            .map_err(|e| anyhow!("Could not start a thread for {}: {e}", config.name))?;
        let mut backend = Backend {
            info: BackendInfo {
                id: config.id.clone(),
                name: config.name.clone(),
                agent: None,
                can_list_sessions: false,
                can_load_sessions: false,
            },
            cmds: cmd_tx,
            events: event_rx,
            stop: Some(stop_tx),
            thread: Some(thread),
        };
        let ready = ready_rx
            .await
            .map_err(|_| anyhow!("{} stopped before it finished starting.", config.name))??;
        backend.info.agent = ready.agent;
        backend.info.can_list_sessions = ready.can_list_sessions;
        backend.info.can_load_sessions = ready.can_load_sessions;
        Ok(backend)
    }

    pub fn info(&self) -> &BackendInfo {
        &self.info
    }

    /// The event stream. Clones share one queue: each event goes to one receiver.
    pub fn events(&self) -> async_channel::Receiver<AgentEvent> {
        self.events.clone()
    }

    async fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Cmd) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        self.cmds
            .send(make(tx))
            .await
            .map_err(|_| anyhow!("{} is not running.", self.info.name))?;
        rx.await.map_err(|_| anyhow!("{} stopped before it answered.", self.info.name))?
    }

    /// `session/new` in `cwd`, handing the agent `mcp` servers. Returns the session id.
    pub async fn new_session(&self, cwd: &Path, mcp: &[McpEntry]) -> Result<String> {
        let cwd = absolute(cwd)?;
        let mcp = mcp.to_vec();
        self.ask(|reply| Cmd::New { cwd, mcp, reply }).await
    }

    /// `session/prompt`: resolves when the turn ends. Updates arrive as events meanwhile.
    pub async fn prompt(&self, session_id: &str, text: &str) -> Result<StopReason> {
        let (session, text) = (session_id.to_string(), text.to_string());
        self.ask(|reply| Cmd::Prompt { session, text, reply }).await
    }

    /// `session/cancel`: the running prompt then ends with [`StopReason::Cancelled`].
    pub fn cancel(&self, session_id: &str) -> Result<()> {
        self.cmds
            .try_send(Cmd::Cancel { session: session_id.to_string() })
            .map_err(|_| anyhow!("{} is not running.", self.info.name))
    }

    /// `session/list`, one page. Pass the returned cursor back for the next page.
    pub async fn list_sessions(
        &self,
        cwd: Option<&Path>,
        cursor: Option<String>,
    ) -> Result<(Vec<SessionSummary>, Option<String>)> {
        if !self.info.can_list_sessions {
            return Err(anyhow!("{} cannot list its sessions.", self.info.name));
        }
        let cwd = cwd.map(absolute).transpose()?;
        self.ask(|reply| Cmd::List { cwd, cursor, reply }).await
    }

    /// `session/load`: the agent replays the session as events, then this resolves.
    /// `mcp` servers are handed to the agent as on `session/new`.
    pub async fn load_session(&self, session_id: &str, cwd: &Path, mcp: &[McpEntry]) -> Result<()> {
        if !self.info.can_load_sessions {
            return Err(anyhow!("{} cannot reopen earlier sessions.", self.info.name));
        }
        let (session, cwd) = (session_id.to_string(), absolute(cwd)?);
        let mcp = mcp.to_vec();
        self.ask(|reply| Cmd::Load { session, cwd, mcp, reply }).await
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        // Dropping the connection future kills the agent's process group (SIGKILL).
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let here = std::env::current_dir().map_err(|e| anyhow!("Could not read the current folder: {e}"))?;
    Ok(here.join(path))
}

/// Where the backend thread sends events: the channel, or the hook when one is set.
#[derive(Clone)]
struct Sink {
    channel: async_channel::Sender<AgentEvent>,
    hook: Option<EventHook>,
}

impl Sink {
    fn send(&self, event: AgentEvent) {
        match &self.hook {
            Some(hook) => hook(event),
            None => {
                let _ = self.channel.try_send(event);
            }
        }
    }
}

struct Ready {
    agent: Option<String>,
    can_list_sessions: bool,
    can_load_sessions: bool,
}

/// Runs on the backend thread until stopped or the agent exits.
async fn supervise(
    agent: AcpAgent,
    (name, command): (String, String),
    on_permission: PermissionCallback,
    events: Sink,
    cmds: async_channel::Receiver<Cmd>,
    ready: oneshot::Sender<Result<Ready>>,
    stop: oneshot::Receiver<()>,
) {
    let ready_slot = std::sync::Arc::new(std::sync::Mutex::new(Some(ready)));
    let connection = connect(agent, on_permission, events.clone(), cmds, ready_slot.clone());
    futures::select! {
        result = connection.fuse() => {
            let reason = match result {
                Ok(()) => "it exited.".to_string(),
                Err(e) => e.to_string(),
            };
            // If it died during startup, `start` reports it; otherwise it is a crash event.
            let pending = ready_slot.lock().ok().and_then(|mut s| s.take());
            match pending {
                Some(ready) => {
                    let _ = ready.send(Err(anyhow!("Could not start {name} ({command}): {reason}")));
                }
                None => {
                    let message = format!("{name} stopped: {reason}");
                    events.send(AgentEvent::Crashed { message });
                }
            }
        }
        _ = stop.fuse() => {}
    }
}

type ReadySlot = std::sync::Arc<std::sync::Mutex<Option<oneshot::Sender<Result<Ready>>>>>;

async fn connect(
    agent: AcpAgent,
    on_permission: PermissionCallback,
    events: Sink,
    cmds: async_channel::Receiver<Cmd>,
    ready: ReadySlot,
) -> agent_client_protocol::Result<()> {
    let dirs = SessionDirs::default();
    let (read_dirs, write_dirs, cmd_dirs) = (dirs.clone(), dirs.clone(), dirs);
    Client
        .v1()
        .name("ados")
        .on_receive_notification(
            async |n: SessionNotification, _cx| {
                if let Some(e) = event::from_notification(&n) {
                    started::record(&e);
                    events.send(e);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async |req: RequestPermissionRequest, responder, cx: ConnectionTo<Agent>| {
                // Asking the user can take minutes: answer from a task, not the dispatch loop.
                let ask = on_permission.clone();
                cx.spawn(async move {
                    let chosen = ask(event::permission_request(&req)).await;
                    responder.respond(RequestPermissionResponse::new(outcome(&req, chosen)))
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |req: ReadTextFileRequest, responder, _cx| {
                let result = read_dirs
                    .resolve(&req.session_id.0, &req.path)
                    .and_then(|p| files::read(&p, req.line, req.limit));
                match result {
                    Ok(text) => responder.respond(ReadTextFileResponse::new(text)),
                    Err(msg) => responder.respond_with_internal_error(msg),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |req: WriteTextFileRequest, responder, _cx| {
                let result = write_dirs
                    .resolve(&req.session_id.0, &req.path)
                    .and_then(|p| files::write(&p, &req.content));
                match result {
                    Ok(()) => responder.respond(WriteTextFileResponse::new()),
                    Err(msg) => responder.respond_with_internal_error(msg),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, async |cx: ConnectionTo<Agent>| {
            let init = cx
                .send_request(
                    InitializeRequest::new(ProtocolVersion::V1)
                        .client_capabilities(
                            ClientCapabilities::new().fs(
                                FileSystemCapabilities::new()
                                    .read_text_file(true)
                                    .write_text_file(true),
                            ),
                        )
                        .client_info(Implementation::new("ados", env!("CARGO_PKG_VERSION"))),
                )
                .block_task()
                .await;
            let init = match init {
                Ok(init) => init,
                Err(e) => {
                    if let Some(tx) = ready.lock().ok().and_then(|mut s| s.take()) {
                        let _ = tx.send(Err(anyhow!("The agent did not start: {e}")));
                    }
                    return Ok(());
                }
            };
            let caps = &init.agent_capabilities;
            let info = Ready {
                agent: init.agent_info.as_ref().map(|i| format!("{} {}", i.name, i.version)),
                can_list_sessions: caps.session_capabilities.list.is_some(),
                can_load_sessions: caps.load_session,
            };
            if let Some(tx) = ready.lock().ok().and_then(|mut s| s.take()) {
                let _ = tx.send(Ok(info));
            }
            loop {
                let cmd = futures::select! {
                    cmd = cmds.recv().fuse() => match cmd { Ok(c) => c, Err(_) => return Ok(()) },
                    _ = cx.incoming_closed().fuse() => return Ok(()),
                };
                handle(&cx, &cmd_dirs, cmd)?;
            }
        })
        .await
}

/// Send one command. Requests run in their own task so a long prompt never blocks a cancel.
fn handle(cx: &ConnectionTo<Agent>, dirs: &SessionDirs, cmd: Cmd) -> agent_client_protocol::Result<()> {
    let cx2 = cx.clone();
    match cmd {
        Cmd::Cancel { session } => cx.send_notification(CancelNotification::new(session)),
        Cmd::New { cwd, mcp, reply } => {
            let dirs = dirs.clone();
            cx.spawn(async move {
                let req = NewSessionRequest::new(cwd.clone()).mcp_servers(mcp::to_acp(&mcp));
                let r = cx2.send_request(req).block_task().await;
                let r = r.map(|resp| {
                    let id = resp.session_id.0.to_string();
                    dirs.insert(&id, &cwd);
                    id
                });
                let _ = reply.send(r.map_err(|e| acp_error("Could not start a session", e)));
                Ok(())
            })
        }
        Cmd::Prompt { session, text, reply } => cx.spawn(async move {
            let req = PromptRequest::new(session, vec![ContentBlock::from(text)]);
            let r = cx2.send_request(req).block_task().await;
            let _ = reply.send(
                r.map(|resp| event::stop_reason(resp.stop_reason))
                    .map_err(|e| acp_error("The prompt failed", e)),
            );
            Ok(())
        }),
        Cmd::List { cwd, cursor, reply } => cx.spawn(async move {
            let req = ListSessionsRequest::new().cwd(cwd).cursor(cursor);
            let r = cx2.send_request(req).block_task().await.map(|resp| {
                let list = resp
                    .sessions
                    .into_iter()
                    .map(|s| SessionSummary {
                        id: s.session_id.0.to_string(),
                        cwd: s.cwd,
                        title: s.title,
                        updated_at: s.updated_at,
                    })
                    .collect();
                (list, resp.next_cursor)
            });
            let _ = reply.send(r.map_err(|e| acp_error("Could not list sessions", e)));
            Ok(())
        }),
        Cmd::Load { session, cwd, mcp, reply } => {
            // Register the cwd first: the agent may read files while replaying.
            dirs.insert(&session, &cwd);
            cx.spawn(async move {
                let req = LoadSessionRequest::new(session, cwd).mcp_servers(mcp::to_acp(&mcp));
                let r = cx2.send_request(req).block_task().await;
                let _ = reply.send(r.map(|_| ()).map_err(|e| acp_error("Could not open the session", e)));
                Ok(())
            })
        }
    }
}

/// Map the callback's choice to an ACP outcome. `None` (deny) picks a reject option if offered.
fn outcome(req: &RequestPermissionRequest, chosen: Option<String>) -> RequestPermissionOutcome {
    let id = chosen
        .and_then(|id| req.options.iter().find(|o| *o.option_id.0 == *id))
        .or_else(|| {
            [PermissionOptionKind::RejectOnce, PermissionOptionKind::RejectAlways]
                .iter()
                .find_map(|k| req.options.iter().find(|o| o.kind == *k))
        })
        .map(|o| o.option_id.clone());
    match id {
        Some(id) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id)),
        None => RequestPermissionOutcome::Cancelled,
    }
}
