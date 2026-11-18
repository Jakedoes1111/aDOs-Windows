//! Calendar module for aDOs: native events plus read-only `.ics` subscriptions.
//! See `panel` for the `CalendarPanel` (dock panel name `ados_core::ids::CALENDAR`).
pub mod fields;
pub mod ics;
pub mod model;
mod links;
mod panel;
mod panel_form;
mod panel_grid;
mod panel_view;
pub mod store;
pub mod subs;
#[cfg(test)]
mod tests;

pub use model::{Event, Occurrence};
pub use panel::{init, CalendarPanel};

use jiff::Timestamp;

/// Every occurrence overlapping `[start, end)`, native and subscribed, sorted by start.
/// Reads `events.json` and the cached subscription copies; never touches the network.
/// Files that cannot be read contribute nothing (the panel shows why).
pub fn events_between(start: Timestamp, end: Timestamp) -> Vec<Occurrence> {
    let native = store::load_events().unwrap_or_default();
    let subscribed: Vec<subs::Loaded> = subs::list().unwrap_or_default().iter().map(subs::cached).collect();
    collect(&native, &subscribed, start, end)
}

pub(crate) fn collect(native: &[Event], subscribed: &[subs::Loaded], start: Timestamp, end: Timestamp) -> Vec<Occurrence> {
    let local = model::local_zone();
    let mut out: Vec<Occurrence> = native.iter()
        .flat_map(|ev| model::expand(&model::Series { event: ev.clone(), repeat: None, exdates: vec![] }, start, end, None, &local))
        .collect();
    for cal in subscribed {
        for series in &cal.series {
            out.extend(model::expand(series, start, end, Some(&cal.name), &local));
        }
    }
    out.sort_by(|a, b| (!a.all_day, a.start, &a.title).cmp(&(!b.all_day, b.start, &b.title)));
    out
}
