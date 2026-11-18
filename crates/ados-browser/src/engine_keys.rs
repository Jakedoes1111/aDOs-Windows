//! Window-level shortcuts of the engines. The page must never trigger them: a new tab or window
//! in the engine leaves the aDOs toolbar driving a hidden one. `xembed.rs` grabs each chord on the
//! engine window and drops it, unless aDOs itself owns the chord (aDOs wins).
//!
//! Sources: Servo's `ports/servoshell/desktop/headed_window.rs` (`handle_intercepted_key_bindings`:
//! Ctrl+W, Ctrl+T, Ctrl+Q, Ctrl+1..9, Ctrl+PageUp/PageDown, Alt+Left/Right, Ctrl+[ and Ctrl+],
//! Ctrl+F9..F12) and `gui.rs` (Ctrl+L / Alt+D focus the engine's location bar); Chromium's documented
//! Linux shortcuts (new/close tab and window, reopen tab, tab switching, DevTools, view source, save,
//! open file, print, find, fullscreen, quit). Editing and page chords (copy, paste, select all, undo,
//! arrows, Tab, space, Enter, zoom, reload, F5) are deliberately not listed.
//! Chord spelling is the one `shortcuts::normalize` produces, parsed by `chord_keysym`.

pub const ENGINE_WINDOW_CHORDS: &[&str] = &[
    // New and closing tabs and windows, quit.
    "ctrl-t", "ctrl-shift-t", "ctrl-n", "ctrl-shift-n", "ctrl-w", "ctrl-shift-w", "ctrl-f4", "alt-f4",
    "ctrl-q", "ctrl-shift-q",
    // Switching tabs.
    "ctrl-tab", "ctrl-shift-tab", "ctrl-pageup", "ctrl-pagedown", "ctrl-shift-pageup", "ctrl-shift-pagedown",
    "ctrl-1", "ctrl-2", "ctrl-3", "ctrl-4", "ctrl-5", "ctrl-6", "ctrl-7", "ctrl-8", "ctrl-9",
    "alt-1", "alt-2", "alt-3", "alt-4", "alt-5", "alt-6", "alt-7", "alt-8", "alt-9",
    // Engine history chords that would bypass the aDOs Back/Forward guard.
    "alt-left", "alt-right", "ctrl-[", "ctrl-]",
    // DevTools, view source, engine debug overlays, fullscreen.
    "f12", "ctrl-shift-i", "ctrl-shift-j", "ctrl-shift-c", "ctrl-u", "f11",
    "ctrl-f9", "ctrl-f10", "ctrl-f11", "ctrl-f12",
    // Print, save, open file.
    "ctrl-p", "ctrl-s", "ctrl-o",
    // The engine's own address bar and find chords (find in page is a later stage: inert for now).
    "ctrl-l", "alt-d", "f6", "ctrl-f", "ctrl-g", "ctrl-shift-g", "f3", "shift-f3",
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_has_the_window_chords_and_not_the_editing_ones() {
        for must in ["ctrl-t", "ctrl-n", "ctrl-w", "ctrl-shift-t", "ctrl-q", "f12", "ctrl-p", "ctrl-s", "ctrl-u", "f11", "ctrl-f"] {
            assert!(ENGINE_WINDOW_CHORDS.contains(&must), "{must} missing");
        }
        for editing in ["ctrl-c", "ctrl-v", "ctrl-x", "ctrl-a", "ctrl-z", "ctrl-y", "ctrl-shift-z", "ctrl-enter", "tab", "space", "left", "f5", "ctrl-r", "ctrl--", "ctrl-0"] {
            assert!(!ENGINE_WINDOW_CHORDS.contains(&editing), "{editing} must reach the page");
        }
        let mut sorted = ENGINE_WINDOW_CHORDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ENGINE_WINDOW_CHORDS.len(), "duplicate chord");
    }
}
