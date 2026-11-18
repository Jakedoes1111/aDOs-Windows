//! Spawns `examples/fake_agent.rs` (built by `cargo test`) and checks the client end to end.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ados_agents::{
    config, deny, AgentEvent, Backend, BackendConfig, PermissionCallback, PermissionRequest,
    StopReason, ToolStatus,
};

fn fake_agent() -> BackendConfig {
    // tests run from target/<profile>/deps/; examples land in target/<profile>/examples/.
    let exe = std::env::current_exe().expect("test exe path");
    let path = exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|dir| dir.join("examples").join("fake_agent"))
        .expect("target dir");
    assert!(path.exists(), "{} missing: run via `cargo test -p ados-agents`", path.display());
    BackendConfig {
        id: "fake".into(),
        name: "Fake agent".into(),
        command: path.display().to_string(),
        args: vec![],
        env: Default::default(),
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ados-agents").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Fail instead of hanging forever.
fn run<T>(fut: impl std::future::Future<Output = T>) -> T {
    smol::block_on(smol::future::or(fut, async {
        smol::Timer::after(Duration::from_secs(20)).await;
        panic!("test timed out");
    }))
}

fn drain(backend: &Backend) -> Vec<AgentEvent> {
    let rx = backend.events();
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

fn text_of(events: &[AgentEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn streams_text_tool_calls_and_routes_permission() {
    let cwd = scratch("allow");
    let asked: Arc<Mutex<Vec<PermissionRequest>>> = Arc::default();
    let seen = asked.clone();
    let allow: PermissionCallback = Arc::new(move |req| {
        seen.lock().unwrap().push(req);
        Box::pin(async { Some("allow".to_string()) })
    });
    run(async {
        let backend = Backend::start(&fake_agent(), allow).await.expect("start");
        assert_eq!(backend.info().agent.as_deref(), Some("fake-agent 0.1.0"));
        let session = backend.new_session(&cwd, &[]).await.expect("session/new");
        let reason = backend.prompt(&session, "write me").await.expect("prompt");
        assert_eq!(reason, StopReason::EndTurn);
        let events = drain(&backend);

        // Text chunks arrive in order, and the agent saw the "allow" answer.
        assert_eq!(text_of(&events), "Hello, this is the fake agent. Permission: allow. File says: write me");
        assert!(events.contains(&AgentEvent::Thought { session_id: session.clone(), text: "Thinking about it.".into() }));

        // Tool call, then its status updates, in order.
        let tools: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolCall { .. } | AgentEvent::ToolCallUpdate { .. }))
            .cloned()
            .collect();
        assert_eq!(
            tools,
            vec![
                AgentEvent::ToolCall {
                    session_id: session.clone(),
                    id: "tool-1".into(),
                    title: "Write fake-note.txt".into(),
                    kind: "edit".into(),
                    status: ToolStatus::Pending,
                },
                AgentEvent::ToolCallUpdate { session_id: session.clone(), id: "tool-1".into(), title: None, status: Some(ToolStatus::InProgress) },
                AgentEvent::ToolCallUpdate { session_id: session.clone(), id: "tool-1".into(), title: None, status: Some(ToolStatus::Completed) },
            ]
        );
    });
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1, "permission callback called once");
    assert_eq!(asked[0].title, "Write fake-note.txt");
    assert_eq!(asked[0].options.len(), 2);
    // fs/write_text_file went through the client into the session cwd.
    assert_eq!(std::fs::read_to_string(cwd.join("fake-note.txt")).unwrap(), "write me");
}

#[test]
fn default_callback_denies() {
    let cwd = scratch("deny");
    run(async {
        let backend = Backend::start(&fake_agent(), deny()).await.expect("start");
        let session = backend.new_session(&cwd, &[]).await.expect("session/new");
        backend.prompt(&session, "write me").await.expect("prompt");
        let events = drain(&backend);
        assert!(text_of(&events).ends_with("Permission: reject."), "{}", text_of(&events));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolCallUpdate { status: Some(ToolStatus::Failed), .. })));
    });
    assert!(!cwd.join("fake-note.txt").exists());
}

