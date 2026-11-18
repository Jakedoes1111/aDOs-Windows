use gpui_kit::{Pixels, Size};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The assistant overlay's content coordinates start at the window's client
/// area. The rail is 72px and the top bar 48px; the card keeps an 8px gap
/// below and right of them.
pub(super) const SAFE_LEFT: f32 = 80.;
pub(super) const SAFE_TOP: f32 = 56.;

/// The bubble docks in the top bar, a strip no panel ever draws into, so it can
/// never sit on (and catch the clicks of) a panel's controls. It is vertically
/// centred in the 48px bar and moves only along it.
pub(super) const BUBBLE_TOP: f32 = 8.;
/// The left end of the bar holds the layout switcher, the "modified" dot and
/// Save layout; the bubble stays right of them.
pub(super) const BAR_LEFT: f32 = 420.;

/// The bubble's allowed x range in the top bar. `right` is the width the bar's
/// own right-hand buttons need.
fn bar_limits(size: Size<Pixels>, right: f32) -> (f32, f32) {
    let width = f32::from(size.width);
    let max_x = (width - right - Saved::BUBBLE).max(0.);
    (BAR_LEFT.min(max_x), max_x)
}

/// Which window edge an offset is measured from. `Start` is left, `End` right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(super) enum Anchor {
    #[default]
    Start,
    End,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Saved {
    pub open: bool,
    /// Offset from `anchor_x` along the top bar. Legacy files store absolute pixels here
    /// and `anchored` is false, so the first layout migrates them to an edge offset.
    /// Older files also hold `y`/`anchor_y`; the bubble no longer leaves the top bar,
    /// so those are ignored.
    pub x: f32,
    pub anchor_x: Anchor,
    pub anchored: bool,
    /// The chosen backend id from `backends.toml`.
    pub backend: Option<String>,
    /// The open chat (ACP session id) and the folder it runs in.
    pub chat: Option<String>,
    pub chat_cwd: Option<std::path::PathBuf>,
    pub drafts: BTreeMap<String, String>,
    pub this_module: bool,
    pub minimized: bool,
    /// The chats list shows every folder's chats, not just the home folder's.
    pub all_folders: bool,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            open: false,
            // Resolve against the first window's viewport; old numeric positions still load.
            x: -1.,
            anchor_x: Anchor::Start,
            anchored: false,
            backend: None,
            chat: None,
            chat_cwd: None,
            drafts: BTreeMap::new(),
            this_module: true,
            minimized: false,
            all_folders: false,
        }
    }
}
impl Saved {
    /// The bubble's diameter; positions are stored for its top-left corner.
    pub const BUBBLE: f32 = 32.;

    /// Resolve the bubble's left edge in window-content coordinates (its top is
    /// always `BUBBLE_TOP`). `right` is the width the top bar's right buttons need.
    pub fn bubble_x(&self, size: Size<Pixels>, right: f32) -> f32 {
        let width = f32::from(size.width);
        let x = match self.anchor_x {
            Anchor::Start => self.x,
            Anchor::End => width - Self::BUBBLE - self.x,
        };
        let (min_x, max_x) = bar_limits(size, right);
        x.clamp(min_x, max_x)
    }

    /// Store the bubble's left edge as the offset from the nearer window edge.
    pub fn set_bubble_x(&mut self, x: f32, size: Size<Pixels>, right: f32) {
        let width = f32::from(size.width);
        let (min_x, max_x) = bar_limits(size, right);
        let x = x.clamp(min_x, max_x);
        if x + Self::BUBBLE / 2. <= width / 2. {
            self.anchor_x = Anchor::Start;
            self.x = x;
        } else {
            self.anchor_x = Anchor::End;
            self.x = (width - Self::BUBBLE - x).max(0.);
        }
        self.anchored = true;
    }

    /// Convert a legacy absolute position (or the old `-1` sentinel) to an edge offset.
    /// The default docks the bubble at the right end of the bar, next to its buttons.
    pub fn ensure_anchored(&mut self, size: Size<Pixels>, right: f32) {
        if self.anchored {
            return;
        }
        if self.x < 0. {
            self.anchor_x = Anchor::End;
            self.x = 0.;
            self.anchored = true;
        } else {
            let x = self.x;
            self.set_bubble_x(x, size, right);
        }
    }

    pub fn load() -> (Self, Option<String>) {
        let path = ados_core::paths::state_file("assistant");
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(saved) => (saved, None),
                Err(e) => (
                    Self::default(),
                    Some(format!("Couldn't read assistant state: {e}")),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (
                Self::default(),
                Some(format!("Couldn't read assistant state: {e}")),
            ),
        }
    }
    pub fn save(&self) -> anyhow::Result<()> {
        let path = ados_core::paths::state_file("assistant");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{px, size};

    fn bounds(width: f32, height: f32) -> Size<Pixels> {
        size(px(width), px(height))
    }

    const RIGHT: f32 = 100.;

    #[test]
    fn default_docks_at_right_end_of_top_bar() {
        let mut saved = Saved::default();
        saved.ensure_anchored(bounds(1280., 800.), RIGHT);
        assert_eq!(saved.bubble_x(bounds(1280., 800.), RIGHT), 1280. - RIGHT - 32.);
        // Resizing keeps it docked beside the bar's buttons.
        assert_eq!(saved.bubble_x(bounds(900., 600.), RIGHT), 900. - RIGHT - 32.);
    }

    #[test]
    fn old_bottom_right_position_moves_into_the_bar() {
        // A file from before the bubble docked: anchored 24px from the bottom-right.
        let saved: Saved =
            serde_json::from_str(r#"{"x":24.0,"y":24.0,"anchor_x":"end","anchor_y":"end","anchored":true}"#)
                .unwrap_or_else(|_| Saved::default());
        assert_eq!(saved.bubble_x(bounds(900., 600.), RIGHT), 900. - RIGHT - 32.);
    }

    #[test]
    fn bubble_never_leaves_the_free_part_of_the_bar() {
        let mut saved = Saved::default();
        saved.set_bubble_x(0., bounds(1000., 800.), RIGHT);
        assert_eq!(saved.bubble_x(bounds(1000., 800.), RIGHT), BAR_LEFT);
        saved.set_bubble_x(5000., bounds(1000., 800.), RIGHT);
        assert_eq!(saved.bubble_x(bounds(1000., 800.), RIGHT), 1000. - RIGHT - 32.);
        // A middle position is kept, and anchored to the nearer edge.
        saved.set_bubble_x(600., bounds(1000., 800.), RIGHT);
        assert_eq!(saved.anchor_x, Anchor::End);
        assert_eq!(saved.bubble_x(bounds(1000., 800.), RIGHT), 600.);
    }

    #[test]
    fn tiny_window_keeps_bubble_inside() {
        let saved = Saved::default();
        assert_eq!(saved.bubble_x(bounds(300., 200.), RIGHT), 300. - RIGHT - 32.);
    }
}
