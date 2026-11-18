//! URLs in an event's title, location or details open in the aDOs Browser (`ados_core::links`).
use crate::{model::Occurrence, panel::CalendarPanel};
use gpui_kit::component::WindowExt;
use gpui_kit::*;

pub(crate) fn event_url(o: &Occurrence) -> Option<String> {
    [Some(o.title.as_str()), o.location.as_deref(), o.notes.as_deref()].into_iter().flatten()
        .find_map(|t| ados_core::links::find_urls(t).first().map(|r| t[r.clone()].to_string()))
}

impl CalendarPanel {
    pub(crate) fn open_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_occurrence().as_ref().and_then(event_url) {
            Some(url) => ados_core::links::open_url(&url, window, cx),
            None => window.push_notification("This event has no link to open", cx),
        }
    }
}
