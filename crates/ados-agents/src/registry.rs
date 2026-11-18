//! One shared set of agent backends for all of aDOs: at most one process per backend id,
//! used by the assistant, the Sessions panel and the MCP server alike.
//!
//! - **Leases.** Whoever needs a backend holds a [`Lease`] (a view while it shows something
//!   live, every call while it runs, a turn until it ends, a permission question until it is
//!   answered). When the last lease goes, the process is stopped after the idle delay
//!   (`[settings] idle_stop_secs` in `backends.toml`, default 3 minutes) and started again,
//!   transparently, on next use.
//! - **Fan-out.** Events and permission requests go to whoever [subscribed](SharedBackend::subscribe)
//!   to their session id. A permission request nobody owns is denied.
//! - **Failures stick.** A backend that failed to start, crashed or is not logged in is not
//!   started again until someone calls [`SharedBackend::retry`].
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use futures::channel::oneshot;
use futures::future::{BoxFuture, Shared};

use crate::backend::{Backend, BackendInfo, EventHook, SessionSummary};
use crate::config::{self, BackendConfig};
use crate::event::{AgentEvent, PermissionCallback, PermissionRequest};
use crate::mcp::McpEntry;
use crate::shared::SharedBackend;
use crate::started::Transcript;

/// A permission question and where its answer goes (`None` denies).
pub type Permission = (PermissionRequest, oneshot::Sender<Option<String>>);
/// One `session/list` page and the cursor of the next one.
pub type Page = (Vec<SessionSummary>, Option<String>);

/// Where a subscriber wants its session's events and permission questions.
/// A sink without `permissions` (a read-only reader) never gets asked.
#[derive(Clone)]
pub struct Sink {
    pub events: async_channel::Sender<AgentEvent>,
    pub permissions: Option<async_channel::Sender<Permission>>,
}

impl Sink {
    /// A sink that takes permission questions, and the two ends to read.
    pub fn new() -> (Self, async_channel::Receiver<AgentEvent>, async_channel::Receiver<Permission>) {
        let (events, event_rx) = async_channel::unbounded();
        let (permissions, permission_rx) = async_channel::unbounded();
        (Self { events, permissions: Some(permissions) }, event_rx, permission_rx)
    }
}

/// What a backend's process is doing, for status rows ("Starting pi…").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Stopped,
    Starting,
    Running,
    /// The plain sentence shown with Retry.
    Failed(String),
}

pub(crate) type Start = Shared<BoxFuture<'static, Result<Arc<Backend>, String>>>;

pub(crate) enum Run {
    Stopped,
    Starting(u64, Start),
    Running(Arc<Backend>),
    Failed(String),
}

pub(crate) struct Sub {
    id: u64,
    session: String,
    sink: Sink,
}

pub(crate) type ListKey = (Option<PathBuf>, Option<String>);

pub(crate) struct Slot {
    pub run: Run,
    /// Bumped on every start, so events and results from an older process are dropped.
    pub gen: u64,
    pub leases: usize,
    /// Set when the last lease went while the process runs.
    pub idle_since: Option<Instant>,
    subs: Vec<Sub>,
    /// Sessions open (new or loaded) on the current process.
    pub open: HashSet<String>,
    /// Every session opened through the registry in this run, to reopen after a restart.
    pub known: HashMap<String, (PathBuf, Vec<McpEntry>)>,
    /// Sessions being reopened after a restart: their replay is not shown again.
    pub reopening: HashSet<String>,
    pub lists: HashMap<ListKey, (Instant, Page)>,
    pub info: Option<BackendInfo>,
    pub transcripts: HashMap<String, Transcript>,
    /// How many processes were started for this backend (for tests and the handoff).
    pub starts: u64,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            run: Run::Stopped,
            gen: 0,
            leases: 0,
            idle_since: None,
            subs: vec![],
            open: HashSet::new(),
            known: HashMap::new(),
            reopening: HashSet::new(),
            lists: HashMap::new(),
            info: None,
            transcripts: HashMap::new(),
            starts: 0,
        }
    }
}

