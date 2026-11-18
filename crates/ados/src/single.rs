//! One aDOs per data folder. The first launch holds `ADOS_HOME/run/ados.lock` and listens
//! on an abstract socket named after the folder; a later launch on the same folder asks it
//! to raise its window, then exits. The kernel drops the lock when a process dies, so a
//! crash never leaves a lock that blocks the next start.
use gpui_kit::*;
use std::{
    fs::{File, OpenOptions},
    hash::{DefaultHasher, Hash, Hasher},
    io::{Read, Write},
    os::linux::net::SocketAddrExt,
    os::unix::net::{SocketAddr, UnixListener, UnixStream},
    path::Path,
    time::Duration,
};

/// Held by the running app for its whole life.
pub struct Guard {
    _lock: File,
    listener: UnixListener,
}

pub enum Start {
    /// This is the only aDOs on the folder (or the guard could not be set up: start anyway).
    First(Option<Guard>),
    /// Another aDOs on the folder was asked to show itself.
    HandedOff,
    /// Another aDOs holds the folder but did not answer.
    Busy,
}

fn socket_name(home: &Path) -> String {
    let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let mut hasher = DefaultHasher::new();
    home.hash(&mut hasher);
    format!("ados-{:016x}", hasher.finish())
}

pub fn claim(home: &Path) -> Start {
    let run = home.join("run");
    if let Err(e) = std::fs::create_dir_all(&run) {
        eprintln!("Could not make {}: {e}. Starting without the one-window guard.", run.display());
        return Start::First(None);
    }
    let lock = match OpenOptions::new().read(true).write(true).create(true).truncate(false).open(run.join("ados.lock")) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Could not open the aDOs lock: {e}. Starting without the one-window guard.");
            return Start::First(None);
        }
    };
    let name = socket_name(home);
    let addr = match SocketAddr::from_abstract_name(name.as_bytes()) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("Could not name the aDOs socket: {e}. Starting without the one-window guard.");
            return Start::First(None);
        }
    };
    match lock.try_lock() {
        Ok(()) => match UnixListener::bind_addr(&addr) {
            Ok(listener) => Start::First(Some(Guard { _lock: lock, listener })),
            Err(e) => {
                eprintln!("Could not listen for a second launch: {e}. Starting anyway.");
                Start::First(None)
            }
        },
        Err(std::fs::TryLockError::WouldBlock) => {
            // The running app may still be starting: give it a moment to listen.
            for _ in 0..30 {
                if let Ok(mut stream) = UnixStream::connect_addr(&addr) {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    if stream.write_all(b"raise\n").is_ok() {
                        let mut reply = [0u8; 3];
                        let _ = stream.read(&mut reply);
                        return Start::HandedOff;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Start::Busy
        }
        Err(e) => {
            eprintln!("Could not lock the aDOs data folder: {e}. Starting without the one-window guard.");
            Start::First(None)
        }
    }
}

/// Raise and focus the window whenever a later launch on this folder asks.
pub fn serve(guard: Guard, cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<()>();
    let Guard { _lock, listener } = guard;
    std::thread::spawn(move || {
        let _lock = _lock;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = [0u8; 16];
            if matches!(stream.read(&mut buf), Ok(n) if buf[..n].starts_with(b"raise")) {
                if tx.send_blocking(()).is_err() {
                    break;
                }
                let _ = stream.write_all(b"ok\n");
            }
        }
    });
    cx.spawn(async move |cx| {
        while rx.recv().await.is_ok() {
            cx.update(|cx| {
                for window in cx.windows() {
                    let _ = window.update(cx, |_, window, _| window.activate_window());
                }
                cx.activate(true);
            });
        }
    })
    .detach();
}
