//! The shared registry against the real fake-agent binary: one process per backend,
//! lease counting, the idle stop and transparent restart, fan-out by session id,
//! and a logged-out backend that is not started again until Retry.
use std::path::PathBuf;
use std::time::Duration;

use ados_agents::{AgentEvent, BackendConfig, Registry, Sink, Status, StopReason};

fn fake_agent(name: &str, auth: bool) -> (BackendConfig, PathBuf) {
    let exe = std::env::current_exe().expect("test exe path");
    let path = exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|dir| dir.join("examples").join(format!("fake_agent{}", std::env::consts::EXE_SUFFIX)))
        .expect("target dir");
    assert!(path.exists(), "{} missing: run via `cargo test -p ados-agents`", path.display());
    let dir = scratch(name);
    let starts = dir.join("starts.txt");
    let mut env = std::collections::BTreeMap::new();
    env.insert("FAKE_AGENT_STARTS".to_string(), starts.display().to_string());
    if auth {
        env.insert("FAKE_AGENT_AUTH".to_string(), "1".to_string());
    }
    let config = BackendConfig {
        id: "fake".into(),
        name: "Fake agent".into(),
        command: path.display().to_string(),
        args: vec![],
        env,
    };
    (config, dir)
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ados-registry").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Process starts the fake agent logged (one line each).
fn starts(dir: &std::path::Path) -> usize {
    std::fs::read_to_string(dir.join("starts.txt")).map(|t| t.lines().count()).unwrap_or(0)
}

fn alive(pid: u32) -> bool {
    if cfg!(windows) {
        let out = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output();
        return out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().any(|w| w == pid.to_string()));
    }
    // A killed child stays a zombie until reaped; count only live states.
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|s| !s.split(") ").nth(1).is_some_and(|rest| rest.starts_with('Z')))
        .unwrap_or(false)
}

fn last_pid(dir: &std::path::Path) -> u32 {
    let text = std::fs::read_to_string(dir.join("starts.txt")).expect("starts file");
    text.lines().last().and_then(|l| l.trim().parse().ok()).expect("pid line")
}

