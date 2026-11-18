use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

pub fn install(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    // App toasts must not cover the terminal's bottom prompt.
    use gpui_kit::component::notification::NotificationSettings;
    let mut notifications = NotificationSettings::default();
    notifications.placement = gpui_kit::Anchor::BottomRight;
    notifications.margins.bottom = px(16.);
    notifications.margins.right = px(16.);
    Theme::global_mut(cx).notification = notifications;
    // Toasts never cover the open assistant card (its composer and Send) nor the browser's
    // native page, which paints above GPUI and would hide them: pick the first free spot.
    cx.observe_global::<ados_core::overlay::Cutout>(place_toasts).detach();
    cx.observe_global::<ados_core::overlay::NativeRect>(place_toasts).detach();
    Theme::update(cx, |t| {
        // The default caret blinks every 500 ms forever: a timer repainting an idle window
        // (about 45% CPU idle under software GL with the address bar focused). Blink while
        // the user works, then hold the caret steady after 10 quiet seconds.
        t.cursor_blink = gpui_kit::base::input::CursorBlink::blinking(std::time::Duration::from_millis(500))
            .with_idle_timeout(std::time::Duration::from_secs(10));
        let canvas = rgb(0x030405).into();
        let panel = rgb(0x08090C).into();
        let border = rgb(0x25282D).into();
        let text = rgb(0xD4D7DF).into();
        let muted = rgb(0x939BA8).into();
        let blue = rgb(ados_core::brand::BLUE).into();
        let red = rgb(ados_core::brand::RED).into();
        let c = &mut t.colors;
        c.background = canvas;
        c.foreground = text;
        c.border = border;
        c.window_border = border;
        c.sidebar = panel;
        c.sidebar_border = border;
        c.sidebar_foreground = muted;
        c.sidebar_accent = border;
        c.sidebar_accent_foreground = text;
        c.sidebar_primary = blue;
        c.sidebar_primary_foreground = text;
        c.tab = panel;
        c.tab_bar = panel;
        c.tab_active = canvas;
        c.tab_active_foreground = text;
        c.tab_foreground = muted;
        c.title_bar = panel;
        c.title_bar_border = border;
        c.status_bar = panel;
        c.status_bar_border = border;
        c.muted = panel;
        c.muted_foreground = muted;
        c.popover = panel;
        c.popover_foreground = text;
        c.input = canvas;
        c.list = panel;
        c.list_hover = border;
        c.list_active = blue;
        c.list_active_border = blue;
        c.list_head = panel;
        c.button = panel;
        c.button_foreground = text;
        c.button_hover = border;
        c.button_active = border;
        c.primary = blue;
        c.primary_hover = blue;
        c.primary_active = blue;
        c.primary_foreground = text;
        c.button_primary = blue;
        c.button_primary_hover = blue;
        c.button_primary_active = blue;
        c.button_primary_foreground = text;
        c.accent = border;
        c.accent_foreground = text;
        c.secondary = panel;
        c.secondary_foreground = text;
        c.ring = blue;
        c.selection = blue.opacity(0.3);
        c.drag_border = blue;
        c.drop_target = blue.opacity(0.15);
        c.caret = blue;
        c.link = blue;
        c.danger = red;
        c.button_danger = red;
        c.scrollbar = panel;
        c.scrollbar_thumb = border;
        c.scrollbar_thumb_hover = muted;
        t.radius = px(8.);
        t.radius_lg = px(10.);
        t.shadow = false;
        // PopupMenu rows are 1.625rem in gpui-kit 0.9.1; 20px keeps them above 32px.
        t.font_size = px(20.);
    });
}

fn place_toasts(cx: &mut App) {
    use gpui_kit::component::notification::NotificationSettings;
    let default_width = f32::from(NotificationSettings::default().width);
    let viewport = cx.windows().first().and_then(|w| w.update(cx, |_, window, _| window.viewport_size()).ok());
    let rect = |b: Bounds<Pixels>| Area { x: f32::from(b.origin.x), y: f32::from(b.origin.y), w: f32::from(b.size.width), h: f32::from(b.size.height) };
    let avoid: Vec<Area> = [ados_core::overlay::cutout(cx), ados_core::overlay::native_rect(cx)].into_iter().flatten().map(rect).collect();
    let spot = viewport.map_or(Spot::default_for(default_width), |v| {
        toast_spot(f32::from(v.width), f32::from(v.height), &avoid, default_width)
    });
    let current = &Theme::global(cx).notification;
    let m = &current.margins;
    let same_margins = [m.top, m.right, m.bottom, m.left] == [px(spot.top), px(spot.right), px(spot.bottom), px(spot.left)];
    if current.placement != spot.anchor || f32::from(current.width) != spot.width || !same_margins {
        let settings = &mut Theme::global_mut(cx).notification;
        settings.placement = spot.anchor;
        settings.width = px(spot.width);
        settings.margins.top = px(spot.top);
        settings.margins.right = px(spot.right);
        settings.margins.bottom = px(spot.bottom);
        settings.margins.left = px(spot.left);
        cx.refresh_windows();
    }
}

/// A rectangle in logical window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Area { x: f32, y: f32, w: f32, h: f32 }

impl Area {
    fn meets(&self, other: &Area) -> bool {
        self.x < other.x + other.w && other.x < self.x + self.w && self.y < other.y + other.h && other.y < self.y + self.h
    }
}

/// Where the toast stack goes: an anchor, its margins and its width.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot { anchor: gpui_kit::Anchor, top: f32, right: f32, bottom: f32, left: f32, width: f32 }

