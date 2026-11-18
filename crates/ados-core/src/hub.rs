//! What each module is showing right now, so the assistant always has context.
//! A module calls `publish` whenever its visible content changes (cheap: just strings).
use gpui_kit::{App, Global, SharedString};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Module id from `crate::ids`.
    pub module: SharedString,
    /// What the user would call it: note title, page title, "Terminal".
    pub title: SharedString,
    /// Page URL or file path, when there is one.
    pub location: Option<SharedString>,
    /// Visible text: note body, page text, terminal tail. Cap at ~60k chars.
    pub text: SharedString,
    /// Currently selected text, if any.
    pub selection: Option<SharedString>,
}

#[derive(Default)]
pub struct ContextHub {
    snapshots: BTreeMap<SharedString, Snapshot>,
    active: Option<SharedString>,
}
impl Global for ContextHub {}

impl ContextHub {
    pub fn publish(snapshot: Snapshot, cx: &mut App) {
        let hub = cx.global_mut::<Self>();
        hub.snapshots.insert(snapshot.module.clone(), snapshot);
    }
    pub fn remove(module: &str, cx: &mut App) {
        let hub = cx.global_mut::<Self>();
        hub.snapshots.remove(module);
        if hub.active.as_deref() == Some(module) { hub.active = None; }
    }
    /// The shell (or a module on focus) marks which module the user is in.
    pub fn set_active(module: &str, cx: &mut App) {
        cx.global_mut::<Self>().active = Some(SharedString::from(module.to_string()));
    }
    pub fn active(cx: &App) -> Option<Snapshot> {
        let hub = cx.global::<Self>();
        hub.active.as_ref().and_then(|id| hub.snapshots.get(id)).cloned()
    }
    pub fn get(module: &str, cx: &App) -> Option<Snapshot> {
        cx.global::<Self>().snapshots.get(module).cloned()
    }
    pub fn all(cx: &App) -> Vec<Snapshot> {
        cx.global::<Self>().snapshots.values().cloned().collect()
    }
}
