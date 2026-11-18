//! Tiled page tabs in the Browser: the chords and palette entries, the tab menu items, the
//! dividers between tiles, and saving. The model is `tiles.rs`; the engine side `driver.rs`.
use super::*;
use crate::tiles::{Area, Axis, Layout};
use gpui_kit::component::{menu::{PopupMenu, PopupMenuItem}, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;

actions!(ados_browser, [TileColumns, TileRows, TileGrid, TileNext, UntileTab, UntileGroup, NextTile, PreviousTile, ToggleFollow, TileGrow, TileShrink]);

/// Chord id, palette title, keywords; the palette runs the same action as the chord.
const ENTRIES: &[(&str, &str, &str)] = &[
    ("browser.tile_columns", "Tile tabs side by side", "browser tile split vertical columns"),
    ("browser.tile_rows", "Tile tabs stacked", "browser tile split horizontal rows"),
    ("browser.tile_grid", "Tile tabs in a grid", "browser tile grid"),
    ("browser.tile_next", "Tile with the next tab", "browser tile split next"),
    ("browser.untile_tab", "Untile this tab", "browser tile untile remove"),
    ("browser.untile_group", "Untile all tabs in the group", "browser tile untile group"),
    ("browser.next_tile", "Next tile", "browser tile cycle focus"),
    ("browser.previous_tile", "Previous tile", "browser tile cycle focus"),
    ("browser.follow", "Follower tab: follow the other tile, or stop", "browser tile follower follow links"),
    ("browser.tile_grow", "Make this tile bigger", "browser tile resize grow"),
    ("browser.tile_shrink", "Make this tile smaller", "browser tile resize shrink"),
];

pub(crate) fn init(cx: &mut App) {
    ados_core::shortcuts::bind(cx, "browser.tile_columns", TileColumns);
    ados_core::shortcuts::bind(cx, "browser.tile_rows", TileRows);
    ados_core::shortcuts::bind(cx, "browser.tile_grid", TileGrid);
    ados_core::shortcuts::bind(cx, "browser.tile_next", TileNext);
    ados_core::shortcuts::bind(cx, "browser.untile_tab", UntileTab);
    ados_core::shortcuts::bind(cx, "browser.untile_group", UntileGroup);
    ados_core::shortcuts::bind(cx, "browser.next_tile", NextTile);
    ados_core::shortcuts::bind(cx, "browser.previous_tile", PreviousTile);
    ados_core::shortcuts::bind(cx, "browser.follow", ToggleFollow);
    ados_core::shortcuts::bind(cx, "browser.tile_grow", TileGrow);
    ados_core::shortcuts::bind(cx, "browser.tile_shrink", TileShrink);
    for (id, title, keywords) in ENTRIES {
        let id: &'static str = id;
        ados_core::actions::register(id, title, keywords, None, cx, move |window, cx| {
            let panel = cx.global::<Target>().0.as_ref().and_then(WeakEntity::upgrade);
            match panel {
                Some(panel) if panel.read(cx).active => panel.update(cx, |panel, cx| panel.tile_action(id, window, cx)),
                _ => window.push_notification("Open the Browser module first", cx),
            }
        });
    }
}

impl BrowserPanel {
    /// Wire the tile actions on the panel root (`view.rs`).
    pub(crate) fn tile_actions(&self, el: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        el
            .on_action(cx.listener(|this, _: &TileColumns, window, cx| this.tile_action("browser.tile_columns", window, cx)))
            .on_action(cx.listener(|this, _: &TileRows, window, cx| this.tile_action("browser.tile_rows", window, cx)))
            .on_action(cx.listener(|this, _: &TileGrid, window, cx| this.tile_action("browser.tile_grid", window, cx)))
            .on_action(cx.listener(|this, _: &TileNext, window, cx| this.tile_action("browser.tile_next", window, cx)))
            .on_action(cx.listener(|this, _: &UntileTab, window, cx| this.tile_action("browser.untile_tab", window, cx)))
            .on_action(cx.listener(|this, _: &UntileGroup, window, cx| this.tile_action("browser.untile_group", window, cx)))
            .on_action(cx.listener(|this, _: &NextTile, window, cx| this.tile_action("browser.next_tile", window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTile, window, cx| this.tile_action("browser.previous_tile", window, cx)))
            .on_action(cx.listener(|this, _: &ToggleFollow, window, cx| this.tile_action("browser.follow", window, cx)))
            .on_action(cx.listener(|this, _: &TileGrow, window, cx| this.tile_action("browser.tile_grow", window, cx)))
            .on_action(cx.listener(|this, _: &TileShrink, window, cx| this.tile_action("browser.tile_shrink", window, cx)))
    }
    pub(crate) fn tile_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.tabs.current().id;
        match id {
            "browser.tile_columns" => self.tile_selection(Layout::Columns, window, cx),
            "browser.tile_rows" => self.tile_selection(Layout::Rows, window, cx),
            "browser.tile_grid" => self.tile_selection(Layout::Grid, window, cx),
            "browser.tile_next" => self.tile_next(current, window, cx),
            "browser.untile_tab" => self.untile(current, false, window, cx),
            "browser.untile_group" => self.untile(current, true, window, cx),
            "browser.next_tile" => self.cycle_tile(1, window, cx),
            "browser.previous_tile" => self.cycle_tile(-1, window, cx),
            "browser.follow" => self.toggle_follow(current, window, cx),
            "browser.tile_grow" => self.grow_tile(tiles::STEP, window, cx),
            "browser.tile_shrink" => self.grow_tile(-tiles::STEP, window, cx),
            _ => {}
        }
    }

    /// Tile the tabs marked with Ctrl+click or Shift+click, with the shown one. With nothing
    /// marked, a tiled tab's group takes the new arrangement.
    pub(crate) fn tile_selection(&mut self, layout: Layout, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.tabs.current().id;
        let mut ids: Vec<u64> = self.tabs.list.iter().map(|t| t.id).filter(|id| *id == current || self.marked.contains(id)).collect();
        if ids.len() < 2 && self.tiles.arrange(current, layout) {
            return self.apply_tiles(window, cx);
        }
        ids.dedup();
        match self.tiles.tile(ids.clone(), layout) {
            Ok(()) => {
                self.tabs.gather(&ids);
                self.keep_groups_together();
                self.marked.clear();
                self.apply_tiles(window, cx);
            }
            Err(message) => window.push_notification(message, cx),
        }
    }
    /// Tile tab `id` with the first tab after its group in the strip.
    pub(crate) fn tile_next(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let group = self.tiles.shown(id);
        let last = group.iter().filter_map(|t| self.tabs.index_of(*t)).max().unwrap_or(0);
        let Some(next) = self.tabs.list.get(last + 1).map(|t| t.id) else {
            return window.push_notification("There is no tab after this one to tile with.", cx);
        };
        match self.tiles.tile_with(id, next) {
            Ok(()) => {
                let ids = self.tiles.shown(id);
                self.tabs.gather(&ids);
                self.apply_tiles(window, cx);
            }
            Err(message) => window.push_notification(message, cx),
        }
    }
    pub(crate) fn untile(&mut self, id: u64, whole: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.tiles.group_of(id).is_none() { return window.push_notification("This tab is not tiled.", cx); }
        if whole { self.tiles.untile_group(id) } else { self.tiles.untile(id) }
        // An untiled tab steps out of the group's block, after it.
        self.keep_groups_together();
        self.apply_tiles(window, cx);
    }
    pub(crate) fn toggle_follow(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        match self.tiles.toggle_follow(id) {
            Ok(_) => self.apply_tiles(window, cx),
            Err(message) => window.push_notification(message, cx),
        }
    }
    /// The keyboard to the next or previous tile of the shown group.
    pub(crate) fn cycle_tile(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.tabs.current().id;
        let shown = self.tiles.shown(current);
        if shown.len() < 2 { return; }
        let i = shown.iter().position(|t| *t == current).unwrap_or(0) as i32;
        let next = shown[(i + delta).rem_euclid(shown.len() as i32) as usize];
        if let Some(index) = self.tabs.index_of(next) {
            self.select_tab(index, window, cx);
            self.focus_page(window, cx);
        }
    }
    fn grow_tile(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.tabs.current().id;
        let Some(group) = self.tiles.group_mut(current) else { return window.push_notification("This tab is not tiled.", cx) };
        group.grow(current, delta);
        self.persist_tabs();
        cx.notify();
    }
    /// The tiles changed: save, tell the engine, show the selected tab with its group.
    pub(crate) fn apply_tiles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.send(CommandMsg::Follow(self.tiles.follows.clone()));
        let index = self.tabs.selected;
        self.select_tab(index, window, cx);
        cx.notify();
    }
    /// Before `Select`: the tabs shown with the selected one.
    pub(crate) fn send_tiles(&self) {
        let shown = self.tiles.shown(self.tabs.current().id);
        let tiles = if shown.len() > 1 {
            shown.iter().filter_map(|id| self.tabs.list.iter().find(|t| t.id == *id)).map(|t| (t.id, t.url.clone())).collect()
        } else {
            vec![]
        };
        self.send(CommandMsg::Tiles(tiles));
    }
    /// A tab opened beside a tiled one goes after its group, so a group stays one block.
    pub(crate) fn keep_groups_together(&mut self) {
        for group in self.tiles.groups.clone() { self.tabs.gather(&group.tabs); }
    }
    /// A tab went away: its tile and follower links go with it.
    pub(crate) fn tile_closed(&mut self, id: u64) {
        self.marked.retain(|m| *m != id);
        let follows = self.tiles.follows.clone();
        self.tiles.closed(id);
        if self.tiles.follows != follows { self.send(CommandMsg::Follow(self.tiles.follows.clone())); }
    }

    /// Ctrl+click marks a tab for tiling, Shift+click marks the run from the shown tab.
    pub(crate) fn mark_tab(&mut self, id: u64, range: bool, cx: &mut Context<Self>) {
        if range {
            let (Some(a), Some(b)) = (Some(self.tabs.selected), self.tabs.index_of(id)) else { return };
            let (from, to) = (a.min(b), a.max(b));
            self.marked = self.tabs.list[from..=to].iter().map(|t| t.id).collect();
        } else if let Some(i) = self.marked.iter().position(|m| *m == id) {
            self.marked.remove(i);
        } else {
            self.marked.push(id);
        }
        cx.notify();
    }

    /// The tab menu's tiling entries for tab `id`.
    pub(crate) fn tile_menu(menu: PopupMenu, id: u64, tiled: bool, following: bool, weak: WeakEntity<Self>) -> PopupMenu {
        let run = move |f: fn(&mut Self, u64, &mut Window, &mut Context<Self>)| {
            let weak = weak.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| { let _ = weak.update(cx, |this, cx| f(this, id, window, cx)); }
        };
        menu.item(PopupMenuItem::new("Tile selected tabs side by side").on_click(run(|t, id, w, cx| t.tile_from_menu(id, Layout::Columns, w, cx))))
            .item(PopupMenuItem::new("Tile selected tabs stacked").on_click(run(|t, id, w, cx| t.tile_from_menu(id, Layout::Rows, w, cx))))
            .item(PopupMenuItem::new("Tile selected tabs in a grid").on_click(run(|t, id, w, cx| t.tile_from_menu(id, Layout::Grid, w, cx))))
            .item(PopupMenuItem::new("Tile with the next tab").on_click(run(|t, id, w, cx| t.tile_next(id, w, cx))))
            .when(tiled, |menu| menu.separator()
                .item(PopupMenuItem::new(if following { "Stop following" } else { "Follow the other tile" }).on_click(run(|t, id, w, cx| t.toggle_follow(id, w, cx))))
                .item(PopupMenuItem::new("Untile tab").on_click(run(|t, id, w, cx| t.untile(id, false, w, cx))))
                .item(PopupMenuItem::new("Untile group").on_click(run(|t, id, w, cx| t.untile(id, true, w, cx)))))
    }
    /// From the menu of tab `id`: that tab counts as marked.
    fn tile_from_menu(&mut self, id: u64, layout: Layout, window: &mut Window, cx: &mut Context<Self>) {
        if id != self.tabs.current().id && !self.marked.contains(&id) { self.marked.push(id); }
        self.tile_selection(layout, window, cx);
    }

    /// The tiles of the shown group in window coordinates; one entry for the shown page alone.
    pub(crate) fn tile_bounds(&self, area: Bounds<Pixels>) -> Vec<(u64, Bounds<Pixels>)> {
        let current = self.tabs.current().id;
        match self.tiles.group(current) {
            Some(group) => group.rects(to_area(area)).into_iter().map(|(id, a)| (id, from_area(a))).collect(),
            None => vec![(0, area)],
        }
    }

    /// The draggable, keyboard-adjustable gaps between tiles (`border` hairline; blue while
    /// focused or dragged). Positioned in the page box, whose 2 px border is outside `area`.
    pub(crate) fn dividers(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (Some(area), Some(group)) = (self.area, self.tiles.group(self.tabs.current().id)) else { return vec![] };
        let origin = area.origin;
        group.dividers(to_area(area)).into_iter().enumerate().map(|(n, d)| {
            let focus = self.divider_focus[n.min(self.divider_focus.len() - 1)].clone();
            let active = self.divider_drag == Some((d.axis, d.index));
            let vertical = d.axis == Axis::Vertical;
            let (axis, index) = (d.axis, d.index);
            let line = div().absolute().bg(if active { cx.theme().primary } else { cx.theme().border })
                .map(|el| if vertical { el.top_0().bottom_0().left(px(tiles::GAP / 2. - 0.5)).w(px(1.)) } else { el.left_0().right_0().top(px(tiles::GAP / 2. - 0.5)).h(px(1.)) });
            div().id(("browser-tile-divider", n)).absolute()
                .left(px(d.rect.x - f32::from(origin.x) + 2.)).top(px(d.rect.y - f32::from(origin.y) + 2.))
                .w(px(d.rect.w)).h(px(d.rect.h))
                .cursor(if vertical { CursorStyle::ResizeLeftRight } else { CursorStyle::ResizeUpDown })
                .track_focus(&focus).tab_index(4)
                .focus(|el| el.bg(cx.theme().primary.opacity(0.4)))
                .child(line)
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new("Drag to resize, or focus and use the arrow keys").build(window, cx))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                    this.divider_drag = Some((axis, index));
                    this.divider_focus[n.min(this.divider_focus.len() - 1)].focus(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                    let delta = match (event.keystroke.key.as_str(), vertical) {
                        ("left", true) | ("up", false) => -tiles::STEP,
                        ("right", true) | ("down", false) => tiles::STEP,
                        _ => return,
                    };
                    let current = this.tabs.current().id;
                    if let Some(group) = this.tiles.group_mut(current) { group.nudge(axis, index, delta); }
                    this.persist_tabs();
                    cx.stop_propagation();
                    cx.notify();
                }))
                .into_any_element()
        }).collect()
    }
    /// The pointer moved while a divider is held (the press grabbed the pointer for aDOs).
    pub(crate) fn drag_divider(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (Some((axis, index)), Some(area)) = (self.divider_drag, self.area) else { return };
        let current = self.tabs.current().id;
        let Some(group) = self.tiles.group_mut(current) else { return };
        let pos = if axis == Axis::Vertical { f32::from(position.x) } else { f32::from(position.y) };
        group.drag(axis, index, pos, to_area(area));
        cx.notify();
    }
    pub(crate) fn end_divider_drag(&mut self, cx: &mut Context<Self>) {
        if self.divider_drag.take().is_some() {
            self.persist_tabs();
            cx.notify();
        }
    }

    /// Saved tiles and followers, by tab index.
    pub(crate) fn saved_tiles(&self) -> (Vec<state::SavedGroup>, Vec<(usize, usize)>) {
        let index = |id: &u64| self.tabs.index_of(*id);
        let groups = self.tiles.groups.iter().map(|g| state::SavedGroup {
            tabs: g.tabs.iter().filter_map(index).collect(),
            layout: g.layout,
            sizes: g.sizes.clone(),
            cross: g.cross.clone(),
        }).collect();
        let follows = self.tiles.follows.iter().filter_map(|(f, l)| Some((index(f)?, index(l)?))).collect();
        (groups, follows)
    }
}

