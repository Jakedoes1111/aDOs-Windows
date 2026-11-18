//! Web addresses in the terminal open in the aDOs Browser: Ctrl+click on one, or
//! `terminal.open_link` for the nearest one above the cursor (`ados_core::links`).
use super::*;
use gpui_kit::component::WindowExt;

impl TerminalPanel {
    /// The URL at `point` on its line (wrapped URLs are not joined).
    fn url_at_point(&self, point: TermPoint) -> Option<String> {
        let backend = self.backend.as_ref()?;
        let term = backend.term.lock();
        let line = term.bounds_to_string(TermPoint::new(point.line, Column(0)), TermPoint::new(point.line, term.last_column()));
        let at = line.char_indices().nth(point.column.0).map_or(line.len(), |(i, _)| i);
        ados_core::links::find_urls(&line).into_iter().find(|r| r.contains(&at)).map(|r| line[r].to_string())
    }
    /// The last URL on the cursor's line, else on the nearest line above it.
    fn url_near_cursor(&self) -> Option<String> {
        let backend = self.backend.as_ref()?;
        let term = backend.term.lock();
        let cursor = term.grid().cursor.point.line.0;
        let top = -(term.history_size() as i32);
        (top..=cursor).rev().take(200).find_map(|n| {
            let line = term.bounds_to_string(TermPoint::new(Line(n), Column(0)), TermPoint::new(Line(n), term.last_column()));
            ados_core::links::find_urls(&line).last().map(|r| line[r.clone()].to_string())
        })
    }
    /// Ctrl+click: true when it hit a URL (the click is then not a selection).
    pub(crate) fn ctrl_click(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !event.modifiers.control {
            return false;
        }
        let (point, _) = self.position(event.position);
        match self.url_at_point(point) {
            Some(url) => {
                ados_core::links::open_url(&url, window, cx);
                true
            }
            None => false,
        }
    }
    pub(crate) fn open_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.url_near_cursor() {
            Some(url) => ados_core::links::open_url(&url, window, cx),
            None => window.push_notification("No link found above the cursor", cx),
        }
    }
}
