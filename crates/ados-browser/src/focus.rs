//! Who holds the keyboard inside the Browser (stage 3 focus contract F1-F8, F10).
//! Pure decisions only; `lib.rs` feeds them GPUI state and `xembed.rs` applies X focus.
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The two places inside the Browser that can own the keyboard. Remembered and saved so
/// entering the Browser (F2) and launch (F8) go back to where the owner left it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Place {
    Page,
    Address,
}

/// Where the keyboard goes when the Browser is entered. Nothing remembered yet: the page if
/// one is loaded, the address bar if the page is blank (F2, F8).
pub fn entry_place(remembered: Option<Place>, url: &str) -> Place {
    match remembered {
        Some(place) => place,
        None if is_blank(url) => Place::Address,
        None => Place::Page,
    }
}

pub fn is_blank(url: &str) -> bool {
    let url = url.trim();
    url.is_empty() || url == "about:blank"
}

/// What decides whether the page's native window should hold X focus. Every GPUI path
/// (module switch, overlay, layout, zoom, dock move, hide/show) re-evaluates this one rule.
/// The OS window's own activation is not an input: GPUI reports the window inactive whenever
/// X focus moves into the page child, so the X thread restores the wanted focus itself when
/// focus comes back to aDOs from another application (F7, `xembed.rs`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Owner {
    /// GPUI focus is on the page proxy.
    pub page_focused: bool,
    /// The shell's active module is the Browser (rail and tab marked).
    pub module_active: bool,
    /// A palette, menu or card floats over the workspace (F5, F6).
    pub overlay: bool,
    /// The engine window is attached and its dock tab is the visible one.
    pub shown: bool,
}
impl Owner {
    pub fn page_has_keyboard(self) -> bool {
        self.page_focused && self.module_active && !self.overlay && self.shown
    }
}

/// Keys typed for a page that is about to take focus wait at most this long, then are dropped.
/// Longer than the 1 s held-key deadline in `xembed.rs`. The page being shown has an X event
/// (MapNotify); a page that never shows has none.
pub const TYPED_WAIT: Duration = Duration::from_millis(1500);

/// One engine frame: servoshell picks a click's target from the hover it saw on its previous
/// frame, so a press arriving with the pointer's first entry would miss (audit G3).
pub const HOVER_FRAME: Duration = Duration::from_millis(40);

/// True when the press came so soon after the pointer entered the page (X server ms) that
/// the engine cannot have drawn a frame with that hover yet.
pub fn hover_unseen(entered_at: u32, pressed_at: u32) -> bool {
    entered_at != 0 && pressed_at.wrapping_sub(entered_at) < HOVER_FRAME.as_millis() as u32
}

/// The X keysym for a key GPUI delivered to the page proxy. Esc and Tab stay with aDOs
/// (the Esc ladder and Tab order), and keys without a plain keysym are not passed on.
pub fn typed_keysym(key: &str) -> Option<u32> {
    if matches!(key, "escape" | "tab") { return None; }
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) { return Some(c as u32); }
    crate::xembed::chord_keysym(key).filter(|(mods, _)| *mods == Default::default()).map(|(_, sym)| sym)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_remembers_then_falls_back_on_blank() {
        assert_eq!(entry_place(Some(Place::Address), "https://a.b"), Place::Address);
        assert_eq!(entry_place(Some(Place::Page), "about:blank"), Place::Page);
        assert_eq!(entry_place(None, "about:blank"), Place::Address);
        assert_eq!(entry_place(None, " "), Place::Address);
        assert_eq!(entry_place(None, "http://127.0.0.1:8765/"), Place::Page);
    }

    #[test]
    fn page_keyboard_needs_every_condition() {
        let all = Owner { page_focused: true, module_active: true, overlay: false, shown: true };
        assert!(all.page_has_keyboard());
        assert!(!Owner { page_focused: false, ..all }.page_has_keyboard());
        assert!(!Owner { module_active: false, ..all }.page_has_keyboard());
        assert!(!Owner { overlay: true, ..all }.page_has_keyboard());
        assert!(!Owner { shown: false, ..all }.page_has_keyboard());
    }

    #[test]
    fn typed_keys() {
        assert_eq!(typed_keysym("q"), Some('q' as u32));
        assert_eq!(typed_keysym("-"), Some('-' as u32));
        assert_eq!(typed_keysym("space"), Some(0x20));
        assert_eq!(typed_keysym("enter"), Some(0xff0d));
        assert_eq!(typed_keysym("escape"), None);
        assert_eq!(typed_keysym("tab"), None);
    }

    #[test]
    fn hover_frame() {
        assert!(hover_unseen(1000, 1000));
        assert!(hover_unseen(1000, 1039));
        assert!(!hover_unseen(1000, 1040));
        assert!(!hover_unseen(0, 5));
        // X time wraps after ~49 days.
        assert!(hover_unseen(u32::MAX - 5, 10));
    }
}
