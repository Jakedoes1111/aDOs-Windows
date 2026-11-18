//! The socket side: a Unix socket only the owner can open (on Windows an AF_UNIX socket
//! through `uds_windows`, in the owner's own profile folder), a per-launch secret, and one
//! thread per connection. Tool calls go to the GPUI thread over a channel and are waited
//! for with the tool's timeout, so an agent never hangs on aDOs.
#[cfg(unix)]
use std::io::Read;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::protocol::{self, ToolResult};
use crate::tools::Tool;

/// The bridge's first line: `ADOS-MCP-SECRET <hex>`.
pub const SECRET_PREFIX: &str = "ADOS-MCP-SECRET ";

/// One tool call for the GPUI thread to answer.
pub struct Call {
    pub tool: &'static Tool,
    pub args: Value,
    pub reply: mpsc::Sender<ToolResult>,
}

pub struct Listening {
    pub socket: PathBuf,
    pub secret: String,
}

/// 32 random bytes as hex, from the kernel.
pub fn new_secret() -> Result<String> {
    let mut bytes = [0u8; 32];
    #[cfg(unix)]
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .context("Could not read random bytes for the MCP secret")?;
    #[cfg(windows)]
    {
        use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
        // SAFETY: fills the buffer it is given; the status is checked.
        let status = unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        anyhow::ensure!(status.0 >= 0, "Could not get random bytes for the MCP secret (status {:#x})", status.0);
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Where this run's socket lives: `$XDG_RUNTIME_DIR/ados`, else `ADOS_HOME/run`.
/// Windows: `%LOCALAPPDATA%\ados\run` (machine-only state, never roams), unless `ADOS_HOME` is set.
#[cfg(windows)]
pub fn socket_dir() -> PathBuf {
    match std::env::var_os("LOCALAPPDATA") {
        Some(dir) if !dir.is_empty() && std::env::var_os("ADOS_HOME").is_none() => PathBuf::from(dir).join("ados").join("run"),
        _ => ados_core::paths::home().join("run"),
    }
}

#[cfg(unix)]
pub fn socket_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("ados"),
        _ => ados_core::paths::home().join("run"),
    }
}

/// Bind the socket and start accepting. Calls arrive on `calls`.
pub fn listen(calls: async_channel::Sender<Call>) -> Result<Listening> {
    let dir = socket_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("Could not create {}", dir.display()))?;
    // Windows: the folder is under the owner's profile, which only they (and admins) can open.
    #[cfg(unix)]
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    remove_stale(&dir);
    let socket = dir.join(format!("mcp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).with_context(|| format!("Could not listen on {}", socket.display()))?;
    #[cfg(unix)]
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    let secret = new_secret()?;
    write_current(&dir, &socket, &secret)?;
    let expected = secret.clone();
    std::thread::Builder::new().name("ados-mcp-listen".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let (calls, expected) = (calls.clone(), expected.clone());
            let _ = std::thread::Builder::new()
                .name("ados-mcp-conn".into())
                .spawn(move || serve(stream, &expected, calls));
        }
    })?;
    Ok(Listening { socket, secret })
}

/// `current.json` (owner-only): this run's socket and secret, for MCP clients aDOs does not
/// start itself (pi through pi-mcp-adapter, a hand-configured Claude Code). The bridge reads
/// it when its env does not carry them. Same trust as the 0600 socket: only the owner reads it.
pub const CURRENT_FILE: &str = "current.json";

fn write_current(dir: &Path, socket: &Path, secret: &str) -> Result<()> {
    let path = dir.join(CURRENT_FILE);
    let tmp = dir.join(format!("{CURRENT_FILE}.{}", std::process::id()));
    let body = serde_json::json!({ "socket": socket, "secret": secret, "pid": std::process::id() });
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut f = options.open(&tmp).with_context(|| format!("Could not write {}", tmp.display()))?;
        f.write_all(body.to_string().as_bytes())?;
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("Could not write {}", path.display()))?;
    Ok(())
}

