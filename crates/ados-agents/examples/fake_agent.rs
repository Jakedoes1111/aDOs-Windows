//! A fake ACP agent for tests and cloud checks, grown from the crate's
//! `examples/simple_agent.rs`. It speaks ACP v1 over stdio.
//!
//! Prompts:
//! - anything else: a thought, four text chunks, one tool call that asks permission,
//!   then (if allowed) writes and reads `fake-note.txt` in the session cwd through
//!   the client's `fs/*` methods, and says which option it got.
//! - `pid`: replies with its process id (tests kill it).
//! - `wait`: replies "waiting", then waits up to 10 s for `session/cancel`.
//! - starting with `notice`: pi's update line (as pi-acp forwards it), then a short reply.
//! - containing a line `mcp <tool> <json args>`: calls that tool on the first stdio MCP
//!   server the session got (as a real agent would: spawn it, initialize, tools/call) and
//!   replies with the result, e.g. `mcp tasks_add {"text":"Buy milk tomorrow"}`.
//!
//! It advertises `loadSession` and `sessionCapabilities.list`; the list always
//! includes one earlier session, `old-1`, plus `old-2` (front-matter title, in `$HOME`)
//! and `old-3` (an automation prompt, in another folder) for the chats list.
//! The old sessions carry `updatedAt`. With `FAKE_AGENT_PAGES=n` the list is paged:
//! the first page as above plus a `next_cursor`, then n pages of three older sessions
//! (`old-p<k>-<j>`) each. Loading any `old-` id other than `old-1` also replays a
//! thought and a finished tool call.
//! Loading an id it does not know waits 6 s, then fails with pi's
//! "Unknown sessionId" (a chat saved by an older client). Ids `fake-*` are known to every
//! run (a real agent keeps its sessions on disk), so a restarted fake can reopen them.
//!
//! Test switches: `FAKE_AGENT_AUTH=1` answers `session/list` and `session/new` with ACP's
//! `auth_required` (a logged-out agent); `FAKE_AGENT_STARTS=<file>` appends one line per
//! process start to that file.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::v1::schema::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, Implementation,
    InitializeRequest, InitializeResponse, ListSessionsRequest, McpServer, ListSessionsResponse,
    LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse,
    PermissionOption, PermissionOptionKind, PromptRequest, PromptResponse, ReadTextFileRequest,
    RequestPermissionOutcome, RequestPermissionRequest, SessionCapabilities, SessionInfo,
    SessionListCapabilities, SessionNotification, SessionUpdate, StopReason, ToolCall,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind, WriteTextFileRequest,
};
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::{Agent, Client, Result, Stdio};

#[derive(Default)]
struct State {
    sessions: Mutex<Vec<(String, PathBuf)>>,
    /// MCP servers each session got on new/load.
    mcp: Mutex<Vec<(String, Vec<McpServer>)>>,
    cancelled: AtomicBool,
}

