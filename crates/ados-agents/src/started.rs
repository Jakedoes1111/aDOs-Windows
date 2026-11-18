//! Sessions aDOs started in this run (`session/new` through any [`Backend`](crate::Backend)).
//!
//! The owner's rule: agents may write only into sessions aDOs started, never into the
//! owner's existing chats. This registry is that list. It also keeps a plain transcript of
//! each such session (from its events, plus the owner's prompts) so a tool can read it
//! without reloading it. Entries stay for the whole run: a backend that idle-stops is
//! restarted, and the session reopened, by the [`Registry`](crate::Registry) on next use.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{anyhow, Result};

use crate::event::AgentEvent;

/// Transcripts are capped; the oldest text goes first.
const MAX_TRANSCRIPT: usize = 200_000;

/// One session aDOs started in this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedSession {
    pub id: String,
    /// The `backends.toml` id and display name.
    pub backend: String,
    pub backend_name: String,
    pub cwd: PathBuf,
}

struct Entry {
    info: StartedSession,
    transcript: Transcript,
}

/// A plain-text transcript built from events: `User:`, `Agent:` and `Tool:` turns.
/// Consecutive chunks of one speaker are joined; the oldest text goes past the cap.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    turns: Vec<(&'static str, String)>,
}

impl Transcript {
    /// Add an event's text (thoughts, updates and crashes are left out).
    pub fn record(&mut self, event: &AgentEvent) {
        let (speaker, text) = match event {
            AgentEvent::Text { text, .. } => ("Agent", text.clone()),
            AgentEvent::UserText { text, .. } => ("User", text.clone()),
            AgentEvent::ToolCall { title, .. } => ("Tool", format!("{title}\n")),
            _ => return,
        };
        self.push(speaker, &text);
    }

    pub(crate) fn push(&mut self, speaker: &'static str, text: &str) {
        let turns = &mut self.turns;
        match turns.last_mut() {
            Some((last, buf)) if *last == speaker => buf.push_str(text),
            _ => turns.push((speaker, text.to_string())),
        }
        let mut total: usize = turns.iter().map(|(_, t)| t.len()).sum();
        while total > MAX_TRANSCRIPT && turns.len() > 1 {
            total -= turns.remove(0).1.len();
        }
    }

    pub fn text(&self) -> String {
        self.turns
            .iter()
            .map(|(who, text)| format!("{who}: {}", text.trim()))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

static STARTED: Mutex<BTreeMap<String, Entry>> = Mutex::new(BTreeMap::new());

fn lock() -> std::sync::MutexGuard<'static, BTreeMap<String, Entry>> {
    STARTED.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn insert(info: StartedSession) {
    lock().insert(info.id.clone(), Entry { info, transcript: Transcript::default() });
}

/// Record what the owner (or a tool) sent into a started session as a `User:` turn.
pub(crate) fn note_user(id: &str, text: &str) {
    if let Some(entry) = lock().get_mut(id) {
        entry.transcript.push("User", text);
    }
}

/// Called for every event a backend receives; only started sessions are recorded.
pub(crate) fn record(event: &AgentEvent) {
    let session = match event {
        AgentEvent::Text { session_id, .. }
        | AgentEvent::UserText { session_id, .. }
        | AgentEvent::ToolCall { session_id, .. } => session_id,
        _ => return,
    };
    if let Some(entry) = lock().get_mut(session) {
        entry.transcript.record(event);
    }
}

/// Every session started in this run, oldest first.
pub fn list() -> Vec<StartedSession> {
    lock().values().map(|e| e.info.clone()).collect()
}

/// The session's transcript as plain text, or `None` if aDOs did not start it in this run.
pub fn transcript(id: &str) -> Option<String> {
    lock().get(id).map(|e| e.transcript.text())
}

/// Send a prompt into a session aDOs started in this run. Returns once it is sent;
/// the reply streams to whichever aDOs view owns the session and into the transcript.
/// The turn runs on the shared backend (started again if it idle-stopped) and holds it
/// until it ends.
pub fn prompt(id: &str, text: &str) -> Result<()> {
    let backend = lock()
        .get(id)
        .map(|e| e.info.backend.clone())
        .ok_or_else(|| anyhow!("Session {id} was not started by aDOs in this run, so it cannot be prompted."))?;
    crate::registry::global().get(&backend).prompt_detached(id, text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_joins_chunks_and_caps() {
        let mut t = Transcript::default();
        let s = String::from("s");
        t.record(&AgentEvent::UserText { session_id: s.clone(), text: "hi".into() });
        t.record(&AgentEvent::Text { session_id: s.clone(), text: "Hel".into() });
        t.record(&AgentEvent::Thought { session_id: s.clone(), text: "hidden".into() });
        t.record(&AgentEvent::Text { session_id: s, text: "lo".into() });
        assert_eq!(t.text(), "User: hi\n\nAgent: Hello");
        t.push("User", &"x".repeat(MAX_TRANSCRIPT));
        assert_eq!(t.turns.len(), 1);
    }

    #[test]
    fn unknown_sessions_cannot_be_prompted() {
        let err = prompt("never-started", "hi").expect_err("refused");
        assert!(err.to_string().contains("not started by aDOs"));
        assert!(transcript("never-started").is_none());
    }
}
