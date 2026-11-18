//! [`SharedBackend`]: the handle every aDOs view uses for one backend id. Each call takes a
//! lease for as long as it runs, starts the process when needed (once, however many callers
//! wait), and reopens a session the restarted process does not have open yet.
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use futures::channel::oneshot;
use futures::FutureExt;

use crate::backend::{self, absolute, Backend, BackendInfo};
use crate::event::StopReason;
use crate::launch;
use crate::mcp::McpEntry;
use crate::registry::{retire, Inner, Lease, Page, Run, Sink, Status, Subscription};
use crate::started::{self, StartedSession};

#[derive(Clone)]
pub struct SharedBackend {
    inner: Arc<Inner>,
    id: String,
}

impl SharedBackend {
    pub(crate) fn new(inner: Arc<Inner>, id: &str) -> Self {
        Self { inner, id: id.to_string() }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn status(&self) -> Status {
        match &self.inner.lock().slot(&self.id).run {
            Run::Stopped => Status::Stopped,
            Run::Starting(..) => Status::Starting,
            Run::Running(_) => Status::Running,
            Run::Failed(message) => Status::Failed(message.clone()),
        }
    }

    /// What the agent said about itself the last time it started.
    pub fn info(&self) -> Option<BackendInfo> {
        self.inner.lock().slot(&self.id).info.clone()
    }

    /// How many processes were started for this backend so far.
    pub fn starts(&self) -> u64 {
        self.inner.lock().slot(&self.id).starts
    }

    /// Keep the backend running while the returned lease lives.
    pub fn lease(&self) -> Lease {
        Lease::acquire(&self.inner, &self.id)
    }

    /// Clear a failure (start error, crash, not logged in) so the next call starts it again.
    pub fn retry(&self) {
        let mut st = self.inner.lock();
        let slot = st.slot(&self.id);
        if matches!(slot.run, Run::Failed(_)) {
            slot.run = Run::Stopped;
        }
        slot.lists.clear();
    }

    /// Send `session`'s events (and, if the sink takes them, its permission questions) to `sink`.
    pub fn subscribe(&self, session: &str, sink: Sink) -> Subscription {
        self.inner.subscribe(&self.id, session, sink)
    }

    /// The text seen in `session` since it was opened through the registry in this run.
    pub fn transcript(&self, session: &str) -> Option<String> {
        self.inner.lock().slot(&self.id).transcripts.get(session).map(|t| t.text())
    }

    /// Start the backend (if needed) and report what it is.
    pub async fn start(&self) -> Result<BackendInfo> {
        let _lease = self.lease();
        Ok(self.running().await?.info().clone())
    }

    /// The running process, starting it if needed. Callers hold a lease meanwhile.
    async fn running(&self) -> Result<Arc<Backend>> {
        let (gen, start) = {
            let mut st = self.inner.lock();
            let slot = st.slot(&self.id);
            match &slot.run {
                Run::Running(b) => return Ok(b.clone()),
                Run::Failed(message) => return Err(anyhow!(message.clone())),
                Run::Starting(gen, start) => (*gen, start.clone()),
                Run::Stopped => {
                    let config = self.inner.config(&self.id).map_err(|e| anyhow!(e))?;
                    slot.gen += 1;
                    slot.starts += 1;
                    let gen = slot.gen;
                    let (on_permission, on_event) = self.inner.hooks(&self.id, gen);
                    let start = async move {
                        if let Some(direct) = launch::resolve(&config) {
                            match Backend::start_with(&direct, on_permission.clone(), Some(on_event.clone())).await {
                                Ok(b) => return Ok(Arc::new(b)),
                                Err(e) => eprintln!("aDOs: {} did not start directly ({e:#}); using npx.", config.name),
                            }
                        }
                        Backend::start_with(&config, on_permission, Some(on_event))
                            .await
                            .map(Arc::new)
                            .map_err(|e| e.to_string())
                    }
                    .boxed()
                    .shared();
                    slot.run = Run::Starting(gen, start.clone());
                    (gen, start)
                }
            }
        };
        let result = start.await;
        self.inner.settle(&self.id, gen, &result);
        result.map_err(|e| anyhow!(e))
    }

    /// An auth error stops the process and sticks until [`retry`](Self::retry).
    fn check(&self, e: anyhow::Error) -> anyhow::Error {
        if backend::is_not_logged_in(&e) {
            let mut st = self.inner.lock();
            let old = st.slot(&self.id).take_process(Run::Failed(e.to_string()));
            drop(st);
            old.into_iter().for_each(retire);
        }
        e
    }

    fn name(&self) -> String {
        self.info().map_or_else(|| self.id.clone(), |i| i.name)
    }

    /// `session/list`, one page. Pages are kept for 30 s; a new or loaded session clears them.
    /// A failed backend answers with its failure without starting.
    pub async fn list_sessions(&self, cwd: Option<&Path>, cursor: Option<String>) -> Result<Page> {
        let key = (cwd.map(Path::to_path_buf), cursor.clone());
        {
            let mut st = self.inner.lock();
            let slot = st.slot(&self.id);
            if let Run::Failed(message) = &slot.run {
                return Err(anyhow!(message.clone()));
            }
            if let Some((at, page)) = slot.lists.get(&key) {
                if at.elapsed() < self.inner.list_ttl {
                    return Ok(page.clone());
                }
            }
        }
        let _lease = self.lease();
        let backend = self.running().await?;
        let page = backend.list_sessions(cwd, cursor).await.map_err(|e| self.check(e))?;
        self.inner.lock().slot(&self.id).lists.insert(key, (Instant::now(), page.clone()));
        Ok(page)
    }

    /// `session/new` in `cwd` with `mcp` servers. The session joins [`started`].
    pub async fn new_session(&self, cwd: &Path, mcp: &[McpEntry]) -> Result<String> {
        let _lease = self.lease();
        let backend = self.running().await?;
        let id = backend.new_session(cwd, mcp).await.map_err(|e| self.check(e))?;
        let cwd = absolute(cwd)?;
        {
            let mut st = self.inner.lock();
            let slot = st.slot(&self.id);
            slot.open.insert(id.clone());
            slot.known.insert(id.clone(), (cwd.clone(), mcp.to_vec()));
            slot.lists.clear();
        }
        let info = backend.info();
        started::insert(StartedSession { id: id.clone(), backend: info.id.clone(), backend_name: info.name.clone(), cwd });
        Ok(id)
    }

    /// `session/load`: the replay reaches `session`'s subscribers before this resolves.
    pub async fn load_session(&self, session: &str, cwd: &Path, mcp: &[McpEntry]) -> Result<()> {
        let _lease = self.lease();
        let backend = self.running().await?;
        let cwd = absolute(cwd)?;
        {
            let mut st = self.inner.lock();
            let slot = st.slot(&self.id);
            slot.known.insert(session.to_string(), (cwd.clone(), mcp.to_vec()));
            slot.transcripts.remove(session);
            slot.lists.clear();
        }
        backend.load_session(session, &cwd, mcp).await.map_err(|e| self.check(e))?;
        self.inner.lock().slot(&self.id).open.insert(session.to_string());
        Ok(())
    }

    /// `session/prompt`; resolves when the turn ends. See [`prompt_as`](Self::prompt_as).
    pub async fn prompt(&self, session: &str, text: &str) -> Result<StopReason> {
        self.prompt_as(session, text, text).await
    }

    /// Send `text`, recording `shown` (what the owner typed, without preamble or context)
    /// as the `User:` turn of the session's transcripts. The turn holds its own lease and
    /// runs to its end even if the caller stops waiting.
    pub async fn prompt_as(&self, session: &str, text: &str, shown: &str) -> Result<StopReason> {
        let name = self.name();
        self.spawn_turn(session, text, shown)
            .await
            .map_err(|_| anyhow!("{name} stopped before it answered."))?
    }

    /// Send a prompt and do not wait for the turn (the MCP `sessions_prompt` tool).
    pub fn prompt_detached(&self, session: &str, text: &str) {
        drop(self.spawn_turn(session, text, text));
    }

    fn spawn_turn(&self, session: &str, text: &str, shown: &str) -> oneshot::Receiver<Result<StopReason>> {
        let (tx, rx) = oneshot::channel();
        started::note_user(session, shown);
        self.inner.lock().slot(&self.id).transcripts.entry(session.to_string()).or_default().push("User", shown);
        let (me, lease, session, text) = (self.clone(), self.lease(), session.to_string(), text.to_string());
        let spawned = std::thread::Builder::new().name("ados-agent-turn".into()).spawn(move || {
            let result = smol::block_on(me.turn(&session, &text));
            drop(lease);
            let _ = tx.send(result);
        });
        if let Err(e) = spawned {
            eprintln!("aDOs: could not start a thread for the prompt: {e}");
        }
        rx
    }

    async fn turn(&self, session: &str, text: &str) -> Result<StopReason> {
        let backend = self.running().await?;
        self.reopen(&backend, session).await?;
        backend.prompt(session, text).await.map_err(|e| self.check(e))
    }

    /// After an idle stop the new process has nothing open: load the session again,
    /// without showing its replay to anyone.
    async fn reopen(&self, backend: &Backend, session: &str) -> Result<()> {
        let known = {
            let mut st = self.inner.lock();
            let slot = st.slot(&self.id);
            if slot.open.contains(session) {
                return Ok(());
            }
            let known = slot.known.get(session).cloned();
            if known.is_some() {
                slot.reopening.insert(session.to_string());
            }
            known
        };
        let Some((cwd, mcp)) = known else { return Ok(()) };
        let result = backend.load_session(session, &cwd, &mcp).await;
        let mut st = self.inner.lock();
        let slot = st.slot(&self.id);
        slot.reopening.remove(session);
        match result {
            Ok(()) => {
                slot.open.insert(session.to_string());
                Ok(())
            }
            Err(e) => {
                drop(st);
                let message = format!("Could not reopen the session after {} restarted: {e}", backend.info().name);
                Err(self.check(e.context(message)))
            }
        }
    }

    /// `session/cancel`. Nothing to cancel when the process is not running.
    pub fn cancel(&self, session: &str) -> Result<()> {
        let backend = match &self.inner.lock().slot(&self.id).run {
            Run::Running(b) => b.clone(),
            _ => return Ok(()),
        };
        backend.cancel(session)
    }
}
