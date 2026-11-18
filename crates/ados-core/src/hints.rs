//! Hint mode contract: label generation, the look, and `mark()`, the one-line way a
//! control says "I can be clicked from hint mode". The shell owns the mode itself.
use gpui_kit::*;
use serde::{Deserialize, Serialize};

/// Home-row letters, Vimium order.
pub const ALPHABET: &str = "sadjklewcmpgh";
/// Labels nearer than this (both axes, logical px) to a kept one are dropped, like the owner's tool.
pub const DEDUP_PX: f32 = 14.;
/// Hint mode ends on its own after this long without a key, like the owner's tool.
pub const TIMEOUT_MS: u64 = 5500;

/// Shortest unique labels (Vimium's algorithm): no label is a prefix of another.
pub fn labels(count: usize) -> Vec<String> {
    if count == 0 { return vec![]; }
    let letters: Vec<char> = ALPHABET.chars().collect();
    let mut hints = vec![String::new()];
    let mut offset = 0;
    while hints.len() - offset < count || hints.len() == 1 {
        let hint = hints[offset].clone();
        offset += 1;
        for ch in &letters { hints.push(format!("{ch}{hint}")); }
    }
    let mut out: Vec<String> = hints[offset..offset + count].to_vec();
    out.sort();
    out.into_iter().map(|h| h.chars().rev().collect()).collect()
}

/// Indices of points to keep, in order: a point within `min` of an earlier kept one is dropped.
pub fn dedup(points: &[(f32, f32)], min: f32) -> Vec<usize> {
    let mut kept: Vec<usize> = vec![];
    for (i, (x, y)) in points.iter().enumerate() {
        if kept.iter().all(|&k| (points[k].0 - x).abs() >= min || (points[k].1 - y).abs() >= min) {
            kept.push(i);
        }
    }
    kept
}

/// RGBA, 0–1 per channel, like the owner's `~/.config/hints/config.json`.
pub type Color4 = [f32; 4];

/// The hint look. Saved in `ADOS_HOME/hints.json`; edit it to change colours.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub background: Color4,
    pub text: Color4,
    pub typed: Color4,
}
impl Default for Style {
    fn default() -> Self {
        // Same as the owner's desktop hints.
        Self {
            background: [0.769, 0.71, 0.612, 0.95],
            text: [0.043, 0.039, 0.027, 1.],
            typed: [0.561, 0.514, 0.443, 1.],
        }
    }
}
impl Style {
    pub fn path() -> std::path::PathBuf { crate::paths::home().join("hints.json") }
    /// The saved look, or the default (written out so it can be edited). A broken file says why.
    pub fn load() -> (Self, Option<String>) {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(style) => (style, None),
                Err(e) => (Self::default(), Some(format!("hints.json could not be read ({e}); using the default colours."))),
            },
            Err(_) => {
                let style = Self::default();
                if let Ok(text) = serde_json::to_string_pretty(&style) { let _ = std::fs::write(&path, text); }
                (style, None)
            }
        }
    }
    pub fn hsla(c: Color4) -> Hsla { gpui_kit::Rgba { r: c[0], g: c[1], b: c[2], a: c[3] }.into() }
    pub fn css(c: Color4) -> String {
        let ch = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        format!("rgba({},{},{},{})", ch(c[0]), ch(c[1]), ch(c[2]), c[3].clamp(0., 1.))
    }
}

/// How high a control sits. A mark is hidden when a higher layer covers its centre, and
/// while a modal overlay (palette, dialog: `OverlayGuard::any`) is open only `Overlay`
/// marks count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// Panels, rail, top bar.
    Base,
    /// Floating but not modal: the assistant card, toasts.
    Float,
    /// Menus opened from a floating card.
    Menu,
    /// Modal overlays: palette, shortcut editor, dialogs.
    Overlay,
}

#[derive(Clone, Copy, Debug)]
pub struct Target { pub bounds: Bounds<Pixels>, pub layer: Layer }

/// Marks and covering surfaces reported during one collecting frame.
#[derive(Default)]
pub struct Targets { pub collecting: bool, pub found: Vec<Target>, pub covers: Vec<Target> }
impl Global for Targets {}

/// Add as a child of a clickable control (`.child(ados_core::hints::mark())`). It takes the
/// parent's box, draws nothing, takes no mouse events, and reports itself only while hint
/// mode is collecting. Hint mode clicks the centre of that box.
pub fn mark() -> impl IntoElement { mark_in(Layer::Base) }
/// Wrap a control that must keep its own children (icon-only buttons) with a mark.
pub fn marked(control: impl IntoElement) -> Div { marked_in(Layer::Base, control) }
/// `marked` for a control on another layer.
pub fn marked_in(layer: Layer, control: impl IntoElement) -> Div { div().flex().relative().child(control).child(mark_in(layer)) }
/// For controls inside modal overlays such as the palette.
pub fn mark_overlay() -> impl IntoElement { mark_in(Layer::Overlay) }

/// A mark for a control on `layer`.
pub fn mark_in(layer: Layer) -> impl IntoElement { probe(layer, None) }

