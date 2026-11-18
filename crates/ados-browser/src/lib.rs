//! Real browser windows embedded as X11 children of the GPUI workspace.
mod driver;
mod engine;
mod engine_keys;
mod nav;
mod focus;
mod hints;
mod page_hints;
mod protocol;
mod runtime;
mod state;
mod xembed;
mod cutout;
mod visibility;
mod view;
mod park;
mod open;
pub mod control;
// Comment mode and its block.
mod comment;
mod comment_block;
mod comment_view;
mod shot;
// Page tabs: the model and the strip.
mod tabs;
mod tab_strip;
// Tiled tabs and follower tabs: the model and the panel side.
mod tiles;
mod tile_view;
mod follow;
// Servo hands a page to Chromium by itself (owner's list, load failure).
mod fallback;

use ados_core::{
    hub::{ContextHub, Snapshot},
    ids,
    overlay::{LayoutEpoch, OverlayGuard},
};
use gpui_kit::base::dock::Panel as BasePanel;
use gpui_kit::component::{
    dock::{Panel, PanelEvent},
    input::{InputEvent, InputState},
    WindowExt,
};
use gpui_kit::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use runtime::{CommandMsg, Event, Runtime};
use state::State;
use std::sync::Arc;
use xembed::Rect;

actions!(ados_browser, [FocusAddress, Reload, Back, Forward, LinkHints, LinkHintsNew, NewTab, CloseTab, NextTab, PreviousTab, ReopenTab]);
#[derive(Default)]
struct Target(Option<WeakEntity<BrowserPanel>>);
impl Global for Target {}

pub fn init(cx: &mut App) {
    cx.set_global(Target::default());
    // Ctrl+L is the shell's `browser.focus_address`; one binding, not two.
    ados_core::shortcuts::bind(cx, "browser.reload", Reload);
    ados_core::shortcuts::bind(cx, "browser.back", Back);
    ados_core::shortcuts::bind(cx, "browser.forward", Forward);
    ados_core::shortcuts::bind(cx, "browser.new_tab", NewTab);
    ados_core::shortcuts::bind(cx, "browser.close_tab", CloseTab);
    ados_core::shortcuts::bind(cx, "browser.next_tab", NextTab);
    ados_core::shortcuts::bind(cx, "browser.previous_tab", PreviousTab);
    ados_core::shortcuts::bind(cx, "browser.reopen_tab", ReopenTab);
    // The page script owns these keys; the registry lists them so they can be rebound.
    ados_core::shortcuts::bind(cx, "browser.hints", LinkHints);
    ados_core::shortcuts::bind(cx, "browser.hints_new", LinkHintsNew);
    comment::init(cx);
    tile_view::init(cx);
    tab_strip::init(cx);
    ados_core::actions::register(
        "browser.focus_address",
        "Open URL",
        "browser navigate address search",
        None,
        cx,
        |window, cx| {
            if let Some(panel) = cx
                .global::<Target>()
                .0
                .as_ref()
                .and_then(WeakEntity::upgrade)
            {
                panel.update(cx, |panel, cx| panel.focus_address(window, cx));
            } else {
                window.push_notification("Open the Browser module first", cx);
            }
        },
    );
    ados_core::actions::register(
        "browser.reload",
        "Reload page",
        "browser refresh",
        None,
        cx,
        |window, cx| {
            if let Some(panel) = cx
                .global::<Target>()
                .0
                .as_ref()
                .and_then(WeakEntity::upgrade)
            {
                if panel.read(cx).active {
                    panel.update(cx, |panel, _| panel.reload());
                } else {
                    window.push_notification("Open the Browser module first", cx);
                }
            } else {
                window.push_notification("Open the Browser module first", cx);
            }
        },
    );
}

