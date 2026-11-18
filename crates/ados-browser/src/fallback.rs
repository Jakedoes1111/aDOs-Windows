//! Servo hands a page to Chromium by itself: a site on the owner's "Open in Chromium" list, or a
//! page Servo could not load. One engine per Browser: the switch is `switch_engine`'s path.
use super::*;
use crate::{engine, protocol::Page, state::Engine};
use ados_core::settings;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::component::{button::{Button, ButtonVariants}, ActiveTheme, IconName};

/// Why the Browser moved to Chromium, or why it could not; shown as one line under the toolbar.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Notice {
    Listed,
    Failed,
    /// Chromium is not installed: the page stays in Servo.
    NoChromium,
}
impl Notice {
    fn sentence(&self) -> &'static str {
        match self {
            Self::Listed => "Opened in Chromium: this site is on your Chromium list.",
            Self::Failed => "Opened in Chromium: Servo could not load this page.",
            Self::NoChromium => "This page stays in Servo: no Chromium-family browser was found.",
        }
    }
}

#[derive(Default)]
pub(crate) struct Fallback {
    /// The line under the toolbar and the address it was shown for; `settled` once that page
    /// has finished loading (a redirect while it loads is not "the next navigation").
    notice: Option<(Notice, String, bool)>,
    /// "Back to Servo" was pressed at this site (host and port): it does not switch again until
    /// the shown page is somewhere else.
    held: Option<String>,
    /// The missing-Chromium sentence was shown once already this run.
    missing_said: bool,
}

/// Host and port, the unit "that address" is held by: a redirect inside a site keeps it.
fn site(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or_default().to_lowercase()
}

/// Servo's own load-failure page, as read over WebDriver (`http://127.0.0.1:1/` in the rig):
/// the address stays the one that failed, the title is "Error loading page" and the body is one
/// line "Could not load the requested page: <reason>" (nightly 2026-11-17).
pub(crate) fn servo_failure(page: &Page) -> bool {
    page.title == "Error loading page" && page.text.trim_start().starts_with("Could not load the requested page")
}

impl BrowserPanel {
    /// A page shown in the Browser (the selected tab, or a tile beside it) while Servo drives it:
    /// switch to Chromium when the owner's list or a load failure says so. True: switched.
    pub(crate) fn maybe_fall_back(&mut self, tab: Option<u64>, page: &Page, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.state.engine != Engine::Servo || page.url.is_empty() { return false; }
        let here = site(&page.url);
        if self.fallback.held.as_ref().is_some_and(|held| *held != here) { self.fallback.held = None; }
        if self.fallback.held.is_some() { return false; }
        let s = settings::get(cx);
        let reason = if settings::site_listed(&page.url, &s.chromium_sites) {
            Notice::Listed
        } else if s.chromium_on_failure && servo_failure(page) {
            Notice::Failed
        } else {
            return false;
        };
        self.fall_back(reason, tab, page.url.clone(), window, cx)
    }

    /// The owner typed an address on the list: open it in Chromium straight away, without
    /// loading it in Servo first. True: switched (the address is the tab's already).
    pub(crate) fn typed_fall_back(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.state.engine != Engine::Servo { return false; }
        if self.fallback.held.as_ref().is_some_and(|held| *held == site(url)) { return false; }
        self.fallback.held = None;
        if !settings::site_listed(url, &settings::get(cx).chromium_sites) { return false; }
        self.fall_back(Notice::Listed, None, url.to_string(), window, cx)
    }

