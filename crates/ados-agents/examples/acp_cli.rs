//! Talk to one backend from `ADOS_HOME/backends.toml`.
//!
//! ```text
//! cargo run -p ados-agents --example acp_cli -- <backend-id> "<prompt>"
//! cargo run -p ados-agents --example acp_cli -- <backend-id> --list
//! ```
//! Prints streamed text and tool calls; permission prompts are answered on stdin.
use std::io::Write;
use std::sync::Arc;

use ados_agents::{config, AgentEvent, Backend, PermissionCallback, PermissionRequest};
use futures::FutureExt;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(id), Some(what)) = (args.first(), args.get(1)) else {
        eprintln!("Usage: acp_cli <backend-id> \"<prompt>\" | --list");
        std::process::exit(2);
    };
    if let Err(e) = smol::block_on(run(id, what)) {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

async fn run(id: &str, what: &str) -> anyhow::Result<()> {
    let backends = config::load()?;
    let Some(cfg) = config::find(&backends, id) else {
        let ids: Vec<_> = backends.iter().map(|b| b.id.as_str()).collect();
        anyhow::bail!("No backend \"{id}\" in {}. Known: {}", config::backends_file().display(), ids.join(", "));
    };
    let ask: PermissionCallback = Arc::new(|req| smol::unblock(move || ask_stdin(&req)).boxed());
    let backend = Backend::start(cfg, ask).await?;
    let info = backend.info();
    eprintln!(
        "[{} started: {}; list={} load={}]",
        info.name,
        info.agent.as_deref().unwrap_or("unnamed agent"),
        info.can_list_sessions,
        info.can_load_sessions
    );
    let cwd = std::env::current_dir()?;

    if what == "--list" {
        let mut cursor = None;
        loop {
            let (page, next) = backend.list_sessions(None, cursor).await?;
            for s in page {
                println!(
                    "{}  {}  {}  {}",
                    s.id,
                    s.updated_at.as_deref().unwrap_or("-"),
                    s.cwd.display(),
                    s.title.as_deref().unwrap_or("(untitled)")
                );
            }
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    let session = backend.new_session(&cwd, &ados_agents::ados_servers()).await?;
    eprintln!("[session {session}]");
    let events = backend.events();
    let mut prompt = Box::pin(backend.prompt(&session, what).fuse());
    let reason = loop {
        futures::select! {
            ev = events.recv().fuse() => match ev {
                Ok(ev) => print_event(&ev),
                Err(_) => anyhow::bail!("The event stream closed."),
            },
            r = prompt => break r?,
        }
    };
    while let Ok(ev) = events.try_recv() {
        print_event(&ev);
    }
    println!();
    eprintln!("[turn ended: {reason:?}]");
    Ok(())
}

fn print_event(ev: &AgentEvent) {
    let mut out = std::io::stdout();
    match ev {
        AgentEvent::Text { text, .. } => {
            let _ = write!(out, "{text}");
        }
        AgentEvent::Thought { text, .. } => {
            let _ = write!(out, "\n(thinking) {text}\n");
        }
        AgentEvent::UserText { text, .. } => {
            let _ = write!(out, "\n> {text}\n");
        }
        AgentEvent::ToolCall { id, title, kind, status, .. } => {
            let _ = write!(out, "\n[tool {id}] {title} ({kind}, {status:?})\n");
        }
        AgentEvent::ToolCallUpdate { id, title, status, .. } => {
            let title = title.as_deref().unwrap_or("");
            let status = status.map(|s| format!("{s:?}")).unwrap_or_default();
            let _ = write!(out, "\n[tool {id}] {title} {status}\n");
        }
        AgentEvent::Crashed { message } => {
            let _ = write!(out, "\n[crashed] {message}\n");
        }
    }
    let _ = out.flush();
}

/// Blocking: show the options on stderr and read a number from stdin. Anything else denies.
fn ask_stdin(req: &PermissionRequest) -> Option<String> {
    eprintln!("\n[permission] {}", if req.title.is_empty() { "The agent asks to run a tool." } else { &req.title });
    for (i, o) in req.options.iter().enumerate() {
        eprintln!("  {}) {} ({:?})", i + 1, o.name, o.kind);
    }
    eprint!("Choose 1-{} (anything else denies): ", req.options.len());
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return None;
    }
    let n: usize = line.trim().parse().ok()?;
    req.options.get(n.checked_sub(1)?).map(|o| o.id.clone())
}
