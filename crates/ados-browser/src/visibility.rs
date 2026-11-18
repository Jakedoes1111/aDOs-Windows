//! Native page eligibility is event driven, never an idle frame loop.
use super::*;

impl BrowserPanel {
    pub(super) fn sync_visibility(&self, cx: &mut Context<Self>) {
        let visible = self.painted_frame && self.active && self.error.is_none() && !OverlayGuard::hides(cx) && !self.dragging;
        if let Some(runtime) = &self.runtime {
            runtime.cutout(self.cutout(cx));
            runtime.visible(visible);
        }
        ados_core::overlay::set_native_rect(if visible && self.ready { self.area } else { None }, cx);
    }
    /// What to cut from the page: the assistant card, plus every open overlay's rectangle.
    fn cutout(&self, cx: &App) -> crate::cutout::Holes {
        let modal = OverlayGuard::any(cx);
        let mut cards: Vec<_> = ados_core::overlay::cutout(cx).into_iter().collect();
        if modal { cards.extend(ados_core::overlay::holes(cx)); }
        let rects = cards.into_iter().filter_map(|card| self.hole(card)).collect();
        crate::cutout::Holes { rects, modal }
    }
    /// One floating rectangle in the aDOs window's physical pixels (each tile cuts its part).
    fn hole(&self, card: Bounds<Pixels>) -> Option<Rect> {
        self.area?;
        Some(physical(card, self.scale))
    }
    // Force one uncached paint between callbacks, reconcile its receipt, then
    // STOP without invalidating the window. Coalesce concurrent change signals.
    pub(super) fn check_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.checking_visibility { return; }
        self.checking_visibility = true;
        let panel = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            if panel.update(cx, |this, _| this.painted_frame = false).is_err() { return; }
            window.refresh();
            window.on_next_frame(move |_, cx| {
                let _ = panel.update(cx, |this, cx| {
                    this.checking_visibility = false;
                    this.sync_visibility(cx);
                });
            });
        });
    }
    pub(super) fn painted(&mut self, bounds: Bounds<Pixels>, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.check_visibility(window, cx);
        self.painted_frame = bounds.size.width > px(0.) && bounds.size.height > px(0.);
        let narrow = bounds.size.width < px(560.);
        if narrow != self.narrow { self.narrow = narrow; cx.notify(); }
        self.area = Some(bounds);
        self.scale = scale;
        // Tiled: each tile's page at its place; the tile with the keyboard is the "page" for
        // hints, comments and the context snapshot.
        let tiles = self.tile_bounds(bounds);
        let current = self.tabs.current().id;
        self.page_bounds = tiles.iter().find(|(id, _)| *id == current || *id == 0).map(|(_, b)| *b).or(Some(bounds));
        let geometry: Vec<(u64, Rect)> = tiles.into_iter().map(|(id, b)| (id, physical(b, scale))).collect();
        if self.geometry != geometry {
            self.geometry = geometry.clone();
            if let Some(runtime) = &self.runtime { runtime.geometry(geometry); }
        }
        self.sync_visibility(cx);
        // A Vulkan parent present can cover a foreign drawable on Xvfb.
        if let Some(runtime) = &self.runtime { runtime.repaint(); }
    }
}

/// Logical bounds to physical pixels; edges are rounded, so clipping and viewport cannot
/// disagree by a pixel.
fn physical(bounds: Bounds<Pixels>, scale: f32) -> Rect {
    let x = (f32::from(bounds.left()) * scale).round();
    let y = (f32::from(bounds.top()) * scale).round();
    Rect {
        x: x as i32,
        y: y as i32,
        w: ((f32::from(bounds.right()) * scale).round() - x).max(0.) as u32,
        h: ((f32::from(bounds.bottom()) * scale).round() - y).max(0.) as u32,
    }
}