    /// `tab`: a tile beside the selected tab (None: the selected tab) whose page is at `url`.
    fn fall_back(&mut self, reason: Notice, tab: Option<u64>, url: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !engine::chromium_found() {
            // Said once a run; the address is held so it is not tried again and again.
            self.fallback.held = Some(site(&url));
            eprintln!("Browser: {reason:?} at {url}, but no Chromium-family browser was found");
            if !self.fallback.missing_said {
                self.fallback.missing_said = true;
                self.fallback.notice = Some((Notice::NoChromium, url, false));
            }
                        return false;
        }
        eprintln!("Browser: switching to Chromium ({reason:?}) at {url}");
        // Every tab keeps its address; the page that asked for the switch opens where it is.
        match tab {
            Some(id) => { if let Some(t) = self.tabs.get_mut(id) { t.url = url.clone(); } }
            None => {
                self.state.url = url.clone();
                self.tabs.current_mut().url = url.clone();
                if !self.input.read(cx).focus_handle(cx).is_focused(window) {
                    self.input.update(cx, |input, cx| input.set_value(url.clone(), window, cx));
                }
            }
        }
        self.persist_tabs();
        self.set_engine(Engine::Chromium, window, cx);
        self.fallback.notice = Some((reason, url, false));
        true
    }

    /// A page of the selected tab was read: the line goes away at the next navigation.
    pub(crate) fn notice_page(&mut self, page: &Page) {
        let Some((_, url, settled)) = &mut self.fallback.notice else { return };
        if page.url.is_empty() || page.url == "about:blank" { return; }
        if *settled && *url != page.url {
            self.fallback.notice = None;
        } else if !page.loading {
            *url = page.url.clone();
            *settled = true;
        }
    }

    /// The owner moved on (typed an address, chose another tab): the line goes.
    pub(crate) fn end_notice(&mut self) { self.fallback.notice = None; }

    /// The engine button, or "Back to Servo": a switch to Servo holds the shown address, so
    /// the same page does not send the Browser back to Chromium.
    pub(crate) fn note_manual_switch(&mut self, to: Engine) {
        self.fallback.notice = None;
        if to == Engine::Servo { self.fallback.held = Some(site(&self.state.url)); }
    }

    pub(crate) fn notice_line(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (notice, _, _) = self.fallback.notice.as_ref()?;
        let theme = cx.theme();
        let (muted, fg, border) = (theme.muted_foreground, theme.foreground, theme.border);
        Some(div().id("browser-engine-notice").flex().items_center().gap_2().pl_3().pr_1().py_1().min_h(px(36.)).flex_shrink_0()
            .border_b_1().border_color(border).text_sm().text_color(fg)
            .child(gpui_kit::component::Icon::new(IconName::Globe).size(px(16.)).text_color(muted))
            .child(div().flex_1().min_w_0().child(notice.sentence()))
            .when(*notice != Notice::NoChromium, |line| line.child(
                Button::new("browser-back-to-servo").tab_index(2).label("Back to Servo").ghost().h(px(32.))
                    .tooltip("Show this page in Servo again")
                    .on_click(cx.listener(|this, _, window, cx| this.switch_engine(window, cx)))))
            .child(Button::new("browser-notice-dismiss").tab_index(2).icon(IconName::Close).ghost().size(px(32.))
                .tooltip("Dismiss")
                .on_click(cx.listener(|this, _, _, cx| { this.end_notice(); cx.notify(); })))
            .into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{servo_failure, site};
    use crate::protocol::Page;
    fn page(url: &str, title: &str, text: &str) -> Page { Page { title: title.into(), url: url.into(), text: text.into(), loading: false, history: 1 } }
    #[test]
    fn sites_hold_host_and_port() {
        assert_eq!(site("https://www.YouTube.com/watch?v=1"), "www.youtube.com");
        assert_eq!(site("http://127.0.0.1:1/"), "127.0.0.1:1");
        assert_ne!(site("http://127.0.0.1:1/"), site("http://127.0.0.1:8765/"));
    }
    #[test]
    fn failure_page_is_recognised() {
        assert!(servo_failure(&page("http://127.0.0.1:1/", "Error loading page", "Could not load the requested page: Request attempted on bad port")));
        assert!(!servo_failure(&page("http://127.0.0.1:8765/", "aDOs fixture", "aDOs fixture")));
        // A real page quoting the sentence is not the failure page.
        assert!(!servo_failure(&page("https://example.com/", "Servo errors", "Could not load the requested page: …")));
    }
}