impl Slot {
    /// Take the process out (it is stopped by [`retire`]) and forget what was open on it.
    pub fn take_process(&mut self, next: Run) -> Option<Arc<Backend>> {
        self.open.clear();
        self.idle_since = None;
        match std::mem::replace(&mut self.run, next) {
            Run::Running(b) => Some(b),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub slots: HashMap<String, Slot>,
    next_sub: u64,
    reaper: bool,
    closed: bool,
}

impl State {
    pub fn slot(&mut self, id: &str) -> &mut Slot {
        self.slots.entry(id.to_string()).or_default()
    }
}

type Locked = Arc<(Mutex<State>, Condvar)>;

pub(crate) struct Inner {
    state: Locked,
    configs: Option<Vec<BackendConfig>>,
    pub idle: Duration,
    pub list_ttl: Duration,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let (lock, cv) = &*self.state;
        lock.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        cv.notify_all();
    }
}

impl Inner {
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn config(&self, id: &str) -> Result<BackendConfig, String> {
        let list = match &self.configs {
            Some(list) => list.clone(),
            None => config::load().map_err(|e| format!("Could not read backends.toml: {e:#}"))?,
        };
        config::find(&list, id).cloned().ok_or_else(|| format!("There is no backend \"{id}\" in backends.toml."))
    }

    /// The last lease went: start the idle clock and wake the reaper.
    fn went_idle(&self, st: &mut State, id: &str) {
        let slot = st.slot(id);
        if slot.leases == 0 && matches!(slot.run, Run::Running(_)) {
            slot.idle_since = Some(Instant::now());
            if !st.reaper {
                st.reaper = true;
                spawn_reaper(self.state.clone(), self.idle);
            }
            self.state.1.notify_all();
        }
    }

    /// A start finished: record it unless something newer replaced it meanwhile.
    pub fn settle(&self, id: &str, gen: u64, result: &Result<Arc<Backend>, String>) {
        let mut st = self.lock();
        let slot = st.slot(id);
        if !matches!(slot.run, Run::Starting(g, _) if g == gen) {
            return;
        }
        match result {
            Ok(backend) => {
                slot.info = Some(backend.info().clone());
                slot.run = Run::Running(backend.clone());
                self.went_idle(&mut st, id);
            }
            Err(message) => slot.run = Run::Failed(message.clone()),
        }
    }

    /// Called on the backend's thread for every event, in wire order.
    fn dispatch(&self, id: &str, gen: u64, event: AgentEvent) {
        let mut st = self.lock();
        let slot = st.slot(id);
        if slot.gen != gen {
            return;
        }
        let (sinks, old) = match &event {
            AgentEvent::Crashed { message } => {
                if !matches!(slot.run, Run::Running(_)) {
                    return;
                }
                let old = slot.take_process(Run::Failed(message.clone()));
                (unique(slot.subs.iter().map(|s| &s.sink)), old)
            }
            _ => {
                let Some(session) = event.session_id() else { return };
                if slot.reopening.contains(session) {
                    return;
                }
                slot.transcripts.entry(session.to_string()).or_default().record(&event);
                (unique(slot.subs.iter().filter(|s| s.session == session).map(|s| &s.sink)), None)
            }
        };
        drop(st);
        for sink in sinks {
            let _ = sink.events.try_send(event.clone());
        }
        if let Some(old) = old {
            retire(old);
        }
    }

    pub fn subscribe(self: &Arc<Self>, backend: &str, session: &str, sink: Sink) -> Subscription {
        let mut st = self.lock();
        st.next_sub += 1;
        let id = st.next_sub;
        st.slot(backend).subs.push(Sub { id, session: session.to_string(), sink });
        Subscription { inner: Arc::downgrade(self), backend: backend.to_string(), id }
    }

    /// The newest subscriber of `session` that takes permission questions.
    fn permission_owner(&self, backend: &str, session: &str) -> Option<async_channel::Sender<Permission>> {
        let mut st = self.lock();
        st.slot(backend).subs.iter().rev().filter(|s| s.session == session).find_map(|s| s.sink.permissions.clone())
    }

    /// What a new process for `id` (generation `gen`) does with events and permission questions.
    pub fn hooks(self: &Arc<Self>, id: &str, gen: u64) -> (PermissionCallback, EventHook) {
        let (weak, backend) = (Arc::downgrade(self), id.to_string());
        let on_event: EventHook = {
            let (weak, backend) = (weak.clone(), backend.clone());
            Arc::new(move |event| {
                if let Some(inner) = weak.upgrade() {
                    inner.dispatch(&backend, gen, event);
                }
            })
        };
        let on_permission: PermissionCallback = Arc::new(move |request: PermissionRequest| {
            let (weak, backend) = (weak.clone(), backend.clone());
            Box::pin(async move {
                let inner = weak.upgrade()?;
                let owner = inner.permission_owner(&backend, &request.session_id)?;
                // A question waiting for its answer keeps the backend running.
                let _lease = Lease::acquire(&inner, &backend);
                drop(inner);
                let (reply, answer) = oneshot::channel();
                owner.send((request, reply)).await.ok()?;
                answer.await.ok().flatten()
            })
        });
        (on_permission, on_event)
    }
}

fn unique<'a>(sinks: impl Iterator<Item = &'a Sink>) -> Vec<Sink> {
    let mut out: Vec<Sink> = vec![];
    for sink in sinks {
        if !out.iter().any(|s| s.events.same_channel(&sink.events)) {
            out.push(sink.clone());
        }
    }
    out
}

