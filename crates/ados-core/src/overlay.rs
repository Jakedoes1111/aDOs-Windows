//! The browser module is a real child X11 window, which always draws above GPUI content.
//! Anything that floats over the workspace (palette, menus, assistant card) calls `open`
//! while visible and `close` when it goes, and the browser hides its window meanwhile.
//! The assistant card publishes a `Cutout` instead, so the page stays visible around it.
use gpui_kit::{App, Global, Bounds, Pixels};

/// A dock paint-eligibility change, including zoom (which is not a saved-layout edit).
#[derive(Default)]
pub struct LayoutEpoch;
impl Global for LayoutEpoch {}

pub fn bump_layout_epoch(cx: &mut App) {
    cx.set_global(LayoutEpoch);
}

/// A panel opened a choice the user must see whole (Sessions' new-session picker). An open
/// assistant card that does not have the keyboard folds back into its bubble so it never
/// hides that choice.
#[derive(Default)]
pub struct MakeRoom(pub u64);
impl Global for MakeRoom {}

pub fn make_room(cx: &mut App) {
    let next = cx.try_global::<MakeRoom>().map_or(1, |m| m.0 + 1);
    cx.set_global(MakeRoom(next));
}

#[derive(Default)]
pub struct OverlayGuard { count: usize, cuttable: usize, hooks: Vec<std::rc::Rc<dyn Fn(bool)>> }
impl Global for OverlayGuard {}

/// The browser's native page rectangle, in logical window coordinates. Native
/// surfaces paint above GPUI, so overlays must avoid this area. Publish None
/// when the browser page is hidden. Repeated equal rectangles are no-ops.
#[derive(Default)]
pub struct NativeRect(pub Option<Bounds<Pixels>>);
impl Global for NativeRect {}

pub fn set_native_rect(rect: Option<Bounds<Pixels>>, cx: &mut App) {
    if !cx.has_global::<NativeRect>() || cx.global::<NativeRect>().0 != rect {
        cx.set_global(NativeRect(rect));
    }
}

pub fn native_rect(cx: &App) -> Option<Bounds<Pixels>> {
    cx.try_global::<NativeRect>().and_then(|state| state.0)
}

/// A floating card that sits over the workspace without hiding the browser page
/// (the assistant card), in logical window coordinates. The browser cuts this
/// rectangle out of its native window instead of hiding the whole page, so the
/// card does not use `OverlayGuard`. Publish None when the card goes.
#[derive(Default)]
pub struct Cutout(pub Option<Bounds<Pixels>>);
impl Global for Cutout {}

pub fn set_cutout(rect: Option<Bounds<Pixels>>, cx: &mut App) {
    if cx.try_global::<Cutout>().map_or(rect.is_some(), |state| state.0 != rect) {
        cx.set_global(Cutout(rect));
    }
}

pub fn cutout(cx: &App) -> Option<Bounds<Pixels>> {
    cx.try_global::<Cutout>().and_then(|state| state.0)
}

/// Reset GPUI's hover/tooltip listeners when an X child remaps under the pointer.
/// Native windows stop delivering GPUI motion/leave events at that boundary.
/// This is a lifecycle event, not a timeout or pointer warp.
pub fn clear_pointer(window: &mut gpui_kit::Window, cx: &mut App) {
    window.dispatch_event(gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
        position: gpui_kit::point(gpui_kit::px(-100.), gpui_kit::px(-100.)),
        ..Default::default()
    }), cx);
    window.refresh();
}

/// A module's live tab title (the Browser shows its page title). Absent: the module's own name.
#[derive(Default)]
pub struct TabTitles(pub std::collections::HashMap<String, String>);
impl Global for TabTitles {}

