//! Every link in aDOs opens in the aDOs Browser: the shell's side of `ados_core::links`.
use crate::{modules, shell::Shell, storage};
use ados_core::ids;
use gpui_kit::component::WindowExt;
use gpui_kit::*;
use serde::Deserialize;

#[derive(Action, Clone, PartialEq, Eq, Deserialize)]
#[action(namespace = ados, no_json)]
pub struct OpenInBrowser {
    pub text: String,
}

pub fn init(cx: &mut App) {
    ados_core::links::set_opener(
        |text, window, cx| window.dispatch_action(Box::new(OpenInBrowser { text: text.into() }), cx),
        cx,
    );
    ados_core::actions::register(
        "browser.system_open",
        "Open page in system browser",
        "external default browser link",
        None,
        cx,
        |window, cx| match ados_core::hub::ContextHub::get(ids::BROWSER, cx).and_then(|s| s.location) {
            Some(url) => ados_core::links::open_in_system_browser(&url, window, cx),
            None => window.push_notification("The Browser has no page open yet", cx),
        },
    );
}

impl Shell {
    pub(crate) fn has_browser(&self, cx: &App) -> bool {
        self.panels(cx).iter().any(|p| p.panel_name(cx) == ids::BROWSER)
    }
    /// Show the Browser (switching to a layout that has one if this one does not) and load `text`.
    pub(crate) fn open_in_browser(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !cfg!(feature = "browser") {
            // No Browser in this build (Windows for now): links go to the system browser.
            let t = text.trim();
            let url = if t.contains("://") || t.contains(char::is_whitespace) { t.to_string() } else { format!("https://{t}") };
            return ados_core::links::open_in_system_browser(&url, window, cx);
        }
        if self.home {
            self.resume_layout(window, cx);
        }
        if !self.has_browser(cx) {
            let Some(name) = self.layout_with_browser(cx) else {
                self.error = Some("No layout has a Browser. Add the Browser to a layout, then open the link again.".into());
                cx.notify();
                return;
            };
            self.switch(&name, window, cx);
            if !self.has_browser(cx) {
                return;
            }
        }
        self.focus_module(ids::BROWSER, window, cx);
        #[cfg(feature = "browser")]
        if let Some(browser) = modules::browser_panel(cx) {
            browser.update(cx, |b, cx| b.open_text(text, window, cx));
            return;
        }
        let _ = text;
        self.error = Some("The Browser is not available in this build.".into());
        cx.notify();
    }
    fn layout_with_browser(&self, _cx: &App) -> Option<String> {
        let has = |state: &gpui_kit::base::dock::DockAreaState| {
            serde_json::to_string(state).is_ok_and(|s| s.contains("\"browser\""))
        };
        storage::names().into_iter().find(|name| {
            *name != self.name
                && match self.arrangements.get(name) {
                    Some(state) => has(state),
                    None => storage::load(name).is_ok_and(|l| has(&l.dock)),
                }
        })
    }
}