fn run<T>(fut: impl std::future::Future<Output = T>) -> T {
    smol::block_on(smol::future::or(fut, async {
        smol::Timer::after(Duration::from_secs(20)).await;
        panic!("test timed out");
    }))
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..100 {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for: {what}");
}

#[test]
fn one_process_shared_by_every_handle_and_list_is_cached() {
    let (config, dir) = fake_agent("shared", false);
    let registry = Registry::new(Some(vec![config]), Duration::from_secs(60));
    let (a, b) = (registry.get("fake"), registry.get("fake"));
    run(async {
        let (first, second) = futures::join!(a.list_sessions(None, None), b.list_sessions(None, None));
        assert_eq!(first.expect("list a").0.len(), 3);
        assert_eq!(second.expect("list b").0.len(), 3);
        a.new_session(&dir, &[]).await.expect("new");
        b.list_sessions(None, None).await.expect("list again");
    });
    assert_eq!(starts(&dir), 1, "two handles, two concurrent starts: one process");
    assert_eq!(a.starts(), 1);
}

#[test]
fn idle_stop_after_last_lease_and_transparent_restart() {
    let (config, dir) = fake_agent("idle", false);
    let registry = Registry::new(Some(vec![config]), Duration::from_millis(400));
    let backend = registry.get("fake");
    let lease = backend.lease();
    let second = lease.clone();
    run(backend.start()).expect("start");
    let pid = last_pid(&dir);
    drop(lease);
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(backend.status(), Status::Running, "one lease is still held");
    assert!(alive(pid));
    drop(second);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(backend.status(), Status::Running, "idle delay not over yet");
    wait_until("idle stop", || backend.status() == Status::Stopped);
    wait_until("process gone", || !alive(pid));

    // A new session after the stop starts it again without any error.
    let session = run(backend.new_session(&dir, &[])).expect("new after idle stop");
    assert_eq!(starts(&dir), 2);
    assert!(session.starts_with("fake-"));
}

#[test]
fn turn_reopens_its_session_after_idle_stop_without_replaying_it() {
    let (config, dir) = fake_agent("reopen", false);
    let registry = Registry::new(Some(vec![config]), Duration::from_millis(200));
    let backend = registry.get("fake");
    let session = run(backend.new_session(&dir, &[])).expect("new");
    let (sink, events, _questions) = Sink::new();
    let _sub = backend.subscribe(&session, sink);
    wait_until("idle stop", || backend.status() == Status::Stopped);
    let stop = run(backend.prompt(&session, "pid")).expect("prompt after restart");
    assert_eq!(stop, StopReason::EndTurn);
    let seen: Vec<AgentEvent> = std::iter::from_fn(|| events.try_recv().ok()).collect();
    assert!(
        !seen.iter().any(|e| matches!(e, AgentEvent::UserText { .. })),
        "the reopen replay must not reach the subscriber: {seen:?}"
    );
    assert!(seen.iter().any(|e| matches!(e, AgentEvent::Text { .. })), "the reply arrives: {seen:?}");
    assert_eq!(starts(&dir), 2);
    let transcript = backend.transcript(&session).expect("transcript");
    assert!(transcript.starts_with("User: pid"), "{transcript}");
}

#[test]
fn events_and_permissions_fan_out_by_session_id() {
    let (config, dir) = fake_agent("fanout", false);
    let registry = Registry::new(Some(vec![config]), Duration::from_secs(60));
    let backend = registry.get("fake");
    let (one, two) = run(async {
        (backend.new_session(&dir, &[]).await.expect("one"), backend.new_session(&dir, &[]).await.expect("two"))
    });
    let (sink_one, events_one, questions_one) = Sink::new();
    let (sink_two, events_two, questions_two) = Sink::new();
    let _a = backend.subscribe(&one, sink_one);
    let _b = backend.subscribe(&two, sink_two);
    // Answer session one's question from a thread, as a UI would.
    let answer = std::thread::spawn(move || {
        let (request, reply) = smol::block_on(questions_one.recv()).expect("question for one");
        let allow = request.options.iter().find(|o| o.kind.is_allow()).map(|o| o.id.clone());
        let _ = reply.send(allow);
        request.session_id
    });
    run(backend.prompt(&one, "hello one")).expect("prompt one");
    assert_eq!(answer.join().expect("answer thread"), one);
    let got_one: Vec<_> = std::iter::from_fn(|| events_one.try_recv().ok()).collect();
    assert!(got_one.iter().all(|e| e.session_id() == Some(one.as_str())));
    assert!(got_one.iter().any(|e| matches!(e, AgentEvent::Text { text, .. } if text.contains("allow"))), "{got_one:?}");
    assert!(events_two.try_recv().is_err(), "session two saw nothing");
    assert!(questions_two.try_recv().is_err(), "session two was not asked");
    assert_eq!(starts(&dir), 1);
}

#[test]
fn unowned_permission_is_denied() {
    let (config, dir) = fake_agent("unowned", false);
    let registry = Registry::new(Some(vec![config]), Duration::from_secs(60));
    let backend = registry.get("fake");
    let session = run(backend.new_session(&dir, &[])).expect("new");
    run(backend.prompt(&session, "nobody listens")).expect("prompt");
    let text = backend.transcript(&session).expect("transcript");
    assert!(text.contains("Permission: reject."), "{text}");
}

#[test]
fn logged_out_backend_is_not_restarted_until_retry() {
    let (config, dir) = fake_agent("auth", true);
    let registry = Registry::new(Some(vec![config]), Duration::from_secs(60));
    let backend = registry.get("fake");
    let err = run(backend.list_sessions(None, None)).expect_err("auth error");
    assert!(ados_agents::is_not_logged_in(&err), "{err:#}");
    assert!(matches!(backend.status(), Status::Failed(_)));
    let pid = last_pid(&dir);
    wait_until("logged-out process stopped", || !alive(pid));
    // Visible again, listed again, asked by MCP: no new process.
    for _ in 0..3 {
        assert!(run(backend.list_sessions(None, None)).is_err());
    }
    assert!(run(backend.new_session(&dir, &[])).is_err());
    assert_eq!(starts(&dir), 1);
    backend.retry();
    assert!(run(backend.list_sessions(None, None)).is_err());
    assert_eq!(starts(&dir), 2, "Retry starts it once more");
}

#[test]
fn crash_reaches_every_subscriber_and_sticks() {
    let (config, dir) = fake_agent("crash", false);
    let registry = Registry::new(Some(vec![config]), Duration::from_secs(60));
    let backend = registry.get("fake");
    let session = run(backend.new_session(&dir, &[])).expect("new");
    let (sink, events, _q) = Sink::new();
    let _sub = backend.subscribe(&session, sink);
    let pid = last_pid(&dir);
    let _ = if cfg!(windows) {
        std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).status()
    } else {
        std::process::Command::new("kill").args(["-9", &pid.to_string()]).status()
    };
    wait_until("crash event", || matches!(events.try_recv(), Ok(AgentEvent::Crashed { .. })));
    assert!(matches!(backend.status(), Status::Failed(_)));
    assert!(run(backend.list_sessions(None, Some("x".into()))).is_err());
    assert_eq!(starts(&dir), 1);
}