/// Returns true when the title changed (the caller then refreshes its window).
pub fn set_tab_title(module: &str, title: Option<String>, cx: &mut App) -> bool {
    let mut titles = cx.try_global::<TabTitles>().map(|t| t.0.clone()).unwrap_or_default();
    if titles.get(module) == title.as_ref() { return false; }
    match title { Some(t) => { titles.insert(module.into(), t); } None => { titles.remove(module); } }
    cx.set_global(TabTitles(titles));
    true
}

pub fn tab_title(module: &str, cx: &App) -> Option<String> {
    cx.try_global::<TabTitles>().and_then(|t| t.0.get(module).cloned())
}

#[derive(Default)]
pub struct BottomNotice(pub bool);
impl Global for BottomNotice {}

/// Rectangles of open overlays that float over the page, in logical window coordinates,
/// keyed by overlay. The browser cuts them out of its page window (modal: clicks on the
/// rest of the page fall through to aDOs). Only effective while an overlay is open.
#[derive(Default)]
pub struct Holes(pub Vec<(&'static str, Bounds<Pixels>)>);
impl Global for Holes {}

pub fn set_hole(key: &'static str, rect: Option<Bounds<Pixels>>, cx: &mut App) {
    let current = cx.try_global::<Holes>().and_then(|h| h.0.iter().find(|(k, _)| *k == key).map(|(_, r)| *r));
    if current == rect { return; }
    let mut list = cx.try_global::<Holes>().map(|h| h.0.clone()).unwrap_or_default();
    list.retain(|(k, _)| *k != key);
    if let Some(rect) = rect { list.push((key, rect)); }
    cx.set_global(Holes(list));
}

pub fn holes(cx: &App) -> Vec<Bounds<Pixels>> {
    cx.try_global::<Holes>().map(|h| h.0.iter().map(|(_, r)| *r).collect()).unwrap_or_default()
}

/// An invisible child that fills its parent and publishes the parent's bounds as the hole
/// `key` on every paint. The owner clears it with `set_hole(key, None)` when it closes.
pub fn hole_tracker(key: &'static str) -> impl gpui_kit::IntoElement {
    use gpui_kit::*;
    canvas(|_, _, _| (), move |bounds, _, _, cx| set_hole(key, Some(bounds), cx))
        .absolute().size_full()
}

impl OverlayGuard {
    /// An overlay that publishes a hole (`hole_tracker`) instead of hiding the page.
    pub fn open_cut(cx: &mut App) {
        cx.global_mut::<Self>().cuttable += 1;
        Self::open(cx);
    }
    pub fn close_cut(cx: &mut App) {
        let g = cx.global_mut::<Self>();
        g.cuttable = g.cuttable.saturating_sub(1);
        Self::close(cx);
    }
    /// True while an overlay that cannot be cut around is open: the page must hide.
    pub fn hides(cx: &App) -> bool { let g = cx.global::<Self>(); g.count > g.cuttable }
    /// Boundary hooks run synchronously, before the caller focuses an overlay input.
    pub fn on_boundary(cx: &mut App, hook: std::rc::Rc<dyn Fn(bool)>) {
        cx.global_mut::<Self>().hooks.push(hook);
    }
    pub fn open(cx: &mut App) {
        let g = cx.global_mut::<Self>();
        g.count += 1;
        let hooks = if g.count == 1 { g.hooks.clone() } else { vec![] };
        for hook in hooks { hook(true); }
    }
    pub fn close(cx: &mut App) {
        let g = cx.global_mut::<Self>();
        let was_open = g.count > 0;
        g.count = g.count.saturating_sub(1);
        if g.count == 0 { cx.set_global(Holes::default()); }
        let g = cx.global_mut::<Self>();
        let hooks = if was_open && g.count == 0 { g.hooks.clone() } else { vec![] };
        for hook in hooks { hook(false); }
    }
    /// True while any overlay is showing. Observe with `cx.observe_global::<OverlayGuard>`.
    pub fn any(cx: &App) -> bool { cx.global::<Self>().count > 0 }
    pub fn count(cx: &App) -> usize { cx.global::<Self>().count }
}
