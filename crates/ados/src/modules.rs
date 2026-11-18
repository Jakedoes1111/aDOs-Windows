//! The only integration table: change one constructor per module to replace a placeholder.
use ados_core::ids;
use gpui_kit::base::dock::{self as base};
use gpui_kit::component::{dock::panel_handle, IconName};
#[cfg(not(all(feature = "notes", feature = "terminal", feature = "browser", feature = "calendar", feature = "sessions", feature = "tasks")))]
use gpui_kit::component::{dock::Panel, v_flex, ActiveTheme, Icon};
use gpui_kit::*;
use std::{collections::HashMap, sync::Arc};

/// Layout trees own containers; this session cache owns the live modules.
#[derive(Default)]
struct LiveModules(HashMap<&'static str, Arc<dyn base::PanelView>>);
impl Global for LiveModules {}

pub type Constructor = fn(&mut Window, &mut App) -> Arc<dyn base::PanelView>;
pub struct Module {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: IconName,
    pub constructor: Constructor,
}

impl Module {
    pub fn panel(&self, window: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
        if let Some(panel) = cx.global::<LiveModules>().0.get(self.id) {
            return panel.clone();
        }
        let panel = (self.constructor)(window, cx);
        cx.global_mut::<LiveModules>().0.insert(self.id, panel.clone());
        panel
    }
}

pub const MODULES: &[Module] = &[
    Module {
        id: ids::HOME,
        title: "Home",
        icon: IconName::LayoutDashboard,
        constructor: home,
    },
    Module {
        id: ids::NOTES,
        title: "Notes",
        icon: IconName::FileText,
        constructor: notes,
    },
    Module {
        id: ids::TERMINAL,
        title: "Terminal",
        icon: IconName::SquareTerminal,
        constructor: terminal,
    },
    Module {
        id: ids::BROWSER,
        title: "Browser",
        icon: IconName::Globe,
        constructor: browser,
    },
    Module {
        id: ids::CALENDAR,
        title: "Calendar",
        icon: IconName::Calendar,
        constructor: calendar,
    },
    Module {
        id: ids::SESSIONS,
        title: "Sessions",
        icon: IconName::GalleryVerticalEnd,
        constructor: sessions,
    },
    Module {
        id: ids::TASKS,
        title: "Tasks",
        icon: IconName::CircleCheck,
        constructor: tasks,
    },
];

/// One entry point for rail, shortcuts, palette, tab focus and layout restore.
pub fn focus_primary(id: &str, window: &mut Window, cx: &mut App) {
    #[cfg(feature = "browser")]
    if let Some(panel) = cx.global::<LiveModules>().0.get(ids::BROWSER) {
        if let Ok(browser) = panel.view().downcast::<ados_browser::BrowserPanel>() {
            browser.read(cx).release_focus();
        }
    }
    let Some(panel) = cx.global::<LiveModules>().0.get(id).cloned() else { return; };
    if let Ok(home) = panel.view().downcast::<crate::home::HomePanel>() {
        home.update(cx, |s, cx| s.focus_primary(window, cx));
        return;
    }
    #[cfg(feature = "notes")]
    if let Ok(notes) = panel.view().downcast::<ados_notes::NotesPanel>() {
        notes.update(cx, |s, cx| s.focus_primary(window, cx));
        return;
    }
    #[cfg(feature = "terminal")]
    if let Ok(terminal) = panel.view().downcast::<ados_terminal::TerminalPanel>() {
        terminal.update(cx, |s, cx| s.focus_primary(window, cx));
        return;
    }
    #[cfg(feature = "browser")]
    if let Ok(browser) = panel.view().downcast::<ados_browser::BrowserPanel>() {
        browser.update(cx, |b, cx| b.focus_primary(window, cx));
        return;
    }
    #[cfg(feature = "calendar")]
    if let Ok(calendar) = panel.view().downcast::<ados_calendar::CalendarPanel>() {
        calendar.update(cx, |c, cx| c.focus_primary(window, cx));
        return;
    }
    #[cfg(feature = "sessions")]
    if let Ok(sessions) = panel.view().downcast::<ados_sessions::SessionsPanel>() {
        sessions.update(cx, |s, cx| s.focus_primary(window, cx));
        return;
    }
    #[cfg(feature = "tasks")]
    if let Ok(tasks) = panel.view().downcast::<ados_tasks::TasksPanel>() {
        tasks.update(cx, |t, cx| t.focus_primary(window, cx));
        return;
    }
    window.focus(&panel.focus_handle(cx), cx);
}

/// A focus handle on the module's root element: focus on any part of the module is inside it.
/// Terminal and Browser already return their root handle from `focus_handle`.
pub fn root_focus(panel: &Arc<dyn base::PanelView>, cx: &App) -> FocusHandle {
    let view = panel.view();
    if let Ok(home) = view.clone().downcast::<crate::home::HomePanel>() {
        return home.read(cx).root_focus();
    }
    #[cfg(feature = "notes")]
    if let Ok(notes) = view.clone().downcast::<ados_notes::NotesPanel>() {
        return notes.read(cx).root_focus();
    }
    #[cfg(feature = "calendar")]
    if let Ok(calendar) = view.clone().downcast::<ados_calendar::CalendarPanel>() {
        return calendar.read(cx).root_focus();
    }
    #[cfg(feature = "sessions")]
    if let Ok(sessions) = view.clone().downcast::<ados_sessions::SessionsPanel>() {
        return sessions.read(cx).root_focus();
    }
    #[cfg(feature = "tasks")]
    if let Ok(tasks) = view.downcast::<ados_tasks::TasksPanel>() {
        return tasks.read(cx).root_focus();
    }
    panel.focus_handle(cx)
}

/// The live Browser panel, for hint mode.
#[cfg(feature = "browser")]
pub fn browser_panel(cx: &App) -> Option<Entity<ados_browser::BrowserPanel>> {
    let panel = cx.global::<LiveModules>().0.get(ids::BROWSER)?;
    panel.view().downcast::<ados_browser::BrowserPanel>().ok()
}

pub fn focus_browser_address(window: &mut Window, cx: &mut App) {
    #[cfg(feature = "browser")]
    if let Some(panel) = cx.global::<LiveModules>().0.get(ids::BROWSER).cloned() {
        if let Ok(browser) = panel.view().downcast::<ados_browser::BrowserPanel>() {
            browser.update(cx, |b, cx| b.focus_address(window, cx));
        }
    }
    #[cfg(not(feature = "browser"))]
    let _ = (window, cx);
}

/// Browser and Terminal panels in no layout let go of their processes after the agent
/// backends' idle delay (`backends.toml` `[settings] idle_stop_secs`); back in a layout,
/// they start again. Called on every dock change.
pub fn sync_in_layout(names: &[String], window: &mut Window, cx: &mut App) {
    let idle = std::time::Duration::from_secs(ados_agents::config::settings().idle_stop_secs);
    let inside = |id: &str| names.iter().any(|n| n == id);
    #[cfg(feature = "browser")]
    if let Some(browser) = browser_panel(cx) {
        let here = inside(ids::BROWSER);
        browser.update(cx, |b, cx| b.set_in_layout(here, idle, window, cx));
    }
    #[cfg(feature = "terminal")]
    if let Some(panel) = cx.global::<LiveModules>().0.get(ids::TERMINAL).cloned() {
        if let Ok(terminal) = panel.view().downcast::<ados_terminal::TerminalPanel>() {
            let here = inside(ids::TERMINAL);
            terminal.update(cx, |t, cx| t.set_in_layout(here, idle, window, cx));
        }
    }
    let _ = (idle, inside, window);
}

pub fn get(id: &str) -> Option<&'static Module> {
    MODULES.iter().find(|m| m.id == id)
}