impl Spot {
    fn default_for(width: f32) -> Self {
        Spot { anchor: gpui_kit::Anchor::BottomRight, top: 16., right: 16., bottom: 16., left: 16., width }
    }
}

/// Room kept for the toast stack (two toasts with an Undo button), or for one.
const STACK_HEIGHT: f32 = 180.;
const ONE_TOAST: f32 = 90.;
/// Below the top bar.
const TOP: f32 = 64.;

/// The first spot, bottom-right first, where a `width`-wide toast stack meets none of
/// `avoid` (the assistant card and the browser's native page). Toasts may sit beside or
/// below them. If no spot is free, the page is still avoided before the card, since a
/// toast under the page cannot be seen at all.
fn toast_spot(vw: f32, vh: f32, avoid: &[Area], width: f32) -> Spot {
    let fallback = Spot::default_for(width);
    if avoid.is_empty() {
        return fallback;
    }
    // Room for two toasts first, then for one, then narrower.
    for (w, h) in [(width, STACK_HEIGHT), (width, ONE_TOAST), (260., STACK_HEIGHT), (260., ONE_TOAST)] {
        let mut xs = vec![vw - 16. - w, 16.];
        let mut ys = vec![vh - 16. - h, TOP];
        for a in avoid {
            xs.extend([a.x - 16. - w, a.x + a.w + 16.]);
            ys.extend([a.y + a.h + 16., a.y - 16. - h]);
        }
        if let Some(spot) = first_free(&xs, &ys, w, h, vw, vh, avoid) {
            return spot;
        }
    }
    // Nothing clears both: a toast over the card is still seen, one under the page is not.
    if avoid.len() > 1 {
        let page = &avoid[avoid.len() - 1..];
        for (w, h) in [(width, ONE_TOAST), (260., ONE_TOAST)] {
            let xs = [vw - 16. - w, 16., page[0].x - 16. - w, page[0].x + page[0].w + 16.];
            let ys = [vh - 16. - h, TOP, page[0].y + page[0].h + 16.];
            if let Some(spot) = first_free(&xs, &ys, w, h, vw, vh, page) {
                return spot;
            }
        }
    }
    fallback
}

/// The first (x, y) whose `w`×`h` stack lies in the window below the top bar and meets
/// none of `avoid`, anchored by its top-left corner (it grows down into the room checked).
fn first_free(xs: &[f32], ys: &[f32], w: f32, h: f32, vw: f32, vh: f32, avoid: &[Area]) -> Option<Spot> {
    ys.iter().flat_map(|&y| xs.iter().map(move |&x| (x, y))).find_map(|(x, y)| {
        let stack = Area { x, y, w, h };
        let inside = x >= 0. && y >= TOP - 16. && x + w <= vw && y + h <= vh;
        (inside && !avoid.iter().any(|b| b.meets(&stack)))
            .then_some(Spot { anchor: gpui_kit::Anchor::TopLeft, top: y, left: x, right: 16., bottom: 16., width: w })
    })
}

#[cfg(test)]
mod tests {
    use super::{toast_spot, Area, Spot, TOP};
    use gpui_kit::Anchor;

    fn place(avoid: &[Area]) -> Spot {
        toast_spot(1280., 800., avoid, 382.)
    }
    fn stack(s: &Spot) -> Area {
        Area { x: s.left, y: s.top, w: s.width, h: super::ONE_TOAST }
    }

    #[test]
    fn nothing_to_avoid_is_bottom_right() {
        assert_eq!(place(&[]).anchor, Anchor::BottomRight);
    }

    #[test]
    fn toasts_avoid_card_and_page() {
        // Research: page on the left half, card on the right (round 8 bug 2).
        let page = Area { x: 72., y: 100., w: 560., h: 680. };
        let card = Area { x: 720., y: 56., w: 440., h: 600. };
        let s = place(&[card, page]);
        assert!(!stack(&s).meets(&card) && !stack(&s).meets(&page), "{s:?}");
        assert!(stack(&s).x + s.width <= 1280. && stack(&s).y + super::ONE_TOAST <= 800.);
        // Card only, page elsewhere: still clear of both.
        let s = place(&[Area { x: 824., y: 56., w: 440., h: 700. }]);
        assert!(!stack(&s).meets(&Area { x: 824., y: 56., w: 440., h: 700. }));
    }

    #[test]
    fn page_beats_card_when_nothing_is_free() {
        // Page fills the workspace under the top bar; the card covers the right edge.
        let page = Area { x: 72., y: TOP, w: 1208., h: 736. };
        let card = Area { x: 820., y: 56., w: 440., h: 720. };
        let s = place(&[card, page]);
        // No spot avoids the page at all here, so this is the documented fallback.
        assert_eq!(s.anchor, Anchor::BottomRight);
        // A page on the left with a short card: below the card is free of both.
        let page = Area { x: 72., y: 100., w: 600., h: 700. };
        let card = Area { x: 720., y: 56., w: 440., h: 300. };
        let s = place(&[card, page]);
        assert!(!stack(&s).meets(&page) && !stack(&s).meets(&card), "{s:?}");
        // Page on the left, a full-height card on the right: over the card, never the page.
        let page = Area { x: 72., y: 100., w: 560., h: 680. };
        let card = Area { x: 700., y: 56., w: 564., h: 730. };
        let s = place(&[card, page]);
        assert!(!stack(&s).meets(&page), "{s:?}");
    }
}