/// Add as a child of a floating surface (card, menu, toast): marks of lower layers under it
/// get no label, since a click there would land on the surface.
pub fn cover(layer: Layer) -> impl IntoElement { probe(layer, Some(px(0.))) }
/// `cover` for a surface whose padding lies outside the element it can reach (a toast's
/// message column): the covered box grows by `pad` on every side.
pub fn cover_padded(layer: Layer, pad: Pixels) -> impl IntoElement { probe(layer, Some(pad)) }

/// A mark (`cover` None) or a cover grown by `cover` on each side.
fn probe(layer: Layer, cover: Option<Pixels>) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            if !cx.try_global::<Targets>().is_some_and(|t| t.collecting) { return; }
            let visible = bounds.intersect(&window.content_mask().bounds);
            if visible.size.width < px(2.) || visible.size.height < px(2.) { return; }
            let targets = cx.global_mut::<Targets>();
            match cover {
                Some(pad) => targets.covers.push(Target { bounds: visible.dilate(pad), layer }),
                None => targets.found.push(Target { bounds: visible, layer }),
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Two marks label the same spot: their corners are closer than `DEDUP_PX` on both axes.
pub fn near(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    (f32::from(a.origin.x) - f32::from(b.origin.x)).abs() < DEDUP_PX
        && (f32::from(a.origin.y) - f32::from(b.origin.y)).abs() < DEDUP_PX
}

/// Indexes of the targets a user can actually click: under a modal overlay only overlay
/// marks; otherwise every mark whose centre no higher-layer cover hides.
pub fn reachable(found: &[Target], covers: &[Target], modal: bool) -> Vec<usize> {
    (0..found.len())
        .filter(|&i| {
            let t = found[i];
            if modal { return t.layer == Layer::Overlay; }
            let centre = t.bounds.center();
            !covers.iter().any(|c| c.layer > t.layer && c.bounds.contains(&centre))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{dedup, labels, near, reachable, Layer, Style, Target, ALPHABET, DEDUP_PX};
    use gpui_kit::{point, px, size, Bounds};

    fn at(x: f32, y: f32, w: f32, h: f32, layer: Layer) -> Target {
        Target { bounds: Bounds::new(point(px(x), px(y)), size(px(w), px(h))), layer }
    }

    #[test]
    fn near_is_within_dedup_on_both_axes() {
        let a = at(10., 10., 5., 5., Layer::Base).bounds;
        assert!(near(a, at(20., 23., 50., 50., Layer::Base).bounds));
        assert!(!near(a, at(10., 30., 5., 5., Layer::Base).bounds));
    }

    #[test]
    fn covered_marks_are_unreachable() {
        let card = at(100., 0., 200., 300., Layer::Float);
        let menu = at(120., 40., 100., 60., Layer::Menu);
        let found = [
            at(10., 10., 20., 20., Layer::Base),   // beside the card
            at(150., 150., 20., 20., Layer::Base), // under the card
            at(150., 150., 20., 20., Layer::Float), // card button
            at(130., 50., 20., 20., Layer::Float), // card control under its menu
            at(130., 50., 20., 20., Layer::Menu),  // menu item
            at(10., 10., 20., 20., Layer::Overlay),
        ];
        assert_eq!(reachable(&found, &[card, menu], false), vec![0, 2, 4, 5]);
        assert_eq!(reachable(&found, &[card, menu], true), vec![5]);
        assert_eq!(reachable(&found, &[], false), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn labels_are_shortest_and_prefix_free() {
        assert!(labels(0).is_empty());
        assert_eq!(labels(1).len(), 1);
        let few = labels(5);
        assert!(few.iter().all(|l| l.len() == 1), "{few:?}");
        let many = labels(40);
        assert_eq!(many.len(), 40);
        for a in &many {
            assert!(a.chars().all(|c| ALPHABET.contains(c)));
            for b in &many {
                if a != b { assert!(!b.starts_with(a.as_str()), "{a} is a prefix of {b}"); }
            }
        }
        assert!(many.iter().all(|l| l.len() <= 2));
        let mut unique = many.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 40);
    }

    #[test]
    fn labels_match_vimium_for_three() {
        // Vimium with this alphabet gives three links a, d, s.
        assert_eq!(labels(3), vec!["a", "d", "s"]);
    }

    #[test]
    fn dedup_drops_near_points_only() {
        let points = [(10., 10.), (20., 15.), (30., 10.), (10., 40.), (24., 12.)];
        assert_eq!(dedup(&points, DEDUP_PX), vec![0, 2, 3]);
        assert_eq!(dedup(&[], DEDUP_PX), Vec::<usize>::new());
    }

    #[test]
    fn style_round_trips_and_css() {
        let style = Style::default();
        let back: Style = serde_json::from_str(&serde_json::to_string(&style).unwrap()).unwrap();
        assert_eq!(style, back);
        assert_eq!(Style::css([1., 0., 0.5, 0.95]), "rgba(255,0,128,0.95)");
        let partial: Style = serde_json::from_str(r#"{"background":[0,0,0,1]}"#).unwrap();
        assert_eq!(partial.text, Style::default().text);
    }
}