/// Read `current.json`: (socket, secret), if the aDOs that wrote it is still running.
pub fn read_current() -> Result<(PathBuf, String), String> {
    let path = socket_dir().join(CURRENT_FILE);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("aDOs is not running (no {}).", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{} is damaged: {e}", path.display()))?;
    let pid = v["pid"].as_u64().unwrap_or(0);
    if !alive(pid) {
        return Err("aDOs is not running (its last run has ended).".into());
    }
    match (v["socket"].as_str(), v["secret"].as_str()) {
        (Some(sock), Some(secret)) => Ok((PathBuf::from(sock), secret.to_string())),
        _ => Err(format!("{} is damaged.", path.display())),
    }
}

/// Sockets left by aDOs runs that are no longer alive.
fn remove_stale(dir: &Path) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let pid = name.strip_prefix("mcp-").and_then(|n| n.strip_suffix(".sock"));
        if let Some(pid) = pid {
            if !pid.parse().is_ok_and(alive) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Whether the aDOs run with this process id is still going.
#[cfg(unix)]
fn alive(pid: u64) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(windows)]
fn alive(pid: u64) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let Ok(pid) = u32::try_from(pid) else { return false };
    // SAFETY: plain Win32 calls; the handle is closed before returning.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let running = GetExitCodeProcess(process, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(process);
        running
    }
}

/// Constant-time comparison, so the secret cannot be guessed byte by byte.
pub fn secret_matches(line: &str, expected: &str) -> bool {
    let Some(given) = line.trim_end().strip_prefix(SECRET_PREFIX) else { return false };
    given.len() == expected.len() && given.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn serve(stream: UnixStream, expected: &str, calls: async_channel::Sender<Call>) {
    let Ok(read_half) = stream.try_clone() else { return };
    let writer = Arc::new(Mutex::new(stream));
    let mut lines = BufReader::new(read_half).lines();
    let first = lines.next().and_then(|l| l.ok()).unwrap_or_default();
    if !secret_matches(&first, expected) {
        let refusal = protocol::error(Value::Null, -32001, "Refused: this connection did not send aDOs's MCP secret.");
        write(&writer, &refusal);
        return;
    }
    for line in lines {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        // Tool calls can take a while; answer each from its own thread so pings still flow.
        let (writer, calls) = (writer.clone(), calls.clone());
        let _ = std::thread::Builder::new().name("ados-mcp-call".into()).spawn(move || {
            let reply = protocol::handle_line(&line, &mut |tool, args| dispatch(&calls, tool, args));
            if let Some(reply) = reply {
                write(&writer, &reply);
            }
        });
    }
}

fn write(writer: &Mutex<UnixStream>, value: &Value) {
    let mut out = writer.lock().unwrap_or_else(|e| e.into_inner());
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

/// Hand the call to the GPUI thread and wait at most the tool's timeout.
pub fn dispatch(calls: &async_channel::Sender<Call>, tool: &'static Tool, args: Value) -> ToolResult {
    let (reply, answer) = mpsc::channel();
    calls
        .send_blocking(Call { tool, args, reply })
        .map_err(|_| "aDOs is closing, so it cannot run tools now.".to_string())?;
    match answer.recv_timeout(Duration::from_secs(tool.timeout)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(format!("aDOs did not answer {} within {} seconds.", tool.name, tool.timeout))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(format!("aDOs dropped the {} call.", tool.name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secret_must_match_exactly() {
        assert!(secret_matches("ADOS-MCP-SECRET abc\n", "abc"));
        assert!(!secret_matches("ADOS-MCP-SECRET abd", "abc"));
        assert!(!secret_matches("ADOS-MCP-SECRET ab", "abc"));
        assert!(!secret_matches(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#, "abc"));
        assert!(!secret_matches("", "abc"));
        assert_eq!(new_secret().expect("secret").len(), 64);
    }

    #[test]
    fn dispatch_times_out_with_a_sentence() {
        let (tx, rx) = async_channel::unbounded();
        let tool: &'static Tool = Box::leak(Box::new(Tool { timeout: 0, ..crate::tools::TOOLS[0] }));
        let err = dispatch(&tx, tool, json!({})).expect_err("timeout");
        assert_eq!(err, "aDOs did not answer actions_list within 0 seconds.");
        drop(rx);
        assert!(dispatch(&tx, tool, json!({})).expect_err("closed").contains("closing"));
    }

    #[test]
    fn dispatch_returns_the_answer() {
        let (tx, rx) = async_channel::unbounded::<Call>();
        std::thread::spawn(move || {
            if let Ok(call) = rx.recv_blocking() {
                let _ = call.reply.send(Ok(json!({ "ran": call.tool.name })));
            }
        });
        let tool = crate::tools::find("actions_list").expect("tool");
        assert_eq!(dispatch(&tx, tool, json!({})), Ok(json!({ "ran": "actions_list" })));
    }
}
