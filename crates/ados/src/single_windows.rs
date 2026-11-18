//! Windows: one aDOs per data folder (`single.rs` is the Linux one). The first launch owns the
//! named pipe `\\.\pipe\ados-<hash of the folder>`; a later launch on the same folder connects,
//! asks it to raise its window, then exits. The pipe goes away with the process, so a crash
//! never blocks the next start.
use gpui_kit::*;
use std::{
    fs::{File, OpenOptions},
    hash::{DefaultHasher, Hash, Hasher},
    io::{Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::Path,
    time::Duration,
};
use windows::{
    core::{HRESULT, HSTRING},
    Win32::{
        Foundation::{ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, HANDLE},
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX},
        System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
            PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        },
        UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY},
    },
};

/// Held by the running app for its whole life.
pub struct Guard {
    pipe: File,
    name: String,
}

pub enum Start {
    /// This is the only aDOs on the folder (or the guard could not be set up: start anyway).
    First(Option<Guard>),
    /// Another aDOs on the folder was asked to show itself.
    HandedOff,
    /// Another aDOs holds the folder but did not answer.
    Busy,
}

fn pipe_name(home: &Path) -> String {
    let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let mut hasher = DefaultHasher::new();
    home.hash(&mut hasher);
    format!(r"\\.\pipe\ados-{:016x}", hasher.finish())
}

/// One server end of the pipe. `first` fails with access denied when the pipe already exists.
fn create(name: &str, first: bool) -> std::io::Result<File> {
    let open = if first { PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE } else { PIPE_ACCESS_DUPLEX };
    let mode = PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    // SAFETY: plain Win32 call with owned arguments; the result is checked before use.
    let handle = unsafe { CreateNamedPipeW(&HSTRING::from(name), open, mode, PIPE_UNLIMITED_INSTANCES, 64, 64, 0, None) };
    if handle.is_invalid() {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a valid handle nobody else owns; the File closes it.
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

pub fn claim(home: &Path) -> Start {
    let name = pipe_name(home);
    match create(&name, true) {
        Ok(pipe) => Start::First(Some(Guard { pipe, name })),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32) => {
            // This launch may take the foreground right now; let the running aDOs use that.
            // SAFETY: no pointers; failure only means the window may flash instead of rising.
            let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
            // The running app may still be starting, or busy with another launch.
            for _ in 0..30 {
                if let Ok(mut pipe) = OpenOptions::new().read(true).write(true).open(&name) {
                    if pipe.write_all(b"raise\n").is_ok() {
                        let mut reply = [0u8; 3];
                        let _ = pipe.read(&mut reply);
                        return Start::HandedOff;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Start::Busy
        }
        Err(e) => {
            eprintln!("Could not set up the one-window guard: {e}. Starting anyway.");
            Start::First(None)
        }
    }
}

/// Raise and focus the window whenever a later launch on this folder asks.
pub fn serve(guard: Guard, cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<()>();
    let Guard { mut pipe, name } = guard;
    std::thread::spawn(move || loop {
        // SAFETY: the handle belongs to `pipe`, which outlives the call.
        let connected = match unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) } {
            Ok(()) => true,
            Err(e) => e.code() == HRESULT::from_win32(ERROR_PIPE_CONNECTED.0),
        };
        // Keep the name taken with a fresh instance before this one closes.
        let Ok(next) = create(&name, false) else { break };
        if connected {
            let mut buf = [0u8; 16];
            if matches!(pipe.read(&mut buf), Ok(n) if buf[..n].starts_with(b"raise")) {
                if tx.send_blocking(()).is_err() {
                    break;
                }
                let _ = pipe.write_all(b"ok\n");
            }
        }
        pipe = next;
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
