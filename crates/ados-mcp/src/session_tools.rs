//! Tools over agent sessions, on the same shared backends as the assistant and Sessions
//! (`ados_agents::registry`): no second set of agent processes. Each call holds a lease
//! only while it runs, so the backends idle-stop afterwards; `session/list` pages are
//! cached for 30 s. Reading loads a session with a read-only subscription (permissions
//! denied); prompting only reaches sessions aDOs started in this run.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use ados_agents::started::{self, Transcript};
use ados_agents::{AgentEvent, Sink};
use futures::lock::Mutex;
use gpui_kit::*;
use serde_json::{json, Value};

use crate::host::toast;
use crate::protocol::ToolResult;

thread_local! {
    /// session id → (backend id, cwd), from the last listing.
    static KNOWN: RefCell<HashMap<String, (String, PathBuf)>> = RefCell::new(HashMap::new());
}

/// One read (load + replay) at a time, so replays are not mixed up.
static READING: Mutex<()> = Mutex::new(());

async fn list_one(id: String, name: String) -> Value {
    match ados_agents::shared(&id).list_sessions(None, None).await {
        Ok((sessions, more)) => {
            let started: Vec<String> = started::list().into_iter().map(|s| s.id).collect();
            let list: Vec<Value> = sessions
                .iter()
                .map(|s| {
                    KNOWN.with(|k| k.borrow_mut().insert(s.id.clone(), (id.clone(), s.cwd.clone())));
                    json!({
                        "id": s.id, "title": s.title, "cwd": s.cwd.to_string_lossy(),
                        "updated_at": s.updated_at, "started_here": started.contains(&s.id),
                    })
                })
                .collect();
            json!({ "backend": id, "name": name, "sessions": list, "more": more.is_some() })
        }
        Err(e) => json!({ "backend": id, "name": name, "error": e.to_string() }),
    }
}

pub async fn list(only: Option<String>, _cx: &mut AsyncApp) -> ToolResult {
    let configs = ados_agents::config::load().map_err(|e| format!("Could not read backends.toml: {e:#}"))?;
    let wanted: Vec<_> = configs.into_iter().filter(|c| only.as_ref().is_none_or(|o| *o == c.id)).collect();
    if wanted.is_empty() {
        return Err(format!("There is no backend \"{}\" in backends.toml.", only.unwrap_or_default()));
    }
    let started: Vec<Value> = started::list()
        .into_iter()
        .filter(|s| only.as_ref().is_none_or(|o| *o == s.backend))
        .map(|s| json!({ "id": s.id, "backend": s.backend, "cwd": s.cwd.to_string_lossy() }))
        .collect();
    let backends = futures::future::join_all(wanted.into_iter().map(|c| list_one(c.id, c.name))).await;
    Ok(json!({ "started_here": started, "backends": backends }))
}

pub async fn read(id: String, backend_id: Option<String>, _cx: &mut AsyncApp) -> ToolResult {
    if let Some(text) = started::transcript(&id) {
        return Ok(json!({ "id": id, "started_here": true, "transcript": text }));
    }
    let mut known = KNOWN.with(|k| k.borrow().get(&id).cloned());
    if known.is_none() {
        if let Some(b) = &backend_id {
            let _ = list_one(b.clone(), b.clone()).await;
            known = KNOWN.with(|k| k.borrow().get(&id).cloned());
        }
    }
    let (b, cwd) = known.ok_or_else(|| format!("Session {id} is not in the last listing. Run sessions_list first."))?;
    let backend = ados_agents::shared(&b);
    // Open in Sessions or the assistant already: its text is known, no reload needed.
    if let Some(text) = backend.transcript(&id).filter(|t| !t.is_empty()) {
        return Ok(json!({ "id": id, "backend": b, "started_here": false, "transcript": text }));
    }
    let _guard = READING.lock().await;
    let (events, rx) = async_channel::unbounded();
    let _sub = backend.subscribe(&id, Sink { events, permissions: None });
    backend.load_session(&id, &cwd, &[]).await.map_err(|e| e.to_string())?;
    let mut transcript = Transcript::default();
    while let Ok(event) = rx.try_recv() {
        if matches!(event, AgentEvent::Text { .. } | AgentEvent::UserText { .. } | AgentEvent::ToolCall { .. }) {
            transcript.record(&event);
        }
    }
    Ok(json!({ "id": id, "backend": b, "started_here": false, "transcript": transcript.text() }))
}

pub fn prompt(id: &str, text: &str, window: &mut Window, cx: &mut App) -> ToolResult {
    if text.trim().is_empty() {
        return Err("The prompt is empty.".into());
    }
    started::prompt(id, text).map_err(|e| e.to_string())?;
    toast(format!("sent a prompt to session {id}"), None, window, cx);
    Ok(json!({ "sent": true, "id": id, "next": "Read the reply with sessions_read." }))
}