#[test]
fn killed_agent_yields_crash_event() {
    let cwd = scratch("crash");
    run(async {
        let backend = Backend::start(&fake_agent(), deny()).await.expect("start");
        let session = backend.new_session(&cwd, &[]).await.expect("session/new");
        backend.prompt(&session, "pid").await.expect("prompt");
        let pid = text_of(&drain(&backend));
        assert!(pid.parse::<u32>().is_ok(), "pid reply: {pid}");
        let status = std::process::Command::new("kill").args(["-9", &pid]).status().expect("kill");
        assert!(status.success());
        let event = backend.events().recv().await.expect("event after kill");
        match event {
            AgentEvent::Crashed { message } => assert!(message.starts_with("Fake agent stopped"), "{message}"),
            other => panic!("expected Crashed, got {other:?}"),
        }
        // Later calls fail with a sentence, not a panic.
        assert!(backend.prompt(&session, "hello").await.is_err());
    });
}

#[test]
fn cancel_ends_turn_as_cancelled() {
    let cwd = scratch("cancel");
    run(async {
        let backend = Backend::start(&fake_agent(), deny()).await.expect("start");
        let session = backend.new_session(&cwd, &[]).await.expect("session/new");
        let events = backend.events();
        let prompt = backend.prompt(&session, "wait");
        let cancel = async {
            let first = events.recv().await.expect("first event");
            assert!(matches!(first, AgentEvent::Text { ref text, .. } if text == "waiting"));
            backend.cancel(&session).expect("cancel");
        };
        let (reason, ()) = futures::join!(prompt, cancel);
        assert_eq!(reason.expect("prompt"), StopReason::Cancelled);
    });
}

#[test]
fn lists_and_loads_sessions() {
    let cwd = scratch("list");
    run(async {
        let backend = Backend::start(&fake_agent(), deny()).await.expect("start");
        assert!(backend.info().can_list_sessions && backend.info().can_load_sessions);
        let made = backend.new_session(&cwd, &[]).await.expect("session/new");
        let (list, next) = backend.list_sessions(None, None).await.expect("session/list");
        assert_eq!(next, None);
        assert_eq!(list[0].id, "old-1");
        assert_eq!(list[0].title.as_deref(), Some("Earlier work"));
        assert!(list.iter().any(|s| s.id == made && s.cwd == cwd));

        backend.load_session("old-1", &cwd, &[]).await.expect("session/load");
        let events = drain(&backend);
        assert_eq!(
            events,
            vec![
                AgentEvent::UserText { session_id: "old-1".into(), text: "earlier question".into() },
                AgentEvent::Text { session_id: "old-1".into(), text: "earlier answer".into() },
            ]
        );
    });
}

#[test]
fn missing_command_is_an_error_not_a_panic() {
    let mut cfg = fake_agent();
    cfg.command = "/nonexistent/ados-no-such-agent".into();
    let err = run(Backend::start(&cfg, deny())).err().expect("start must fail");
    assert!(err.to_string().contains("Fake agent"), "{err}");
}

#[test]
fn backends_toml_default_is_written_and_parsed() {
    let dir = scratch("config");
    let path = dir.join("backends.toml");
    let list = config::load_from(&path).expect("load");
    assert!(path.exists());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), config::DEFAULT_BACKENDS);
    let ids: Vec<_> = list.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["pi", "codex", "claude"]);
    assert_eq!(list[0].command, "npx");
    assert_eq!(list[0].args, ["-y", "pi-acp@0.0.34"]);
    assert_eq!(list[1].args, ["-y", "@agentclientprotocol/codex-acp@2.6.0"]);

    // An edited file is read as-is, not overwritten.
    std::fs::write(&path, "[[backend]]\nid = \"x\"\nname = \"X\"\ncommand = \"x-acp\"\nenv = { A = \"1\" }\n").unwrap();
    let list = config::load_from(&path).expect("reload");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].env.get("A").map(String::as_str), Some("1"));
}

#[test]
fn dropping_the_backend_kills_the_agent() {
    let cwd = scratch("drop");
    let pid = run(async {
        let backend = Backend::start(&fake_agent(), deny()).await.expect("start");
        let session = backend.new_session(&cwd, &[]).await.expect("session/new");
        backend.prompt(&session, "pid").await.expect("prompt");
        text_of(&drain(&backend))
    });
    let proc_dir = PathBuf::from("/proc").join(&pid);
    // `run` dropped the backend; the process may linger as a zombie for a moment at most.
    let gone = (0..50).any(|_| {
        let alive = std::fs::read_to_string(proc_dir.join("stat"))
            .map(|s| !s.contains(") Z "))
            .unwrap_or(false);
        if alive {
            std::thread::sleep(Duration::from_millis(20));
        }
        !alive
    });
    assert!(gone, "fake agent {pid} still running after drop");
}
