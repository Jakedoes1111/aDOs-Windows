//! Page tabs in the Browser: the thin strip above the toolbar, the tab chords, and the engine
//! events for tabs a page opened or closed (`driver.rs`). The model is `tabs.rs`.
use super::*;
use gpui_kit::component::{button::{Button, ButtonVariants}, ActiveTheme, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use ados_core::hints::marked as marked_el;
use gpui_kit::component::{menu::ContextMenuExt, Icon};

const TAB_WIDTH: f32 = 180.;
/// Narrowest a tab gets (a few words and its close button); past that the strip scrolls.
const TAB_MIN: f32 = 112.;

/// Chord id, palette title, keywords; the palette runs the same action as the chord.
const ENTRIES: &[(&str, &str, &str)] = &[
    ("browser.new_tab", "New tab", "browser page tab new open"),
    ("browser.close_tab", "Close tab", "browser page tab close"),
    ("browser.next_tab", "Next tab", "browser page tab next switch"),
    ("browser.previous_tab", "Previous tab", "browser page tab previous switch"),
    ("browser.reopen_tab", "Reopen closed tab", "browser page tab reopen restore undo closed"),
];

pub(crate) fn init(cx: &mut App) {
    for (id, title, keywords) in ENTRIES {
        let id: &'static str = id;
        ados_core::actions::register(id, title, keywords, None, cx, move |window, cx| {
            let panel = cx.global::<Target>().0.as_ref().and_then(WeakEntity::upgrade);
            match panel {
                Some(panel) if panel.read(cx).active => panel.update(cx, |panel, cx| panel.tab_action(id, window, cx)),
                _ => window.push_notification("Open the Browser module first", cx),
            }
        });
    }
}

impl BrowserPanel {
    pub(crate) fn tab_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "browser.new_tab" => self.new_tab(window, cx),
            "browser.close_tab" => self.close_tab(None, window, cx),
            "browser.next_tab" => self.cycle_tab(1, window, cx),
            "browser.previous_tab" => self.cycle_tab(-1, window, cx),
            "browser.reopen_tab" => self.reopen_tab(window, cx),
            _ => {}
        }
    }
    /// Show tab `index`: the address bar, title and Back/Forward follow it; the keyboard stays
    /// where it was inside the Browser.
    pub(crate) fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.list.len() { return; }
        let leaving = std::mem::take(&mut self.nav);
        self.tabs.current_mut().nav = leaving;
        self.tabs.select(index);
        let tab = self.tabs.current().clone();
        self.nav = tab.nav.clone();
        self.state.url = tab.url.clone();
        self.selection.clear();
        self.error = None;
        self.end_notice();
        // A tab a page opened behind may not have its address read yet: it is not a blank tab.
        let blank = focus::is_blank(&tab.url) && tab.opener.is_none();
        let shown = if focus::is_blank(&tab.url) { String::new() } else { tab.url.clone() };
        self.input.update(cx, |input, cx| input.set_value(shown, window, cx));
        self.show_title(&tab.title, window, cx);
        self.send_tiles();
        self.send(CommandMsg::Select(tab.id, tab.url.clone()));
        self.persist_tabs();
        // A blank tab is for typing an address; any other keeps the keyboard where it was.
        if blank && self.active { self.focus_address(window, cx); } else { self.sync_keyboard(window, cx); }
        cx.notify();
    }
    pub(crate) fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab("about:blank".into(), window, cx);
    }
    pub(crate) fn open_tab(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab_at(None, url, window, cx);
    }
    /// A new tab at `url`, at `at` in the strip (else right of the shown one), selected.
    fn open_tab_at(&mut self, at: Option<usize>, url: String, window: &mut Window, cx: &mut Context<Self>) {
        let leaving = std::mem::take(&mut self.nav);
        self.tabs.current_mut().nav = leaving;
        let index = match at { Some(at) => self.tabs.insert(at, &url), None => self.tabs.add(&url) };
        self.tabs.select(index);
        self.keep_groups_together();
        let id = self.tabs.current().id;
        self.state.url = url.clone();
        self.selection.clear();
        self.end_notice();
        self.input.update(cx, |input, cx| input.set_value(if focus::is_blank(&url) { String::new() } else { url.clone() }, window, cx));
        self.show_title("", window, cx);
        self.send(CommandMsg::NewTab(id, url.clone()));
        self.persist_tabs();
        if focus::is_blank(&url) { self.focus_address(window, cx); } else { self.focus_page(window, cx); }
        cx.notify();
    }
    /// Ctrl+W in the Browser: close the page tab. The last tab stays and the Browser hides, as
    /// Ctrl+W does in every other module.
    pub(crate) fn close_tab(&mut self, id: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.unwrap_or(self.tabs.current().id);
        if self.tabs.list.len() == 1 {
            if let Some(hide) = ados_core::actions::ActionRegistry::all(cx).into_iter().find(|a| a.id.as_ref() == "shell.close") {
                window.defer(cx, move |window, cx| (hide.run)(window, cx));
            }
            return;
        }
        self.send(CommandMsg::Close(id));
        let was_selected = self.tabs.current().id == id;
        let was_shown = self.tiles.shown(self.tabs.current().id).contains(&id);
        self.tile_closed(id);
        if let Some(next) = self.tabs.remove(id, true) {
            // `remove` already moved the selection; show it.
            self.tabs.selected = next;
            let tab = self.tabs.current().clone();
            self.nav = tab.nav.clone();
            self.select_tab(next, window, cx);
        } else if was_shown {
            // A tile beside the shown one closed: the group is shown without it.
            self.select_tab(self.tabs.selected, window, cx);
        } else if !was_selected {
            self.persist_tabs();
        }
        cx.notify();
    }
    pub(crate) fn reopen_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((url, at)) = self.tabs.reopen() { self.open_tab_at(Some(at), url, window, cx); }
    }
    pub(crate) fn cycle_tab(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.list.len() > 1 { self.select_tab(self.tabs.step(delta), window, cx); }
    }

    /// A page opened a tab or window. It is selected with the keyboard in it, unless another
    /// module or an overlay owns the keyboard (F6): then it waits in the background, marked.
    pub(crate) fn tab_opened(&mut self, id: u64, opener: Option<u64>, url: String, title: String, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.adopt(id, opener, url, title);
        self.keep_groups_together();
        let Some(index) = self.tabs.index_of(id) else { return };
        // A Ctrl+click or middle click on a link opens it behind, as in other browsers.
        let behind = std::mem::take(&mut self.open_behind);
        let owns_keyboard = !behind && self.active && self.ready
            && ContextHub::active(cx).is_none_or(|s| s.module.as_ref() == ids::BROWSER)
            && !OverlayGuard::any(cx);
        if owns_keyboard {
            self.select_tab(index, window, cx);
            self.focus_page(window, cx);
        } else {
            self.tabs.list[index].unseen = true;
            self.persist_tabs();
            // The engine may have shown the new tab in the shown tab's window: show ours again.
            if behind {
                let tab = self.tabs.current().clone();
                self.send(CommandMsg::Select(tab.id, tab.url));
            }
            self.sync_keyboard(window, cx);
        }
        cx.notify();
    }
    /// A tab closed itself (`window.close()`, the end of a sign-in): the tab that opened it is
    /// shown again, so the page that waits for the sign-in gets it.
    pub(crate) fn tab_closed(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let was_selected = self.tabs.current().id == id;
        let was_shown = self.tiles.shown(self.tabs.current().id).contains(&id);
        self.tile_closed(id);
        let next = self.tabs.remove(id, false);
        if self.tabs.list.is_empty() {
            let start = self.state.engine.start_page().to_string();
            self.tabs = tabs::Tabs::restore(&[start], 0, "");
            let tab = self.tabs.current().clone();
            self.send(CommandMsg::NewTab(tab.id, tab.url.clone()));
            self.nav = Default::default();
            self.select_tab(0, window, cx);
        } else if let Some(next) = next.filter(|_| was_selected) {
            self.nav = self.tabs.list[next].nav.clone();
            self.select_tab(next, window, cx);
        } else if was_shown {
            self.select_tab(self.tabs.selected, window, cx);
        } else {
            self.persist_tabs();
        }
        cx.notify();
    }
    /// A snapshot of a tab that is not shown (Chromium reports every tab): keep its title.
    pub(crate) fn background_page(&mut self, id: u64, url: String, title: String) {
        if let Some(tab) = self.tabs.get_mut(id) {
            if !url.is_empty() && url != "about:blank" { tab.url = url; }
            tab.title = title;
            self.persist_tabs();
        }
    }
    pub(crate) fn show_title(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.title != title {
            self.title = title.to_string();
            let shown = Some(tab_title(&self.title)).filter(|t| t != "Browser");
            if ados_core::overlay::set_tab_title(ids::BROWSER, shown, cx) { window.refresh(); }
        }
    }
    pub(crate) fn persist_tabs(&mut self) {
        let urls = self.tabs.urls();
        let (groups, follows) = self.saved_tiles();
        if self.state.tabs != urls || self.state.selected != self.tabs.selected || self.state.tile_groups != groups || self.state.follows != follows {
            self.state.tabs = urls;
            self.state.selected = self.tabs.selected;
            self.state.tile_groups = groups;
            self.state.follows = follows;
            self.persist();
        }
    }

    pub(crate) fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.tabs.selected;
        let current_id = self.tabs.current().id;
        let shown = self.tiles.shown(current_id);
        let weak = cx.entity().downgrade();
        let mut items: Vec<AnyElement> = vec![];
        let mut run: Vec<AnyElement> = vec![];
        let mut run_group: Option<usize> = None;
        let mut selected_item = 0;
        let list = &self.tabs.list;
        for (index, tab) in list.iter().enumerate() {
            let group = self.tiles.group_of(tab.id);
            if group != run_group {
                self.flush_group(&mut items, &mut run, run_group, cx);
                run_group = group;
            }
            // Its place among the strip's items: a group is one item, other tabs one each.
            if index == selected { selected_item = items.len() + if group.is_some() { 0 } else { run.len() }; }
            let current = index == selected;
            let label = strip_label(tab);
            let id = tab.id;
            let marked = self.marked.contains(&id);
            let leader = self.tiles.leader_of(id).and_then(|l| list.iter().find(|t| t.id == l)).map(strip_label);
            let tiled = group.is_some();
            let following = leader.is_some();
            let tooltip_label = match &leader { Some(leader) => SharedString::from(format!("{label}\nFollows {leader}: links clicked there open here")), None => label.clone() };
            let menu_weak = weak.clone();
            let el = div().id(("browser-tab", id as usize))
                .flex().items_center().gap_1().h(px(34.)).pl_2().w(px(TAB_WIDTH)).flex_shrink_1().min_w(px(TAB_MIN))
                .border_b_2()
                // The tab with the keyboard: blue underline like the dock's tabs. Its tiles beside it read in full colour.
                .when(current, |el| el.border_color(cx.theme().primary).text_color(cx.theme().foreground))
                .when(!current, |el| el.border_color(gpui_kit::transparent_black())
                    .text_color(if shown.contains(&id) { cx.theme().foreground } else { cx.theme().muted_foreground })
                    .hover(|el| el.bg(cx.theme().secondary_hover)))
                .when(marked, |el| el.bg(cx.theme().primary.opacity(0.18)))
                .child(marked_el(div().id(("browser-tab-select", id as usize)).flex_1().min_w_0().h(px(32.)).flex().items_center().gap_1()
                    .when(tab.unseen, |el| el.child(div().size(px(6.)).rounded_full().flex_shrink_0().bg(cx.theme().primary)))
                    .when(following, |el| el.child(Icon::new(gpui_kit::assets::IconName::ArrowRight).xsmall().flex_shrink_0().text_color(cx.theme().primary)))
                    .child(div().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().text_sm().child(label.clone()))
                    .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tooltip_label.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        let m = event.modifiers();
                        if m.control || m.shift { return this.mark_tab(id, m.shift, cx); }
                        this.marked.clear();
                        if let Some(index) = this.tabs.index_of(id) { this.select_tab(index, window, cx); }
                    }))).flex_1().min_w_0())
                .child(marked_el(Button::new(("browser-tab-close", id as usize)).icon(IconName::Close).ghost().xsmall().size(px(32.))
                    .tooltip(ados_core::shortcuts::tooltip("Close tab", "browser.close_tab", cx))
                    .on_click(cx.listener(move |this, _, window, cx| this.close_tab(Some(id), window, cx)))).flex_shrink_0())
                .context_menu(move |menu, window, cx| tile_view::menu_guard(Self::tile_menu(menu, id, tiled, following, menu_weak.clone()), window, cx));
            run.push(el.into_any_element());
        }
        self.flush_group(&mut items, &mut run, run_group, cx);
        // A newly shown tab is scrolled into view once; the owner can scroll away after.
        // A tab added this frame has no bounds yet: ask again on the next frame.
        if self.strip_shown.get() != Some(current_id) {
            let count = self.strip_scroll.children_count();
            if count == items.len() { self.strip_shown.set(Some(current_id)); } else { cx.notify(); }
            self.strip_scroll.scroll_to_item(selected_item);
        }
        div().id("browser-tabs").flex().items_center().gap_1().px_1().h(px(36.)).min_w_0().flex_shrink_0().overflow_hidden()
            .bg(cx.theme().secondary).border_b_1().border_color(cx.theme().border)
            // Tabs shrink to their minimum width (close button kept), then the list scrolls;
            // the plus button stays outside it.
            .child(div().id("browser-tab-list").flex().items_center().gap_1().h_full().min_w_0().flex_shrink_1()
                .overflow_x_scroll().track_scroll(&self.strip_scroll).children(items))
            .child(marked_el(Button::new("browser-tab-new").icon(IconName::Plus).ghost().size(px(32.))
                .tooltip(ados_core::shortcuts::tooltip("New tab", "browser.new_tab", cx))
                .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx)))).flex_shrink_0())
    }
    /// Tabs of one tile group read as one block: a hairline box with its arrangement button.
    /// (Only gpui-kit's default icons are embedded in the shell, hence panel-left for columns.)
    fn flush_group(&self, items: &mut Vec<AnyElement>, run: &mut Vec<AnyElement>, group: Option<usize>, cx: &mut Context<Self>) {
        if run.is_empty() { return; }
        let Some(g) = group.and_then(|g| self.tiles.groups.get(g)) else { items.append(run); return };
        let layout = g.layout;
        let first = g.tabs[0];
        let (icon, next) = match layout {
            tiles::Layout::Columns => (gpui_kit::assets::IconName::PanelLeft, tiles::Layout::Rows),
            tiles::Layout::Rows => (gpui_kit::assets::IconName::PanelBottom, tiles::Layout::Grid),
            tiles::Layout::Grid => (gpui_kit::assets::IconName::LayoutDashboard, tiles::Layout::Columns),
        };
        let tip = format!("Tiled {}; click for {}", layout.label(), next.label());
        items.push(div().flex().items_center().h(px(36.)).flex_shrink_1().border_1().rounded(px(8.)).border_color(cx.theme().border)
            .child(marked_el(Button::new(("browser-tile-layout", first as usize)).icon(icon).ghost().size(px(32.)).flex_shrink_0()
                .tooltip(tip)
                .on_click(cx.listener(move |this, _, window, cx| { if this.tiles.arrange(first, next) { this.apply_tiles(window, cx); } }))))
            .children(std::mem::take(run))
            .into_any_element());
    }
}

/// What a tab shows: its title, else its address without the scheme, else "New tab".
fn strip_label(tab: &tabs::PageTab) -> SharedString {
    let title = tab.title.split_whitespace().collect::<Vec<_>>().join(" ");
    if !title.is_empty() && title != "about:blank" { return title.into(); }
    if focus::is_blank(&tab.url) { return "New tab".into(); }
    tab.url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string().into()
}
