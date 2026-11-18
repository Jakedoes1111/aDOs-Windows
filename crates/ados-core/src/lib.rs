//! Shared contracts between the aDOs shell and its modules. Owned by the orchestrator.
//! Modules depend on this crate and on `gpui-kit`, never on each other.
pub mod actions;
pub mod ask;
pub mod capture;
pub mod hints;
pub mod hub;
pub mod links;
pub mod overlay;
pub mod notifications;
pub mod paths;
pub mod settings;
pub mod shortcuts;

/// Stable module ids. Also used as dock panel names.
pub mod ids {
    pub const HOME: &str = "home";
    pub const NOTES: &str = "notes";
    pub const TERMINAL: &str = "terminal";
    pub const BROWSER: &str = "browser";
    pub const AGENT: &str = "agent";
    pub const CALENDAR: &str = "calendar";
    pub const SESSIONS: &str = "sessions";
    pub const TASKS: &str = "tasks";
}

/// Brand colours that have no theme token.
pub mod brand {
    pub const BLUE: u32 = 0x376BFA;
    pub const RED: u32 = 0xF04B55;
}

pub fn init(cx: &mut gpui_kit::App) {
    cx.set_global(hub::ContextHub::default());
    cx.set_global(actions::ActionRegistry::default());
    cx.set_global(overlay::OverlayGuard::default());
    cx.set_global(overlay::Holes::default());
    cx.set_global(overlay::NativeRect::default());
    cx.set_global(notifications::LatestUndo::default());
}
