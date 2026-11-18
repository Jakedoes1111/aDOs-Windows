//! The GPUI side: start the socket, register aDOs's MCP server for new agent sessions,
//! and answer each tool call on the GPUI thread.
use std::rc::Rc;

use ados_agents::McpEntry;
use anyhow::{anyhow, Result};
use gpui_kit::component::{button::{Button, ButtonVariants}, notification::Notification, WindowExt};
use gpui_kit::*;
use serde_json::Value;

use crate::protocol::ToolResult;
use crate::server::{self, Call};
use crate::{bridge, browser_tools, data_tools, session_tools, shell_tools};

/// What the shell tells the server about itself.
pub struct Hooks {
    /// The current layout's name, as the shell last saved it.
    pub current_layout: fn() -> Option<String>,
}

pub(crate) struct HostHooks(pub Hooks);
impl Global for HostHooks {}

/// Start the server for this run. Agents started after this get it on `session/new`/`session/load`.
/// Clients aDOs does not start (the probe, pi's MCP extension) find it through `current.json`.
pub fn start(hooks: Hooks, cx: &mut App) -> Result<()> {
    let (tx, rx) = async_channel::unbounded::<Call>();
    let listening = server::listen(tx)?;
    let exe = std::env::current_exe().map_err(|e| anyhow!("Could not find the aDOs binary: {e}"))?;
    let socket = listening.socket.to_string_lossy().to_string();
    ados_agents::mcp::set_ados_server(Some(McpEntry {
        name: "ados".into(),
        command: exe,
        args: vec!["mcp-bridge".into()],
        env: vec![(bridge::SOCKET_ENV.into(), socket), (bridge::SECRET_ENV.into(), listening.secret)],
    }));
    cx.set_global(HostHooks(hooks));
    cx.spawn(async move |cx| {
        while let Ok(call) = rx.recv().await {
            cx.spawn(async move |cx| {
                let result = run(call.tool.name, call.args, cx).await;
                let _ = call.reply.send(result);
            })
            .detach();
        }
    })
    .detach();
    Ok(())
}

/// Every tool runs through here. A tool acts for the owner but never takes their keyboard:
/// whatever had focus before the call (usually the assistant's composer) has it after,
/// even when the call switched layouts or ran a focus action. If that control is gone
/// (a layout replaced its panel), the focus the call left stays.
async fn run(tool: &str, args: Value, cx: &mut AsyncApp) -> ToolResult {
    let before = with_window(cx, |window, cx| Ok(window.focused(cx))).ok().flatten();
    let result = run_tool(tool, args, cx).await;
    if let Some(before) = before {
        let _ = with_window(cx, |window, cx| {
            if window.focused(cx).as_ref() == Some(&before) { return Ok(()); }
            let after = window.focused(cx);
            window.focus(&before, cx);
            window.on_next_frame(move |window, cx| {
                if !before.contains_focused(window, cx) {
                    if let Some(after) = after { window.focus(&after, cx); }
                }
            });
            Ok(())
        });
    }
    result
}

async fn run_tool(tool: &str, args: Value, cx: &mut AsyncApp) -> ToolResult {
    let s = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    match tool {
        "sessions_list" => session_tools::list(args.get("backend").and_then(Value::as_str).map(str::to_string), cx).await,
        "sessions_read" => session_tools::read(s("id"), args.get("backend").and_then(Value::as_str).map(str::to_string), cx).await,
        "sessions_prompt" => with_window(cx, |w, cx| session_tools::prompt(&s("id"), &s("text"), w, cx)),
        t if t.starts_with("browser_") => browser_tools::run(t, &args, cx).await,
        "actions_run" => shell_tools::run_action(s("id"), cx).await,
        "layouts_switch" => shell_tools::switch_layout(s("name"), cx).await,
        "layouts_save" => shell_tools::save_layout(cx).await,
        _ => with_window(cx, |window, cx| match tool {
            "actions_list" => shell_tools::actions_list(cx),
            "context_active" => shell_tools::context_active(cx),
            "context_module" => shell_tools::context_module(&s("id"), cx),
            "context_all" => shell_tools::context_all(cx),
            "layouts_list" => shell_tools::layouts_list(cx),
            "notes_list" => data_tools::notes_list(),
            "notes_read" => data_tools::notes_read(&s("path")),
            "notes_create" => data_tools::notes_create(&s("title"), &s("body"), window, cx),
            "notes_append" => data_tools::notes_append(&s("path"), &s("text"), window, cx),
            "capture_save" => data_tools::capture_save(&s("text"), window, cx),
            "tasks_list" => data_tools::tasks_list(),
            "tasks_add" => data_tools::tasks_add(&s("text"), window, cx),
            "tasks_set_done" => {
                let done = args.get("done").and_then(Value::as_bool).unwrap_or(false);
                data_tools::tasks_set_done(&s("id"), done, window, cx)
            }
            "calendar_events_between" => data_tools::events_between(&s("start"), &s("end")),
            other => Err(format!("{other} is not implemented.")),
        }),
    }
}

/// Run `f` with the aDOs window (the active one, else the first).
pub(crate) fn with_window<R>(cx: &mut AsyncApp, f: impl FnOnce(&mut Window, &mut App) -> Result<R, String>) -> Result<R, String> {
    let handle = cx
        .update(|cx| cx.active_window().or_else(|| cx.windows().first().copied()))
        .ok_or_else(|| "aDOs has no open window.".to_string())?;
    handle
        .update(cx, |_, window, cx| f(window, cx))
        .map_err(|_| "The aDOs window closed.".to_string())?
}

/// Latest assistant toast; a new one replaces the previous.
struct AssistantToast;

pub(crate) type Undo = Rc<dyn Fn(&mut Window, &mut App)>;

/// Show "Assistant: …" in aDOs, with Undo when there is one (also on the shell's undo key).
/// Modules that only re-read their files on focus are told to look again, so the owner
/// sees the change without clicking. Runs after every change and every Undo.
fn refresh_modules(window: &mut Window, cx: &mut App) {
    ados_tasks::refresh(cx);
    ados_notes::panel::refresh_list(window, cx);
}

pub(crate) fn toast(message: String, undo: Option<Undo>, window: &mut Window, cx: &mut App) {
    refresh_modules(window, cx);
    let undo = undo.map(|undo| {
        Rc::new(move |window: &mut Window, cx: &mut App| {
            undo(window, cx);
            refresh_modules(window, cx);
        }) as Undo
    });
    let mut note = Notification::new().message(format!("Assistant: {message}")).id::<AssistantToast>()
        // Hint labels of controls under the toast stay hidden.
        .content(|_, _, _| ados_core::hints::cover_padded(ados_core::hints::Layer::Float, px(16.)).into_any_element());
    if let Some(undo) = undo {
        let shell_undo = undo.clone();
        ados_core::notifications::remember_undo(move |window, cx| shell_undo(window, cx), cx);
        note = note.autohide(false).action(move |_, _, _| {
            let undo = undo.clone();
            Button::new("assistant-undo").label("Undo").ghost().child(ados_core::hints::mark_in(ados_core::hints::Layer::Float)).on_click(move |_, window, cx| {
                window.remove_notification::<AssistantToast>(cx);
                undo(window, cx);
            })
        });
    }
    window.push_notification(note, cx);
}

/// A follow-up message after Undo, or when Undo itself fails.
pub(crate) fn notice(message: String, window: &mut Window, cx: &mut App) {
    window.push_notification(Notification::new().message(message).id::<AssistantToast>(), cx);
}