fn main() -> Result<()> {
    if let Some(path) = std::env::var_os("FAKE_AGENT_STARTS") {
        use std::io::Write;
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path);
        let _ = file.and_then(|mut f| writeln!(f, "{}", std::process::id()));
    }
    let auth = std::env::var("FAKE_AGENT_AUTH").is_ok_and(|v| v == "1");
    let state = Arc::new(State::default());
    smol::block_on(
        Agent
            .v1()
            .name("fake-agent")
            .on_receive_request(
                async |req: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(req.protocol_version)
                            .agent_capabilities(
                                AgentCapabilities::new().load_session(true).session_capabilities(
                                    SessionCapabilities::new().list(SessionListCapabilities::new()),
                                ),
                            )
                            .agent_info(Implementation::new("fake-agent", "0.1.0")),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |req: NewSessionRequest, responder, _cx| {
                    if auth {
                        return responder.respond_with_error(agent_client_protocol::Error::auth_required());
                    }
                    let id = {
                        let mut list = state.sessions.lock().expect("state lock");
                        // The pid keeps ids apart across restarts.
                        let id = format!("fake-{}-{}", std::process::id(), list.len() + 1);
                        list.push((id.clone(), req.cwd));
                        state.mcp.lock().expect("state lock").push((id.clone(), req.mcp_servers));
                        id
                    };
                    responder.respond(NewSessionResponse::new(id))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |req: ListSessionsRequest, responder, _cx| {
                    if auth {
                        return responder.respond_with_error(agent_client_protocol::Error::auth_required());
                    }
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
                    let pages: usize = std::env::var("FAKE_AGENT_PAGES").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
                    let page: usize = req.cursor.as_deref().and_then(|c| c.strip_prefix("page-")).and_then(|n| n.parse().ok()).unwrap_or(0);
                    if page > 0 {
                        let infos = (1..=3)
                            .map(|j| {
                                SessionInfo::new(format!("old-p{page}-{j}"), home.clone())
                                    .title(format!("Older session {page}.{j}"))
                                    .updated_at(format!("2026-10-{:02}T1{j}:00:00Z", 28 - page))
                            })
                            .collect();
                        let next = (page < pages).then(|| format!("page-{}", page + 1));
                        return responder.respond(ListSessionsResponse::new(infos).next_cursor(next));
                    }
                    let mut infos = vec![
                        SessionInfo::new("old-1", "/tmp").title("Earlier work".to_string())
                            .updated_at("2026-11-12T09:30:00Z".to_string()),
                        SessionInfo::new("old-2", home.clone())
                            .title("---\nname: weekly\n---\n\nPlan the week".to_string())
                            .updated_at("2026-11-13T08:00:00Z".to_string()),
                        SessionInfo::new("old-3", format!("{home}/unity"))
                            .title("It is your turn: Unity's planning loop woke you.".to_string())
                            .updated_at("2026-11-10T17:45:00Z".to_string()),
                    ];
                    for (id, cwd) in state.sessions.lock().expect("state lock").iter() {
                        infos.push(SessionInfo::new(id.clone(), cwd.clone()));
                    }
                    let next = (pages > 0).then(|| "page-1".to_string());
                    responder.respond(ListSessionsResponse::new(infos).next_cursor(next))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |req: LoadSessionRequest, responder, cx: ConnectionTo<Client>| {
                    let id = req.session_id.clone();
                    let known = id.0.starts_with("old-")
                        || id.0.starts_with("fake-")
                        || state.sessions.lock().expect("state lock").iter().any(|(s, _)| s.as_str() == &*id.0);
                    if !known {
                        smol::Timer::after(Duration::from_secs(6)).await;
                        return responder.respond_with_error(
                            agent_client_protocol::Error::invalid_params()
                                .data(serde_json::json!(format!("Unknown sessionId: {}", id.0))),
                        );
                    }
                    state.sessions.lock().expect("state lock").push((id.0.to_string(), req.cwd));
                    state.mcp.lock().expect("state lock").push((id.0.to_string(), req.mcp_servers));
                    say(&cx, &id, SessionUpdate::UserMessageChunk(chunk("earlier question")))?;
                    if id.0.starts_with("old-") && &*id.0 != "old-1" {
                        say(&cx, &id, SessionUpdate::AgentThoughtChunk(chunk("Looking back at it.")))?;
                        say(&cx, &id, SessionUpdate::ToolCall(
                            ToolCall::new("old-tool", "Read plan.md").kind(ToolKind::Read).status(ToolCallStatus::Completed),
                        ))?;
                    }
                    say(&cx, &id, SessionUpdate::AgentMessageChunk(chunk("earlier answer")))?;
                    responder.respond(LoadSessionResponse::new())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_notification(
                async |_n: CancelNotification, _cx| {
                    state.cancelled.store(true, Ordering::SeqCst);
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_request(
                async |req: PromptRequest, responder, cx: ConnectionTo<Client>| {
                    // The turn waits on the client (permission, fs), so it must run off the dispatch loop.
                    let state = state.clone();
                    let cx2 = cx.clone();
                    cx.spawn(async move {
                        let reason = turn(&cx2, &state, req).await?;
                        responder.respond(PromptResponse::new(reason))
                    })
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(Stdio::new()),
    )
}

fn chunk(text: &str) -> ContentChunk {
    ContentChunk::new(ContentBlock::from(text))
}

fn say(
    cx: &ConnectionTo<Client>,
    session: &agent_client_protocol::v1::schema::SessionId,
    update: SessionUpdate,
) -> Result<()> {
    cx.send_notification(SessionNotification::new(session.clone(), update))
}

async fn turn(cx: &ConnectionTo<Client>, state: &State, req: PromptRequest) -> Result<StopReason> {
    let sid = req.session_id.clone();
    let prompt = req
        .prompt
        .iter()
        .find_map(|b| match b {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .unwrap_or_default();

    if prompt == "pid" {
        say(cx, &sid, SessionUpdate::AgentMessageChunk(chunk(&std::process::id().to_string())))?;
        return Ok(StopReason::EndTurn);
    }
    if prompt.starts_with("notice") {
        say(cx, &sid, SessionUpdate::AgentMessageChunk(chunk("New version available: 9.9.9 (installed 0.1.0). Run: npm i -g pi\n")))?;
        say(cx, &sid, SessionUpdate::AgentMessageChunk(chunk("Reply after the notice.")))?;
        return Ok(StopReason::EndTurn);
    }
    if let Some(line) = prompt.lines().map(str::trim).find(|l| l.starts_with("mcp ")) {
        let rest = line["mcp ".len()..].trim().to_string();
        let (tool, args) = rest.split_once(' ').map(|(t, a)| (t.to_string(), a.to_string())).unwrap_or((rest.clone(), "{}".into()));
        let servers = state.mcp.lock().expect("state lock").iter().rev()
            .find(|(id, _)| *id == *sid.0).map(|(_, s)| s.clone()).unwrap_or_default();
        say(cx, &sid, SessionUpdate::ToolCall(
            ToolCall::new("mcp-1", format!("ados: {tool}")).kind(ToolKind::Other).status(ToolCallStatus::InProgress),
        ))?;
        let t = tool.clone();
        let result = smol::unblock(move || call_mcp(&servers, &t, &args)).await;
        let (status, text) = match result {
            Ok(text) => (ToolCallStatus::Completed, text),
            Err(e) => (ToolCallStatus::Failed, e),
        };
        say(cx, &sid, SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("mcp-1", ToolCallUpdateFields::new().status(status))))?;
        say(cx, &sid, SessionUpdate::AgentMessageChunk(chunk(&format!("MCP {tool}: {text}"))))?;
        return Ok(StopReason::EndTurn);
    }
    if prompt == "wait" {
        state.cancelled.store(false, Ordering::SeqCst);
        say(cx, &sid, SessionUpdate::AgentMessageChunk(chunk("waiting")))?;
        for _ in 0..100 {
            if state.cancelled.load(Ordering::SeqCst) {
                return Ok(StopReason::Cancelled);
            }
            smol::Timer::after(Duration::from_millis(100)).await;
        }
        return Ok(StopReason::EndTurn);
    }

    say(cx, &sid, SessionUpdate::AgentThoughtChunk(chunk("Thinking about it.")))?;
    for part in ["Hello", ", ", "this is ", "the fake agent."] {
        say(cx, &sid, SessionUpdate::AgentMessageChunk(chunk(part)))?;
    }
    let title = "Write fake-note.txt";
    say(
        cx,
        &sid,
        SessionUpdate::ToolCall(
            ToolCall::new("tool-1", title).kind(ToolKind::Edit).status(ToolCallStatus::Pending),
        ),
    )?;
    let ask = RequestPermissionRequest::new(
        sid.clone(),
        ToolCallUpdate::new("tool-1", ToolCallUpdateFields::new().title(title.to_string())),
        vec![
            PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
            PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
        ],
    );
    let answer = cx.send_request(ask).block_task().await?;
    let chosen = match answer.outcome {
        RequestPermissionOutcome::Selected(s) => s.option_id.0.to_string(),
        _ => "cancelled".to_string(),
    };

    let update = |status| {
        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "tool-1",
            ToolCallUpdateFields::new().status(status),
        ))
    };
    if chosen == "allow" {
        say(cx, &sid, update(ToolCallStatus::InProgress))?;
        let cwd = state
            .sessions
            .lock()
            .expect("state lock")
            .iter()
            .find(|(id, _)| *id == *sid.0)
            .map(|(_, cwd)| cwd.clone())
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let path = cwd.join("fake-note.txt");
        cx.send_request(WriteTextFileRequest::new(sid.clone(), path.clone(), prompt.clone()))
            .block_task()
            .await?;
        let back = cx.send_request(ReadTextFileRequest::new(sid.clone(), path)).block_task().await?;
        say(cx, &sid, update(ToolCallStatus::Completed))?;
        say(cx, &sid, SessionUpdate::AgentMessageChunk(chunk(&format!(" Permission: {chosen}. File says: {}", back.content))))?;
    } else {
        say(cx, &sid, update(ToolCallStatus::Failed))?;
        say(cx, &sid, SessionUpdate::AgentMessageChunk(chunk(&format!(" Permission: {chosen}."))))?;
    }
    Ok(StopReason::EndTurn)
}

/// Spawn the session's first stdio MCP server and run one tool, the way an agent does.
/// Returns the tool's text, or a sentence saying what went wrong.
fn call_mcp(servers: &[McpServer], tool: &str, args: &str) -> std::result::Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    let server = servers.iter().find_map(|s| match s {
        McpServer::Stdio(s) => Some(s.clone()),
        _ => None,
    }).ok_or("this session got no stdio MCP server")?;
    let args: serde_json::Value = serde_json::from_str(args).map_err(|e| format!("arguments are not JSON: {e}"))?;
    let mut child = std::process::Command::new(&server.command)
        .args(&server.args)
        .envs(server.env.iter().map(|e| (e.name.clone(), e.value.clone())))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", server.command.display()))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let messages = [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fake-agent","version":"0.1.0"}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":tool,"arguments":args}}),
    ];
    for m in &messages {
        writeln!(stdin, "{m}").map_err(|e| format!("could not write to the MCP server: {e}"))?;
    }
    let mut answer = Err("the MCP server closed without answering".to_string());
    for line in BufReader::new(stdout).lines().map_while(std::result::Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if let Some(err) = v.get("error") {
            answer = Err(err["message"].as_str().unwrap_or("error").to_string());
            break;
        }
        if v["id"] == 2 {
            let text = v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
            answer = if v["result"]["isError"] == true { Err(text) } else { Ok(text) };
            break;
        }
    }
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    answer
}
