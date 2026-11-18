//! The page side of hint mode: Esc inside page hints, and the calls aDOs's window-wide
//! hint mode makes (collect targets, show labels, filter, pick, hide).
use super::*;
use std::future::Future;

const CANCEL: &str = "!!(window.__adosHints && window.__adosHints.cancel())";

impl BrowserPanel {
    pub(super) fn page_exit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.commenting() {
            if let Some(runtime) = &self.runtime { runtime.shortcut_done(); }
            return if self.comment.card.is_some() { self.cancel_card(window, cx) } else { self.leave_comment(window, cx) };
        }
        let Some(runtime) = self.runtime.clone() else { return self.focus_address(window, cx) };
        let (tx, rx) = async_channel::bounded(1);
        runtime.command(CommandMsg::Eval(CANCEL.into(), tx));
        cx.spawn_in(window, async move |this, cx| {
            let cancelled = matches!(rx.recv().await, Ok(Ok(text)) if text == "true");
            let _ = this.update_in(cx, |this, window, cx| {
                if !cancelled { this.focus_address(window, cx); }
                // The grabbed Esc moved X focus to the proxy. The page keeps GPUI focus,
                // so it must get the X keyboard back too, or `f` and Tab land in GPUI
                // (Tab then walks into another module's editor).
                else if this.page_focus.is_focused(window) && !this.hint_hold.get() {
                    if let Some(runtime) = &this.runtime { runtime.focus_page(); }
                }
                if let Some(runtime) = &this.runtime { runtime.shortcut_done(); }
            });
        })
        .detach();
    }

    fn page_shown(&self, cx: &App) -> bool {
        self.active && self.ready && self.painted_frame && self.error.is_none() && !OverlayGuard::any(cx)
    }

    /// Visible clickables of the page in window coordinates, in the page's order, or
    /// `None` when no page is on screen. The future yields an empty list on any failure.
    pub fn hint_targets(&self, window: &Window, cx: &App) -> Option<impl Future<Output = Vec<Bounds<Pixels>>> + 'static> {
        if !self.page_shown(cx) { return None; }
        let page = self.page_bounds?;
        let runtime = self.runtime.clone()?;
        let scale = window.scale_factor();
        let (tx, rx) = async_channel::bounded(1);
        runtime.command(CommandMsg::Eval("window.__adosHints ? window.__adosHints.collect() : ''".into(), tx));
        Some(async move {
            match rx.recv().await {
                Ok(Ok(text)) => page_targets(&text, page, scale),
                _ => vec![],
            }
        })
    }
    /// Draw `labels[i]` over target `i` inside the page (the page is a native window above GPUI).
    pub fn hints_show(&mut self, labels: &[Option<String>]) {
        self.hint_hold.set(true);
        self.eval(format!("window.__adosHints && window.__adosHints.show({})", serde_json::json!(labels)));
    }
    pub fn hints_typed(&self, typed: &str) {
        self.eval(format!("window.__adosHints && window.__adosHints.typed({})", serde_json::json!(typed)));
    }
    /// Click target `index` and give the keyboard to the page.
    pub fn hints_pick(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Comment mode: the labelled target becomes the comment's target, nothing is clicked.
        if self.comment.on && self.comment.card.is_none() { return self.comment_hint(index, window, cx); }
        self.hint_hold.set(false);
        self.eval(format!("window.__adosHints && window.__adosHints.pick({index})"));
        self.focus_page(window, cx);
    }
    /// Remove page labels; a page that had the keyboard gets it back.
    pub fn hints_hide(&mut self, window: &Window) {
        self.hint_hold.set(false);
        self.eval("window.__adosHints && window.__adosHints.hide()".into());
        if self.page_focus.is_focused(window) {
            if let Some(runtime) = &self.runtime { runtime.focus_page(); }
        }
    }
    /// Hold the keyboard in aDOs while hint mode runs (the page has no labels yet).
    pub fn hints_hold(&self) {
        self.hint_hold.set(true);
        if let Some(runtime) = &self.runtime { runtime.focus_address(); }
    }
    fn eval(&self, script: String) {
        if let Some(runtime) = &self.runtime {
            let (tx, _) = async_channel::bounded(1);
            runtime.command(CommandMsg::Eval(script, tx));
        }
    }
}

/// `collect()` output (CSS px of the viewport) to window coordinates inside `page`.
fn page_targets(text: &str, page: Bounds<Pixels>, scale: f32) -> Vec<Bounds<Pixels>> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return vec![] };
    let dpr = v["dpr"].as_f64().filter(|d| *d > 0.).unwrap_or(1.) as f32;
    let k = dpr / scale.max(0.1);
    let Some(items) = v["items"].as_array() else { return vec![] };
    items.iter().map(|item| {
        let n = |i: usize| item.get(i).and_then(|x| x.as_f64()).unwrap_or(0.) as f32 * k;
        Bounds::new(point(page.origin.x + px(n(0)), page.origin.y + px(n(1))), size(px(n(2)), px(n(3))))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::page_targets;
    use gpui_kit::{point, px, size, Bounds};
    #[test]
    fn page_targets_map_css_to_window() {
        let page = Bounds::new(point(px(100.), px(50.)), size(px(800.), px(600.)));
        let text = r#"{"dpr":2,"w":800,"h":600,"items":[[10,20,30,40]]}"#;
        let out = page_targets(text, page, 2.);
        assert_eq!(out, vec![Bounds::new(point(px(110.), px(70.)), size(px(30.), px(40.)))]);
        assert_eq!(page_targets("", page, 1.), vec![]);
        assert_eq!(page_targets(r#"{"items":[[1,2,3,4]]}"#, page, 1.).len(), 1);
    }
}