pub fn register(cx: &mut App) {
    cx.set_global(LiveModules::default());
    #[cfg(feature = "notes")]
    ados_notes::init(cx);
    #[cfg(feature = "terminal")]
    ados_terminal::init(cx);
    #[cfg(feature = "browser")]
    ados_browser::init(cx);
    #[cfg(feature = "palette")]
    ados_palette::init(cx);
    #[cfg(feature = "assistant")]
    ados_assistant::init(cx);
    #[cfg(feature = "calendar")]
    ados_calendar::init(cx);
    #[cfg(feature = "sessions")]
    ados_sessions::init(cx);
    #[cfg(feature = "tasks")]
    ados_tasks::init(cx);
    for module in MODULES {
        base::register_panel(cx, module.id, move |_, window, cx| {
            module.panel(window, cx)
        });
    }
}

/// Stands in for a module left out of this build (only compiled when one is).
#[cfg(not(all(feature = "notes", feature = "terminal", feature = "browser", feature = "calendar", feature = "sessions", feature = "tasks")))]
fn placeholder(id: &'static str, cx: &mut App) -> Arc<dyn base::PanelView> {
    panel_handle(cx.new(|cx| PlaceholderPanel {
        id,
        focus: cx.focus_handle(),
    }))
}
fn home(window: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    panel_handle(crate::home::HomePanel::view(window, cx))
}
#[cfg(not(feature = "notes"))]
fn notes(_: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    placeholder(ids::NOTES, cx)
}
#[cfg(feature = "notes")]
fn notes(window: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    let notes = ados_notes::NotesPanel::view(window, cx);
    cx.subscribe(&notes, |_, _: &ados_notes::panel::NotesChanged, cx| cx.refresh_windows()).detach();
    panel_handle(notes)
}
#[cfg(not(feature = "terminal"))]
fn terminal(_: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    placeholder(ids::TERMINAL, cx)
}
#[cfg(feature = "terminal")]
fn terminal(window: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    panel_handle(ados_terminal::TerminalPanel::view(window, cx))
}
#[cfg(not(feature = "browser"))]
fn browser(_: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    placeholder(ids::BROWSER, cx)
}
#[cfg(feature = "browser")]
fn browser(window: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    panel_handle(ados_browser::BrowserPanel::view(window, cx))
}
#[cfg(not(feature = "calendar"))]
fn calendar(_: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    placeholder(ids::CALENDAR, cx)
}
#[cfg(feature = "calendar")]
fn calendar(window: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    panel_handle(ados_calendar::CalendarPanel::view(window, cx))
}
#[cfg(not(feature = "sessions"))]
fn sessions(_: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    placeholder(ids::SESSIONS, cx)
}
#[cfg(feature = "sessions")]
fn sessions(window: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    panel_handle(ados_sessions::SessionsPanel::view(window, cx))
}
#[cfg(not(feature = "tasks"))]
fn tasks(_: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    placeholder(ids::TASKS, cx)
}
#[cfg(feature = "tasks")]
fn tasks(window: &mut Window, cx: &mut App) -> Arc<dyn base::PanelView> {
    panel_handle(ados_tasks::TasksPanel::view(window, cx))
}
#[cfg(not(all(feature = "notes", feature = "terminal", feature = "browser", feature = "calendar", feature = "sessions", feature = "tasks")))]
struct PlaceholderPanel {
    id: &'static str,
    focus: FocusHandle,
}
#[cfg(not(all(feature = "notes", feature = "terminal", feature = "browser", feature = "calendar", feature = "sessions", feature = "tasks")))]
impl Focusable for PlaceholderPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
#[cfg(not(all(feature = "notes", feature = "terminal", feature = "browser", feature = "calendar", feature = "sessions", feature = "tasks")))]
impl EventEmitter<base::PanelEvent> for PlaceholderPanel {}
#[cfg(not(all(feature = "notes", feature = "terminal", feature = "browser", feature = "calendar", feature = "sessions", feature = "tasks")))]
impl base::Panel for PlaceholderPanel {
    fn panel_name(&self) -> &'static str {
        self.id
    }
}
#[cfg(not(all(feature = "notes", feature = "terminal", feature = "browser", feature = "calendar", feature = "sessions", feature = "tasks")))]
impl Panel for PlaceholderPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(get(self.id).map(|m| m.title).unwrap_or(self.id))
            .into_any_element()
    }
}
#[cfg(not(all(feature = "notes", feature = "terminal", feature = "browser", feature = "calendar", feature = "sessions", feature = "tasks")))]
impl Render for PlaceholderPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let module = get(self.id);
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|s, _, w, cx| w.focus(&s.focus, cx)),
            )
            .bg(cx.theme().background)
            .text_color(cx.theme().muted_foreground)
            .child(
                Icon::new(
                    module
                        .map(|m| m.icon.clone())
                        .unwrap_or(IconName::LayoutDashboard),
                )
                .size_8(),
            )
            .child(module.map(|m| m.title).unwrap_or(self.id))
    }
}
