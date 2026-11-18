//! Where the bubble and the card sit. The bubble docks in the top bar (see
//! `state::BUBBLE_TOP`); the card hangs below it.
use super::assistant::Assistant;
use super::state::{Saved, BUBBLE_TOP, SAFE_LEFT, SAFE_TOP};
use gpui_kit::*;

/// Width the top bar's right-hand buttons take, measured in the rig: about 15px
/// padding, Quick capture and Assistant (40px each, 10px apart) and a 15px gap
/// before the bubble. Client-drawn window decorations add a divider and three
/// window buttons.
/// The bar and its buttons follow the interface zoom (the rem size; 20px at 100 %).
fn bar_right(window: &Window) -> f32 {
    const BUTTONS: f32 = 15. + 40. + 10. + 40. + 15.;
    const WINDOW_BUTTONS: f32 = 1. + 10. + 3. * (40. + 10.);
    zoom(window) * match window.window_decorations() {
        Decorations::Client { .. } => BUTTONS + WINDOW_BUTTONS,
        Decorations::Server => BUTTONS,
    }
}
fn zoom(window: &Window) -> f32 { f32::from(window.rem_size()) / 20. }
/// The bubble's top: centred in the top bar, which is 48px at 100 % and follows the zoom.
fn bubble_top(window: &Window) -> f32 { (BUBBLE_TOP + Saved::BUBBLE / 2.) * zoom(window) - Saved::BUBBLE / 2. }

impl Assistant {
    /// The bubble's displayed top-left corner, in the top bar.
    pub(super) fn bubble_position(&self, window: &mut Window, _cx: &mut App) -> (f32, f32) {
        let size = Self::available_size(window);
        (self.saved.bubble_x(size, bar_right(window)), bubble_top(window))
    }

    /// Move the bubble along the top bar to `x` (its left edge) and keep it there.
    pub(super) fn set_bubble_x(&mut self, x: f32, window: &Window) {
        let size = Self::available_size(window);
        self.saved.set_bubble_x(x, size, bar_right(window));
    }

    pub(super) fn ensure_anchored(&mut self, window: &Window) {
        let size = Self::available_size(window);
        self.saved.ensure_anchored(size, bar_right(window));
    }
}

/// Where the card goes: next to the bubble (above it when the card fits there, else
/// below it, else on the side with more room), always inside the allowed area: below
/// the top bar, right of the rail, 8px from the other edges. A card taller than its
/// side is shortened to fit; its reading area scrolls. Returns `(x, y, height)`.
pub(super) fn card_rect(bubble: (f32, f32), width: f32, wanted: f32, content: Size<Pixels>) -> (f32, f32, f32) {
    const GAP: f32 = 8.;
    let (bubble_x, bubble_y) = bubble;
    let (area_w, area_h) = (f32::from(content.width), f32::from(content.height));
    let (top, bottom) = (SAFE_TOP.min(area_h), (area_h - GAP).max(0.));
    let (left, right) = (SAFE_LEFT.min(area_w), (area_w - GAP).max(0.));
    // Flip the anchor edge rather than pinning the card to the window edge: the header
    // grip must keep moving even when the bubble is on the right.
    let x = if bubble_x + width + GAP <= right { bubble_x } else { bubble_x + Saved::BUBBLE - width };
    let x = x.min(right - width).max(left);
    let above = (bubble_y - GAP - top).max(0.);
    let below_top = (bubble_y + Saved::BUBBLE + GAP).max(top);
    let below = (bottom - below_top).max(0.);
    if wanted <= above {
        (x, bubble_y - GAP - wanted, wanted)
    } else if wanted <= below {
        (x, below_top, wanted)
    } else if above >= below && above >= MIN_CARD {
        (x, top, above)
    } else if below >= MIN_CARD {
        (x, below_top, below)
    } else {
        // Neither side has room (a tiny window): use the whole allowed height.
        let height = wanted.min(bottom - top).max(0.);
        (x, (bottom - height).max(top), height)
    }
}

/// The shortest card worth placing beside the bubble (header, one row, composer).
const MIN_CARD: f32 = 200.;

#[cfg(test)]
mod tests {
    // Not `use super::*`: that glob re-exports `gpui_kit::*`, whose `test` attribute macro
    // shadows the built-in `#[test]` and expands into itself forever.
    use super::{card_rect, SAFE_LEFT, SAFE_TOP};
    use gpui_kit::{px, size, Pixels, Size};

    fn content(width: f32, height: f32) -> Size<Pixels> {
        size(px(width), px(height))
    }

    #[test]
    fn card_hangs_below_the_docked_bubble() {
        // Bubble in the top bar: the card's top edge is fixed just under the bar, so
        // it only grows or shrinks downward as the conversation changes.
        for wanted in [200., 400., 700.] {
            let (_, y, _) = card_rect((1000., 8.), 440., wanted, content(1280., 800.));
            assert_eq!(y, SAFE_TOP);
        }
    }

    #[test]
    fn card_never_covers_the_top_bar() {
        let area = content(1280., 760.);
        // Fits above the bubble.
        assert_eq!(card_rect((400., 700.), 440., 300., area), (400., 392., 300.));
        // The case from pi2.png: too tall for above, no room below either; above is the
        // larger side, so the card is shortened to start at the top bar's edge.
        let (_, y, height) = card_rect((400., 706.), 440., 660., area);
        assert_eq!((y, height), (SAFE_TOP, 706. - 8. - SAFE_TOP));
        // Bubble high up: the card goes below it.
        assert_eq!(card_rect((400., 100.), 440., 300., area), (400., 140., 300.));
        // Bubble in the middle, card taller than either side: the bigger side, shortened.
        let (_, y, height) = card_rect((400., 300.), 440., 700., area);
        assert_eq!((y, height), (340., 760. - 8. - 340.));
        for bubble_y in [56., 200., 380., 500., 640., 720.] {
            for wanted in [180., 400., 700., 900.] {
                let (x, y, h) = card_rect((90., bubble_y), 440., wanted, area);
                assert!(y >= SAFE_TOP && y + h <= 752. && x >= SAFE_LEFT, "{bubble_y} {wanted}");
            }
        }
    }

    #[test]
    fn card_flips_left_near_the_right_edge() {
        let (x, _, _) = card_rect((1200., 700.), 440., 300., content(1280., 760.));
        assert_eq!(x, 1200. + 32. - 440.);
    }
}
