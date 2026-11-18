//! The stdio shim agents run (`ados mcp-bridge`): it connects to the running aDOs's socket,
//! sends the secret, then copies stdin to the socket and the socket to stdout.
//! Every ACP agent must accept a stdio MCP server, so this works with all of them.
//! Without the env (a client the owner configured by hand, e.g. pi through pi-mcp-adapter),
//! it reads the running aDOs's `current.json` instead.
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(windows)]
use uds_windows::UnixStream;

pub const SOCKET_ENV: &str = "ADOS_MCP_SOCKET";
pub const SECRET_ENV: &str = "ADOS_MCP_SECRET";

/// Run the bridge; returns the process exit code. Errors go to stderr (agents log it).
pub fn run() -> i32 {
    let (socket, secret) = match (std::env::var_os(SOCKET_ENV), std::env::var(SECRET_ENV).ok()) {
        (Some(socket), Some(secret)) => (std::path::PathBuf::from(socket), secret),
        _ => match crate::server::read_current() {
            Ok(found) => found,
            Err(e) => {
                eprintln!("ados mcp-bridge: {e}");
                return 2;
            }
        },
    };
    let mut stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ados mcp-bridge: aDOs is not reachable at {}: {e}", socket.to_string_lossy());
            return 1;
        }
    };
    if writeln!(stream, "{}{secret}", crate::server::SECRET_PREFIX).is_err() {
        eprintln!("ados mcp-bridge: aDOs closed the connection.");
        return 1;
    }
    let Ok(mut from_ados) = stream.try_clone() else { return 1 };
    let out = std::thread::spawn(move || {
        let mut stdout = std::io::stdout();
        let mut buf = [0u8; 8192];
        while let Ok(n) = from_ados.read(&mut buf) {
            if n == 0 || stdout.write_all(&buf[..n]).and_then(|()| stdout.flush()).is_err() {
                break;
            }
        }
    });
    let mut stdin = std::io::stdin();
    let mut buf = [0u8; 8192];
    while let Ok(n) = stdin.read(&mut buf) {
        if n == 0 || stream.write_all(&buf[..n]).is_err() {
            break;
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = out.join();
    0
}