/// Tiles and followers saved by index, back to tab ids.
pub(crate) fn restore_tiles(state: &State, tabs: &tabs::Tabs) -> tiles::Tiles {
    let id = |i: &usize| tabs.list.get(*i).map(|t| t.id);
    let mut out = tiles::Tiles::default();
    for saved in &state.tile_groups {
        let ids: Vec<u64> = saved.tabs.iter().filter_map(id).collect();
        if ids.len() != saved.tabs.len() || out.tile(ids.clone(), saved.layout).is_err() { continue; }
        if let Some(group) = out.group_mut(ids[0]) {
            *group = group.clone().with_sizes(saved.sizes.clone(), saved.cross.clone());
        }
    }
    for (f, l) in &state.follows {
        if let (Some(f), Some(l)) = (id(f), id(l)) {
            if out.group_of(f).is_some() && out.group_of(f) == out.group_of(l) && out.leader_of(l).is_none() && out.follower_of(l).is_none() {
                out.follows.push((f, l));
            }
        }
    }
    out
}

/// The page windows draw above GPUI: while the tab menu is open they hide, as for the
/// dock's own menus (same rule as the shell's `dock_skin::guard`).
pub(crate) fn menu_guard(menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    OverlayGuard::open(cx);
    // The tab's tooltip must not stay drawn over the menu: end the hover that showed it.
    // Deferred: it dispatches a mouse move while the menu is being built.
    window.defer(cx, |window, cx| ados_core::overlay::clear_pointer(window, cx));
    let open = std::rc::Rc::new(std::cell::Cell::new(true));
    let dismissed = open.clone();
    cx.subscribe(&cx.entity(), move |_, _, _: &DismissEvent, cx| {
        if dismissed.replace(false) { OverlayGuard::close(cx); }
    })
    .detach();
    cx.on_release(move |_, cx| {
        if open.replace(false) { OverlayGuard::close(cx); }
    })
    .detach();
    menu
}

fn to_area(b: Bounds<Pixels>) -> Area {
    Area { x: f32::from(b.origin.x), y: f32::from(b.origin.y), w: f32::from(b.size.width), h: f32::from(b.size.height) }
}
fn from_area(a: Area) -> Bounds<Pixels> {
    Bounds { origin: point(px(a.x), px(a.y)), size: size(px(a.w), px(a.h)) }
}
