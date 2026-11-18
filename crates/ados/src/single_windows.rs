//! Windows: no one-window guard yet, every launch starts (`single.rs` is the Linux one).
// ponytail: stub until port step 2 (lock file + named pipe "raise"), see docs/design/windows-port.md.
use gpui_kit::App;
use std::path::Path;

pub struct Guard;

#[allow(dead_code)]
pub enum Start {
    First(Option<Guard>),
    HandedOff,
    Busy,
}

pub fn claim(_home: &Path) -> Start {
    Start::First(None)
}

pub fn serve(_guard: Guard, _cx: &mut App) {}