/// Stop a process off the caller's thread: dropping a `Backend` joins its thread, which
/// must never happen on that thread itself or while the registry is locked.
pub(crate) fn retire(backend: Arc<Backend>) {
    let _ = std::thread::Builder::new().name("ados-agent-stop".into()).spawn(move || drop(backend));
}

/// One thread per registry stops backends whose idle delay ran out. It sleeps on a condvar
/// until the next deadline, or until woken; it never polls.
fn spawn_reaper(state: Locked, idle: Duration) {
    let spawned = std::thread::Builder::new().name("ados-agent-idle".into()).spawn(move || {
        let (lock, cv) = &*state;
        let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if st.closed {
                return;
            }
            let now = Instant::now();
            let mut next: Option<Instant> = None;
            let mut stopped = vec![];
            for slot in st.slots.values_mut() {
                let Some(since) = slot.idle_since.filter(|_| slot.leases == 0) else { continue };
                if !matches!(slot.run, Run::Running(_)) {
                    continue;
                }
                let due = since + idle;
                if due <= now {
                    stopped.extend(slot.take_process(Run::Stopped));
                } else {
                    next = Some(next.map_or(due, |n| n.min(due)));
                }
            }
            if !stopped.is_empty() {
                drop(st);
                stopped.into_iter().for_each(retire);
                st = lock.lock().unwrap_or_else(|e| e.into_inner());
                continue;
            }
            st = match next {
                Some(due) => cv.wait_timeout(st, due - now).unwrap_or_else(|e| e.into_inner()).0,
                None => cv.wait(st).unwrap_or_else(|e| e.into_inner()),
            };
        }
    });
    if let Err(e) = spawned {
        eprintln!("aDOs: could not start the idle-stop thread, backends will keep running: {e}");
    }
}

/// Holding one keeps the backend from being idle-stopped. Clones are separate leases.
pub struct Lease {
    inner: Arc<Inner>,
    id: String,
}

impl Lease {
    pub(crate) fn acquire(inner: &Arc<Inner>, id: &str) -> Self {
        let mut st = inner.lock();
        let slot = st.slot(id);
        slot.leases += 1;
        slot.idle_since = None;
        Self { inner: inner.clone(), id: id.to_string() }
    }
}

impl Clone for Lease {
    fn clone(&self) -> Self {
        Self::acquire(&self.inner, &self.id)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut st = self.inner.lock();
        let slot = st.slot(&self.id);
        slot.leases = slot.leases.saturating_sub(1);
        self.inner.went_idle(&mut st, &self.id);
    }
}

/// Events for one session keep coming while this lives.
pub struct Subscription {
    inner: Weak<Inner>,
    backend: String,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.lock().slot(&self.backend).subs.retain(|s| s.id != self.id);
        }
    }
}

/// The set of shared backends. Cheap to clone; [`global`] is the one aDOs uses.
#[derive(Clone)]
pub struct Registry {
    inner: Arc<Inner>,
}

impl Registry {
    /// `configs: None` reads `ADOS_HOME/backends.toml` at every start, so edits apply on
    /// the next start. Tests pass their own list and a short `idle` delay.
    pub fn new(configs: Option<Vec<BackendConfig>>, idle: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Arc::new((Mutex::new(State::default()), Condvar::new())),
                configs,
                idle,
                list_ttl: Duration::from_secs(30),
            }),
        }
    }

    /// The handle for backend `id` (nothing starts until it is used).
    pub fn get(&self, id: &str) -> SharedBackend {
        SharedBackend::new(self.inner.clone(), id)
    }
}

/// aDOs's registry, configured from `backends.toml` on first use.
pub fn global() -> &'static Registry {
    static GLOBAL: OnceLock<Registry> = OnceLock::new();
    GLOBAL.get_or_init(|| Registry::new(None, Duration::from_secs(config::settings().idle_stop_secs)))
}

/// Shorthand for `global().get(id)`.
pub fn shared(id: &str) -> SharedBackend {
    global().get(id)
}