pub struct BrowserPanel {
    focus: FocusHandle,
    page_focus: FocusHandle,
    page_input: bool,
    input: Entity<InputState>,
    state: State,
    runtime: Option<Arc<Runtime>>,
    active: bool,
    ready: bool,
    error: Option<String>,
    title: String,
    parent: Option<u32>,
    /// Each shown page's rectangle sent to the host (physical pixels; tab 0 when not tiled).
    geometry: Vec<(u64, Rect)>,
    /// The whole page area and the window's scale, from the last paint.
    area: Option<Bounds<Pixels>>,
    scale: f32,
    narrow: bool,
    painted_frame: bool,
    /// A module tab is being dragged: the page hides so drop zones and ghost show.
    dragging: bool,
    checking_visibility: bool,
    page_bounds: Option<Bounds<Pixels>>,
    _subscriptions: Vec<Subscription>,
    quit_subscription: Option<Subscription>,
    generation: u64,
    shortcut_version: u64,
    /// aDOs hint mode holds the keyboard: do not hand it back to the page meanwhile.
    hint_hold: std::rc::Rc<std::cell::Cell<bool>>,
    /// Out of every layout: the engine stops when this timer fires (`park.rs`).
    park_task: Option<Task<()>>,
    parked: bool,
    /// The owner's selection in the page, for the context snapshot (`control.rs`).
    selection: String,
    /// Comment mode, its card and the comments waiting to be sent (`comment.rs`).
    comment: comment::Comments,
    /// Position in the page history: Back and Forward are disabled when there is nowhere to go (`nav.rs`).
    nav: nav::Nav,
    /// The page tabs (`tabs.rs`, `tab_strip.rs`).
    tabs: tabs::Tabs,
    /// Tiled tabs and followers (`tiles.rs`, `tile_view.rs`).
    tiles: tiles::Tiles,
    /// Tabs marked for tiling with Ctrl+click or Shift+click.
    marked: Vec<u64>,
    /// The divider being dragged.
    divider_drag: Option<(tiles::Axis, usize)>,
    divider_focus: Vec<FocusHandle>,
    /// The last press in the page was a Ctrl+click or middle click: the tab it opens is a
    /// background tab. Cleared by the next press or by that tab.
    open_behind: bool,
    /// The engine was started again by itself after it stopped (`Event::Failed`).
    restarted: bool,
    /// The tab strip's scroll, and the tab it last brought into view.
    strip_scroll: ScrollHandle,
    strip_shown: std::cell::Cell<Option<u64>>,
    /// Automatic switch to Chromium and its line under the toolbar (`fallback.rs`).
    fallback: fallback::Fallback,
}
impl BrowserPanel {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let panel = cx.new(|cx| Self::new(window, cx));
        if !cx.has_global::<Target>() {
            init(cx);
        }
        cx.global_mut::<Target>().0 = Some(panel.downgrade());
        panel
    }
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut state = State::load();
        let tabs = tabs::Tabs::restore(&state.tabs, state.selected, &state.url);
        state.url = tabs.current().url.clone();
        let tiles = tile_view::restore_tiles(&state, &tabs);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search or enter a URL")
                .default_value(state.url.clone())
        });
        let parent = window
            .window_handle()
            .ok()
            .and_then(|handle| match handle.as_raw() {
                RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok(),
                RawWindowHandle::Xcb(h) => Some(h.window.get()),
                _ => None,
            });
        let focus = cx.focus_handle();
        // Tab order inside the panel: address, Back/Forward/Reload, engine, page (`view.rs`).
        let page_focus = cx.focus_handle().tab_stop(true).tab_index(3);
        let subscriptions = vec![
            cx.on_focus_in(&page_focus, window, |this, window, cx| {
                this.page_input = true;
                this.remember(focus::Place::Page);
                this.sync_keyboard(window, cx);
                cx.notify();
            }),
            cx.on_focus_out(&page_focus, window, |this, _, window, cx| {
                this.page_input = false;
                this.sync_keyboard(window, cx);
                cx.notify();
            }),
            cx.on_focus_out(&focus, window, |this, _, _, _| {
                if let Some(runtime) = &this.runtime { runtime.focus_address(); }
            }),
            cx.observe_global_in::<ContextHub>(window, |this, window, cx| {
                this.sync_keyboard(window, cx);
                this.check_visibility(window, cx);
            }),
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Focus) {
                    this.remember(focus::Place::Address);
                    if let Some(runtime) = &this.runtime {
                        runtime.focus_address();
                    }
                }
                if let InputEvent::PressEnter { .. } = event {
                    let url = state::address(&this.input.read(cx).value(), this.state.engine);
                    this.state.url = url.clone();
                    this.tabs.current_mut().url = url.clone();
                    this.persist_tabs();
                    this.persist();
                    this.error = None;
                    this.end_notice();
                    this.input
                        .update(cx, |input, cx| input.set_value(url.clone(), window, cx));
                    this.page_focus.focus(window, cx);
                    // A site on the Chromium list opens there without loading in Servo first.
                    if !this.typed_fall_back(&url, window, cx) { this.send(CommandMsg::Navigate(url)); }
                    cx.notify();
                }
            }),
            cx.observe_global_in::<OverlayGuard>(window, |this, window, cx| {
                this.sync_visibility(cx);
                this.check_visibility(window, cx);
                this.sync_keyboard(window, cx);
            }),
            cx.observe_global::<ados_core::overlay::Cutout>(|this, cx| this.sync_visibility(cx)),
            cx.observe_global::<ados_core::overlay::Holes>(|this, cx| this.sync_visibility(cx)),
            cx.observe_global_in::<LayoutEpoch>(window, |this, window, cx| {
                // The dock changed: until the next paint proves where the page now belongs,
                // it must not stay drawn at its old position (over another module).
                this.painted_frame = false;
                this.sync_visibility(cx);
                this.check_visibility(window, cx);
            }),
            // The user's keymap changed: the page must hand back the new chords.
            cx.observe_global::<ados_core::shortcuts::Shortcuts>(|this, cx| {
                let version = ados_core::shortcuts::version(cx);
                if version != this.shortcut_version {
                    this.shortcut_version = version;
                    if let Some(runtime) = &this.runtime {
                        runtime.shortcuts(ados_core::shortcuts::native_keys(cx));
                        runtime.command(CommandMsg::Hints(hints::config(cx)));
                    }
                }
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                this.check_visibility(window, cx);
            }),
        ];
        let mut panel = Self {
            focus,
            page_focus,
            page_input: false,
            input,
            state,
            runtime: None,
            active: true,
            ready: false,
            error: None,
            title: "Browser".into(),
            parent,
            geometry: vec![],
            area: None,
            scale: 1.,
            narrow: false,
            painted_frame: false,
            dragging: false,
            checking_visibility: false,
            page_bounds: None,
            _subscriptions: subscriptions,
            quit_subscription: None,
            generation: 0,
            shortcut_version: ados_core::shortcuts::version(cx),
            hint_hold: Default::default(),
            park_task: None,
            parked: false,
            selection: String::new(),
            comment: Default::default(),
            nav: Default::default(),
            tabs,
            tiles,
            marked: vec![],
            divider_drag: None,
            divider_focus: (0..tiles::MAX * 2).map(|_| cx.focus_handle()).collect(),
            open_behind: false,
            restarted: false,
            strip_scroll: ScrollHandle::new(),
            strip_shown: Default::default(),
            fallback: Default::default(),
        };
        panel.keep_groups_together();
        panel.start(window, cx);
        panel
    }
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(old) = self.runtime.take() {
            old.stop();
        }
        self.quit_subscription = None;
        self.ready = false;
        self.nav.reset();
        self.geometry.clear();
        self.generation += 1;
        let generation = self.generation;
        let Some(parent) = self.parent else {
            self.error =
                Some("Embedded browsing requires the X11 backend. Start aDOs on X11.".into());
            return;
        };
        self.shortcut_version = ados_core::shortcuts::version(cx);
        let tab = self.tabs.current().id;
        match Runtime::start(parent, self.state.engine, tab, self.state.url.clone(), ados_core::shortcuts::native_keys(cx), hints::config(cx)) {
            Ok((runtime, events)) => {
                self.error = None;
                let quit_runtime = runtime.clone();
                self.quit_subscription = Some(cx.on_app_quit(move |_, _| {
                    quit_runtime.stop();
                    async {}
                }));
                let native = Arc::downgrade(&runtime);
                OverlayGuard::on_boundary(cx, std::rc::Rc::new(move |open| {
                    if let Some(runtime) = native.upgrade() { runtime.overlay(open); }
                }));
                if OverlayGuard::any(cx) { runtime.overlay(true); }
                self.runtime = Some(runtime);
                // Restored tiles and followers: the engine opens the other tiles beside the first page.
                if self.tiles.group_of(tab).is_some() {
                    self.send(CommandMsg::Follow(self.tiles.follows.clone()));
                    self.send_tiles();
                    self.send(CommandMsg::Select(tab, self.state.url.clone()));
                }
                self.sync_visibility(cx);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = events.recv().await {
                        if this
                            .update_in(cx, |this, window, cx| {
                                if this.generation == generation {
                                    this.event(event, window, cx);
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    fn event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Ready => {
                self.ready = true;
                self.sync_visibility(cx);
                self.sync_keyboard(window, cx);
            }
            Event::Error(error) => {
                self.error = Some(error);
            }
            // An engine that died (a crash, or killed with the last session) is started again
            // once without asking; a second stop before the owner's Retry shows the message.
            Event::Failed(_) if !self.restarted => {
                eprintln!("Browser engine stopped; starting it again");
                self.restarted = true;
                self.start(window, cx);
            }
            Event::Failed(error) => {
                self.ready = false;
                self.error = Some(error);
            }
            // GPUI hears nothing once the pointer is over the page: end hover and tooltips.
            // Deferred: it dispatches a mouse move that reaches this panel's own listeners.
            Event::PointerEntered => window.defer(cx, |window, cx| ados_core::overlay::clear_pointer(window, cx)),
            Event::PagePressed { .. } if self.commenting() => {
                ContextHub::set_active(ids::BROWSER, cx);
                self.comment_pressed(window, cx);
            }
            Event::PagePressed { behind } => {
                self.open_behind = behind;
                self.page_focus.focus(window, cx);
                ContextHub::set_active(ids::BROWSER, cx);
                self.refresh_selection_soon(window, cx);
            }
            // Esc while the page shows link hints cancels them and keeps the page focused.
            Event::PageExit => self.page_exit(window, cx),
            Event::Shortcut(index) => {
                // From inside the page, dispatch from the page proxy: an action that
                // does not move focus (Ctrl+S) must leave the keyboard in the page.
                let from_page = self.page_input;
                if !from_page { self.focus.focus(window, cx); }
                if let Some(action) = ados_core::shortcuts::native_action(index, cx) {
                    window.dispatch_action(action, cx);
                }
                // Keep subsequent native key events frozen until the dispatched
                // action has focused its GPUI target, then release them in order.
                let runtime = self.runtime.clone();
                let page_focus = self.page_focus.clone();
                let hold = self.hint_hold.clone();
                window.on_next_frame(move |window, _| {
                    window.on_next_frame(move |window, _| {
                        if let Some(runtime) = runtime {
                            if from_page && page_focus.is_focused(window) && !hold.get() { runtime.focus_page(); }
                            runtime.shortcut_done();
                        }
                    });
                    window.refresh();
                });
            },
            Event::Opened { id, opener, url, title } => self.tab_opened(id, opener, url, title, window, cx),
            Event::Closed(id) => self.tab_closed(id, window, cx),
            // The host already gave the pressed tile the keyboard; PagePressed follows.
            Event::TilePressed(id) => {
                if let Some(index) = self.tabs.index_of(id).filter(|_| id != self.tabs.current().id) { self.select_tab(index, window, cx); }
            }
            Event::Followed(id, url) => {
                // A link the owner clicked in a leader is a navigation: the switch line goes.
                self.end_notice();
                self.background_page(id, url, String::new())
            }
            Event::Page(id, page) if id != self.tabs.current().id => {
                // A tile shown beside the selected tab is on screen; a background tab waits until shown.
                if self.tiles.shown(self.tabs.current().id).contains(&id) && self.maybe_fall_back(Some(id), &page, window, cx) {
                    cx.notify();
                    return;
                }
                self.background_page(id, page.url, page.title)
            }
            Event::Page(_, mut page) => {
                // Chromium's own error page reads `chrome-error://chromewebdata/`: the tab keeps the
                // address that failed, so Reload, a relaunch or "Back to Servo" tries it again.
                if page.url.starts_with("chrome-error://") { page.url = self.state.url.clone(); }
                if page.url != "about:blank" && self.maybe_fall_back(None, &page, window, cx) {
                    cx.notify();
                    return;
                }
                self.notice_page(&page);
                // Chromium can expose an initial blank document while the
                // requested URL is still committing. Do not persist that as last URL.
                if page.url == "about:blank" && self.state.url != "about:blank" {
                    return;
                }
                if !page.url.is_empty() {
                    self.nav.page(&page.url, page.history);
                    self.comment_page(&page.url, window, cx);
                    self.tabs.current_mut().title = page.title.clone();
                    self.show_title(&page.title, window, cx);
                    if self.state.url != page.url {
                        self.selection.clear();
                        self.state.url = page.url.clone();
                        self.tabs.current_mut().url = page.url.clone();
                        self.persist_tabs();
                        self.persist();
                        // Never overwrite an address the user is still typing.
                        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
                            self.input.update(cx, |input, cx| {
                                input.set_value(page.url.clone(), window, cx)
                            });
                        }
                    }
                    ContextHub::publish(
                        Snapshot {
                            module: ids::BROWSER.into(),
                            title: page.title.into(),
                            location: Some(page.url.into()),
                            text: page.text.into(),
                            selection: (!self.selection.is_empty()).then(|| self.selection.clone().into()),
                        },
                        cx,
                    );
                }
            }
        }
        self.sync_visibility(cx);
        cx.notify();
    }
    fn persist(&mut self) {
        if let Err(e) = self.state.save() {
            self.error = Some(format!("Could not save browser state: {e}"));
        }
    }
    /// Back (-1) or Forward (1); nothing happens when the history has nowhere to go.
    fn go(&mut self, delta: i32) {
        if self.nav.go(delta) {
            self.send(if delta < 0 { CommandMsg::Back } else { CommandMsg::Forward });
        }
    }
    /// Reload always loads the address the bar shows, even when the engine page is blank or a different one.
    fn reload(&mut self) {
        if self.nav.url() == self.state.url { self.send(CommandMsg::Reload) } else { self.send(CommandMsg::Navigate(self.state.url.clone())) }
    }
    fn send(&self, msg: CommandMsg) {
        if let Some(runtime) = &self.runtime {
            runtime.command(msg);
        }
    }
    /// Release the native X child before handing the keyboard to another module.
    pub fn release_focus(&self) {
        if let Some(runtime) = &self.runtime { runtime.focus_address(); }
    }
    /// Entering the Browser: the keyboard goes back to where it last was inside it (F2, F8).
    pub fn focus_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.commenting() { return self.comment_focus(window, cx); }
        self.active = true;
        match focus::entry_place(self.state.focus, &self.state.url) {
            focus::Place::Page if self.error.is_none() => self.focus_page(window, cx),
            _ => {
                if let Some(runtime) = &self.runtime { runtime.focus_address(); }
                // The whole address is selected, so typing replaces it (never prepends to it).
                self.input.update(cx, |input, cx| {
                    input.focus_handle(cx).focus(window, cx);
                    input.select_all(window, cx);
                });
            }
        }
        ContextHub::set_active(ids::BROWSER, cx);
    }
    /// Keyboard into the page (before the engine is attached the proxy holds it; `Ready` hands X focus on).
    pub fn focus_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active = true;
        self.page_focus.focus(window, cx);
        self.sync_visibility(cx);
        self.check_visibility(window, cx);
        self.sync_keyboard(window, cx);
        ContextHub::set_active(ids::BROWSER, cx);
    }
    /// The one rule for X focus on the page window (focus.rs `Owner`); every path calls it.
    fn sync_keyboard(&self, window: &Window, cx: &App) {
        let Some(runtime) = &self.runtime else { return };
        let owner = focus::Owner {
            page_focused: self.page_focus.is_focused(window),
            module_active: ContextHub::active(cx).is_none_or(|s| s.module.as_ref() == ids::BROWSER),
            overlay: OverlayGuard::any(cx),
            shown: self.ready && self.active,
        };
        if owner.page_has_keyboard() {
            // Hint mode holds the keyboard in aDOs until it ends.
            if !self.hint_hold.get() { runtime.focus_page(); }
        } else {
            runtime.focus_address();
        }
    }
    /// A key that reached the page proxy in GPUI: typed straight after the chord that entered
    /// the Browser, before the page held X focus. The page gets it (F4).
    fn typed_on_proxy(&self, event: &KeyDownEvent) {
        let m = event.keystroke.modifiers;
        if m.control || m.alt || m.platform || m.function { return; }
        if let (Some(runtime), Some(keysym)) = (&self.runtime, focus::typed_keysym(&event.keystroke.key)) {
            runtime.typed(keysym);
        }
    }
    fn remember(&mut self, place: focus::Place) {
        if self.state.focus != Some(place) {
            self.state.focus = Some(place);
            self.persist();
        }
    }

    pub fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(runtime) = &self.runtime {
            runtime.focus_address();
        }
        self.input.update(cx, |input, cx| {
            input.focus_handle(cx).focus(window, cx);
            input.select_all(window, cx);
        });
        ContextHub::set_active(ids::BROWSER, cx);
    }
    /// The engine button and "Back to Servo".
    fn switch_engine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.state.engine.other();
        self.note_manual_switch(to);
        self.set_engine(to, window, cx);
    }
    /// One engine per Browser: every tab, tile and follower keeps its address (`start`).
    fn set_engine(&mut self, to: state::Engine, window: &mut Window, cx: &mut Context<Self>) {
        self.state.engine = to;
        self.persist();
        self.start(window, cx);
        cx.notify();
    }
}
impl Drop for BrowserPanel {
    fn drop(&mut self) {
        if let Some(runtime) = &self.runtime {
            runtime.stop();
        }
    }
}
impl EventEmitter<PanelEvent> for BrowserPanel {}
impl Focusable for BrowserPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl BasePanel for BrowserPanel {
    fn panel_name(&self) -> &'static str {
        ids::BROWSER
    }
    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.active = active;
        self.sync_visibility(cx);
        self.check_visibility(window, cx);
        self.sync_keyboard(window, cx);
        cx.notify();
    }
    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        // The shell caches modules across layout replacement, including its
        // first-run template setup. Removal is NOT the panel's lifetime end.
        self.active = false;
        self.sync_visibility(cx);
    }
}
impl Panel for BrowserPanel {
    /// The tab shows the page title, trimmed; the rail tooltip stays "Browser".
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        tab_title(&self.title)
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

fn tab_title(page: &str) -> String {
    let page = page.split_whitespace().collect::<Vec<_>>().join(" ");
    if page.is_empty() || page == "about:blank" { return "Browser".into(); }
    let mut chars = page.chars();
    let head: String = chars.by_ref().take(24).collect();
    if chars.next().is_some() { format!("{}…", head.trim_end()) } else { head }
}

#[cfg(test)]
mod tab_title_tests {
    use super::tab_title;
    #[test]
    fn trimmed_or_browser() {
        assert_eq!(tab_title(""), "Browser");
        assert_eq!(tab_title("  a 
 b "), "a b");
        assert_eq!(tab_title(&"x".repeat(40)), format!("{}…", "x".repeat(24)));
    }
}
