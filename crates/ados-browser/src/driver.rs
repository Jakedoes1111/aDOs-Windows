//! The engine's tabs and windows behind aDOs's page tabs (`tabs.rs`).
//!
//! Each aDOs tab is one engine tab: a WebDriver window handle (Servo) or a CDP page target
//! (Chromium). Several engine tabs can share one native window (Servo's `PainterId`, Chromium's
//! `windowId`); a page's `window.open` or `target=_blank` gets a native window of its own. Every
//! native window is taken into aDOs (`Embed::adopt`) and only the selected tab's is shown, so a
//! popup is never left floating or hidden behind aDOs.
use crate::{
    protocol::{Cdp, WebDriver},
    runtime::{CommandMsg, Event, HostMsg, Sender},
    xembed::{Embed, Tile},
};
use anyhow::{Context, Result};
use x11rb::connection::Connection as _;
use std::time::{Duration, Instant};

/// Ids for tabs a page opened. aDOs's own tabs are numbered below this (`tabs.rs`).
pub const PAGE_OPENED: u64 = 1 << 40;
/// How long a new engine window may take to appear before aDOs says it could not take it in.
const ADOPT_WAIT: Duration = Duration::from_secs(3);

pub(crate) struct Tab {
    pub(crate) id: u64,
    /// WebDriver handle or CDP target id.
    pub(crate) handle: String,
    /// The native window it lives in, in the engine's own terms.
    group: String,
}
struct Window {
    group: String,
    window: u32,
    clip_top: u32,
}

pub enum Kind {
    Cdp { pages: Vec<Cdp>, port: u16, agent: ureq::Agent, hints: String },
    Servo {
        wd: WebDriver,
        next_snapshot: Option<Instant>,
        /// Settings call to repeat on each page after a runtime keymap or colour change.
        hints: Option<String>,
    },
}

pub struct Driver {
    pub kind: Kind,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) selected: Option<u64>,
    windows: Vec<Window>,
    /// Native windows that never hold a page (Servo's launcher window).
    ignored_windows: Vec<u32>,
    finder: Embed,
    pid: u32,
    /// Chromium's window class marker; Servo windows are found by process id.
    marker: String,
    next_id: u64,
    /// What the host shows now: one page, or every tile of the selected tab's group.
    pub(crate) shown: Vec<Tile>,
    shown_active: u64,
    /// The tabs tiled with the selected one (with their addresses), from the panel; empty or
    /// without the selected tab when it is shown alone.
    pub(crate) tiles: Vec<(u64, String)>,
    /// (follower, leader) links: plain link clicks in the leader open in the follower.
    pub(crate) follows: Vec<(u64, u64)>,
    /// When the leader's queue of followed links is read next.
    pub(crate) next_follow: Option<Instant>,
    /// Chromium: the pending read of the leader's queue.
    pub(crate) follow_reply: Option<async_channel::Receiver<Result<String, String>>>,
    /// Servo: tabs not driven now whose page is read at this time (a follower that navigated).
    pub(crate) reads: Vec<(u64, Instant)>,
    /// Since when a tab's native window has not been found; `None` when all are.
    unpaired_since: Option<Instant>,
    unpaired_reported: bool,
    servo_clip: u32,
}

/// Servo handles read `PainterId: N, TopLevelBrowsingContext(..)`: tabs of one painter share a window.
fn painter(handle: &str) -> String {
    handle.split(',').next().filter(|p| p.starts_with("PainterId")).unwrap_or(handle).to_string()
}
/// The painter number, when the handle has one.
fn painter_number(handle: &str) -> Option<u64> {
    painter(handle).strip_prefix("PainterId: ")?.trim().parse().ok()
}
/// servoshell's launcher window (the command-line page) is created before the WebDriver session's
/// window, so its painter is older. Its handle can appear after the session starts: never a tab.
fn launcher(handle: &str, home: &str) -> bool {
    matches!((painter_number(handle), painter_number(home)), (Some(h), Some(home)) if h < home)
}

impl Driver {
    /// `window` is the native window already shown (startup), holding tab `first`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(mut kind: Kind, first: u64, window: u32, clip_top: u32, finder: Embed, pid: u32, marker: String) -> Result<Self> {
        let (handle, group) = match &mut kind {
            Kind::Servo { wd, .. } => {
                // Any engine tab from before the session (servoshell's launcher page) is not ours.
                let home = wd.home.clone();
                for handle in wd.handles()?.into_iter().filter(|h| *h != home) {
                    if wd.switch(&handle).is_ok() { let _ = wd.close_current(); }
                }
                wd.switch(&home)?;
                (home.clone(), painter(&home))
            }
            Kind::Cdp { pages, .. } => {
                let page = pages.first_mut().context("Chromium has no page")?;
                (page.target.clone(), page.window_id().unwrap_or_default())
            }
        };
        let ignored_windows = finder.page_windows(pid, &marker)?.into_iter().filter(|w| *w != window).collect();
        Ok(Self {
            kind,
            tabs: vec![Tab { id: first, handle, group: group.clone() }],
            selected: Some(first),
            windows: vec![Window { group, window, clip_top }],
            ignored_windows,
            finder,
            pid,
            marker,
            next_id: PAGE_OPENED,
            shown: vec![Tile { tab: first, window, clip_top }],
            shown_active: first,
            tiles: vec![],
            follows: vec![],
            next_follow: None,
            follow_reply: None,
            reads: vec![],
            unpaired_since: None,
            unpaired_reported: false,
            servo_clip: clip_top,
        })
    }

    pub fn command(&mut self, msg: CommandMsg, events: &async_channel::Sender<Event>, host: &Sender<HostMsg>) -> Result<()> {
        match msg {
            CommandMsg::Select(id, url) => self.select(id, &url, events, host),
            CommandMsg::NewTab(id, url) => self.open(id, &url, events, host),
            CommandMsg::Close(id) => self.close(id),
            CommandMsg::Tiles(tiles) => { self.tiles = tiles; Ok(()) }
            CommandMsg::Follow(follows) => self.set_follows(follows),
            msg => self.page_command(msg, events, host),
        }
    }

    pub(crate) fn handle_of(&self, id: u64) -> Option<String> {
        self.tabs.iter().find(|t| t.id == id).map(|t| t.handle.clone())
    }

    /// Show tab `id`, with the tiles of its group; a restored tab not yet in the engine is
    /// opened at `url` first.
    fn select(&mut self, id: u64, url: &str, events: &async_channel::Sender<Event>, host: &Sender<HostMsg>) -> Result<()> {
        if self.handle_of(id).is_none() { self.open_engine_tab(id, url, self.is_tiled(id))?; }
        self.selected = Some(id);
        self.separate_tiles()?;
        let handle = self.handle_of(id).context("the tab has no engine tab")?;
        // Chromium can open a tab in another tile's window (a Ctrl+click goes to the last
        // active window): each shown tile's own page is brought to the front of its window.
        let others: Vec<String> = self.shown_ids().into_iter().filter(|t| *t != id).filter_map(|t| self.handle_of(t)).collect();
        match &mut self.kind {
            Kind::Servo { wd, next_snapshot, .. } => {
                wd.switch(&handle)?;
                *next_snapshot = None;
            }
            Kind::Cdp { pages, .. } => {
                for page in pages.iter_mut().filter(|p| others.contains(&p.target)) { page.activate()?; }
                if let Some(page) = pages.iter_mut().find(|p| p.target == handle) {
                    page.activate()?;
                    page.snapshot()?;
                }
            }
        }
        self.next_follow = Some(Instant::now());
        if let Kind::Cdp { .. } = self.kind {
            for (_, leader) in self.follows.clone() { self.eval_in(leader, crate::follow::FOLLOW_INSTALL); }
        }
        // Servo reads only the page it drives: the other tiles' titles are read once they load
        // (a leader's link holder is installed then too).
        if matches!(self.kind, Kind::Servo { .. }) {
            let later = Instant::now() + Duration::from_millis(1500);
            let others: Vec<u64> = if self.is_tiled(id) { self.tiles.iter().map(|(t, _)| *t).filter(|t| *t != id).collect() } else { vec![] };
            for tile in others {
                if !self.reads.iter().any(|(r, _)| *r == tile) { self.reads.push((tile, later)); }
            }
        }
        self.pair_windows(events, host)?;
        self.show(host);
        if matches!(self.kind, Kind::Servo { .. }) { self.servo_snapshot(events)?; }
        Ok(())
    }

    /// A new engine tab for aDOs tab `id`, selected, at `url`.
    fn open(&mut self, id: u64, url: &str, events: &async_channel::Sender<Event>, host: &Sender<HostMsg>) -> Result<()> {
        self.open_engine_tab(id, url, false)?;
        self.selected = Some(id);
        self.pair_windows(events, host)?;
        self.show(host);
        if matches!(self.kind, Kind::Servo { .. }) { self.servo_snapshot(events)?; }
        Ok(())
    }

    /// An engine tab for aDOs tab `id` at `url`; WebDriver is left driving it. `own_window`:
    /// in a native window of its own (a tile), not beside the other tabs.
    fn open_engine_tab(&mut self, id: u64, url: &str, own_window: bool) -> Result<()> {
        let (handle, group) = match &mut self.kind {
            Kind::Servo { wd, next_snapshot, .. } => {
                let handle = if own_window { wd.new_window()? } else { wd.new_tab()? };
                wd.switch(&handle)?;
                // A new engine tab is already blank; Servo answers an explicit about:blank with a 404 page.
                if !crate::focus::is_blank(url) { wd.navigate(url)?; }
                *next_snapshot = None;
                (handle.clone(), painter(&handle))
            }
            // Chromium tabs aDOs opens always get a window of their own.
            Kind::Cdp { pages, port, agent, hints } => {
                let target = match pages.first_mut() {
                    Some(page) => page.create(url)?,
                    None => {
                        let v: serde_json::Value = agent.put(format!("http://127.0.0.1:{port}/json/new?{url}")).send_empty()?.body_mut().read_json()?;
                        v["id"].as_str().context("Chromium returned no new page")?.to_string()
                    }
                };
                let mut page = Cdp::connect(*port, agent, Some(&target))?;
                page.add_script(hints)?;
                let group = page.window_id().unwrap_or_default();
                pages.push(page);
                (target, group)
            }
        };
        self.tabs.push(Tab { id, handle, group });
        Ok(())
    }

    fn is_tiled(&self, id: u64) -> bool { self.tiles.len() > 1 && self.tiles.iter().any(|(t, _)| *t == id) }

    /// One native window shows one page, so every tile needs a window of its own. A tile that
    /// shares its window with an earlier tile is opened again, at its address, in a new
    /// window (its back/forward history stays behind). Then each window shows its tile.
    fn separate_tiles(&mut self) -> Result<()> {
        let Some(active) = self.selected else { return Ok(()) };
        if !self.is_tiled(active) { return Ok(()); }
        let group_of = |this: &Self, id: u64| this.tabs.iter().find(|t| t.id == id).map(|t| t.group.clone()).unwrap_or_default();
        let mut used: Vec<String> = vec![];
        for (id, url) in self.tiles.clone() {
            if self.handle_of(id).is_none() { self.open_engine_tab(id, &url, true)?; }
            if used.contains(&group_of(self, id)) {
                let old = self.handle_of(id).unwrap_or_default();
                self.tabs.retain(|t| t.id != id);
                self.open_engine_tab(id, &url, true)?;
                match &mut self.kind {
                    Kind::Servo { wd, .. } => {
                        wd.switch(&old)?;
                        wd.close_current()?;
                    }
                    Kind::Cdp { pages, .. } => {
                        if let Some(i) = pages.iter().position(|p| p.target == old) {
                            pages[i].close_target()?;
                            pages.remove(i);
                        }
                    }
                }
                eprintln!("Browser gave tab {id} a window of its own for tiling");
            }
            used.push(group_of(self, id));
        }
        if let Kind::Servo { wd, .. } = &self.kind {
            for (id, _) in &self.tiles {
                let handle = self.tabs.iter().find(|t| t.id == *id).map(|t| &t.handle);
                if let Some(handle) = handle.filter(|_| *id != active) { wd.switch(handle)?; }
            }
        }
        self.forget_empty_windows();
        Ok(())
    }

    fn close(&mut self, id: u64) -> Result<()> {
        let Some(handle) = self.handle_of(id) else { return Ok(()) };
        self.tabs.retain(|t| t.id != id);
        if self.selected == Some(id) { self.selected = None; }
        let back = self.selected.and_then(|s| self.handle_of(s));
        match &mut self.kind {
            Kind::Servo { wd, .. } => {
                wd.switch(&handle)?;
                wd.close_current()?;
                // WebDriver has no current tab now: act on the selected one again.
                if let Some(back) = back { wd.switch(&back)?; }
            }
            Kind::Cdp { pages, .. } => {
                if let Some(i) = pages.iter().position(|p| p.target == handle) {
                    pages[i].close_target()?;
                    pages.remove(i);
                }
            }
        }
        self.forget_empty_windows();
        Ok(())
    }

    /// Native windows with no tab left are the engine's to destroy; aDOs stops tracking them.
    fn forget_empty_windows(&mut self) {
        let tabs = &self.tabs;
        self.windows.retain(|w| tabs.iter().any(|t| t.group == w.group));
        let windows = &self.windows;
        self.shown.retain(|s| windows.iter().any(|w| w.window == s.window));
    }

    /// Compare the engine's tabs with ours: a tab a page opened is adopted (`Event::Opened`), a
    /// tab that closed itself is reported (`Event::Closed`).
    pub fn sync(&mut self, events: &async_channel::Sender<Event>, host: &Sender<HostMsg>) -> Result<()> {
        let handles: Vec<(String, String)> = match &self.kind {
            Kind::Servo { wd, .. } => wd.handles()?.into_iter().map(|h| (h, String::new())).collect(),
            Kind::Cdp { port, agent, .. } => Cdp::targets(*port, agent)?,
        };
        let gone: Vec<u64> = self.tabs.iter().filter(|t| !handles.iter().any(|(h, _)| *h == t.handle)).map(|t| t.id).collect();
        for id in gone {
            self.tabs.retain(|t| t.id != id);
            if let Kind::Cdp { pages, .. } = &mut self.kind {
                let tabs = &self.tabs;
                pages.retain(|p| tabs.iter().any(|t| t.handle == p.target));
            }
            if self.selected == Some(id) {
                self.selected = None;
                if let Kind::Servo { wd, .. } = &self.kind {
                    if let Some(other) = self.tabs.first() { let _ = wd.switch(&other.handle); }
                }
            }
            let _ = events.try_send(Event::Closed(id));
        }
        self.forget_empty_windows();
        let opener = self.selected;
        let selected_handle = opener.and_then(|s| self.handle_of(s));
        let selected_group = self.tabs.iter().find(|t| Some(t.id) == opener).map(|t| t.group.clone());
        for (handle, address) in handles {
            if self.tabs.iter().any(|t| t.handle == handle) { continue; }
            if let Kind::Servo { wd, .. } = &self.kind {
                if launcher(&handle, &wd.home) {
                    if wd.switch(&handle).is_ok() { let _ = wd.close_current(); }
                    if let Some(back) = &selected_handle { wd.switch(back)?; }
                    continue;
                }
            }
            let id = self.next_id;
            self.next_id += 1;
            let (group, url, title) = match &mut self.kind {
                Kind::Servo { wd, .. } => {
                    let group = painter(&handle);
                    // A popup in its own window can be read without changing what is shown.
                    let (url, title) = if Some(&group) != selected_group.as_ref() && wd.switch(&handle).is_ok() {
                        let read = wd.location().unwrap_or_default();
                        if let Some(back) = &selected_handle { wd.switch(back)?; }
                        read
                    } else {
                        Default::default()
                    };
                    (group, url, title)
                }
                Kind::Cdp { pages, port, agent, hints } => {
                    let mut page = Cdp::connect(*port, agent, Some(&handle))?;
                    page.add_script(hints)?;
                    let group = page.window_id().unwrap_or_default();
                    pages.push(page);
                    (group, address, String::new())
                }
            };
            eprintln!("Browser adopted engine tab {handle} as tab {id}");
            self.tabs.push(Tab { id, handle, group });
            let _ = events.try_send(Event::Opened { id, opener, url, title });
        }
        self.pair_windows(events, host)?;
        self.show(host);
        Ok(())
    }

    /// Find the native window of every engine window not yet held, and take it into aDOs.
    fn pair_windows(&mut self, events: &async_channel::Sender<Event>, host: &Sender<HostMsg>) -> Result<()> {
        // Drain the finder's own event queue: its root subscription must not pile up.
        while self.finder.conn.poll_for_event()?.is_some() {}
        let mut groups: Vec<String> = vec![];
        for tab in &self.tabs {
            if !self.windows.iter().any(|w| w.group == tab.group) && !groups.contains(&tab.group) {
                groups.push(tab.group.clone());
            }
        }
        if groups.is_empty() {
            self.unpaired_since = None;
            self.unpaired_reported = false;
            return Ok(());
        }
        let known: Vec<u32> = self.windows.iter().map(|w| w.window).chain(self.ignored_windows.iter().copied()).collect();
        let found: Vec<u32> = self.finder.page_windows(self.pid, &self.marker)?.into_iter().filter(|w| !known.contains(w)).collect();
        if found.len() >= groups.len() {
            // New windows pair with new engine windows in creation order (X ids grow).
            let newest = &found[found.len() - groups.len()..];
            for (group, &window) in groups.into_iter().zip(newest) {
                let clip_top = self.clip_for(&group, window);
                host.send(HostMsg::Adopt(window));
                eprintln!("Browser holds engine window {window:#x} ({group}), chrome crop {clip_top}px");
                self.windows.push(Window { group, window, clip_top });
            }
            self.unpaired_since = None;
            self.unpaired_reported = false;
        } else {
            let since = *self.unpaired_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= ADOPT_WAIT && !self.unpaired_reported {
                self.unpaired_reported = true;
                let _ = events.try_send(Event::Error(
                    "A page opened a window aDOs could not take in. It may be open outside aDOs; close it there.".into(),
                ));
            }
        }
        Ok(())
    }

    /// The engine's own toolbar height in `window`, cropped away like the first window's.
    fn clip_for(&mut self, group: &str, window: u32) -> u32 {
        match &mut self.kind {
            Kind::Servo { .. } => self.servo_clip,
            Kind::Cdp { pages, .. } => {
                let tabs = &self.tabs;
                let page = pages.iter_mut().find(|p| tabs.iter().any(|t| t.group == group && t.handle == p.target));
                page.and_then(|p| p.viewport_height().ok())
                    .and_then(|height| self.finder.chrome_height(window, height).ok())
                    .unwrap_or(0)
            }
        }
    }

    /// While an engine window is still missing, the driver loop checks again this soon.
    pub fn retry_due(&self) -> Option<Duration> {
        self.unpaired_since.filter(|_| !self.unpaired_reported).map(|_| Duration::from_millis(150))
    }

    /// The tabs shown now: the selected tab's tile group, or the selected tab alone.
    pub(crate) fn shown_ids(&self) -> Vec<u64> {
        let Some(active) = self.selected else { return vec![] };
        if self.tiles.iter().any(|(t, _)| *t == active) { self.tiles.iter().map(|(t, _)| *t).collect() } else { vec![active] }
    }

    fn show(&mut self, host: &Sender<HostMsg>) {
        let Some(active) = self.selected else { return };
        let mut tiles: Vec<Tile> = vec![];
        for id in self.shown_ids() {
            let Some(tab) = self.tabs.iter().find(|t| t.id == id) else { continue };
            let Some(window) = self.windows.iter().find(|w| w.group == tab.group) else { continue };
            if tiles.iter().any(|t| t.window == window.window) { continue; }
            tiles.push(Tile { tab: id, window: window.window, clip_top: window.clip_top });
        }
        if !tiles.iter().any(|t| t.tab == active) { return; }
        if self.shown != tiles || self.shown_active != active {
            self.shown_active = active;
            self.shown = tiles.clone();
            host.send(HostMsg::Show(tiles, active));
        }
    }

    fn servo_snapshot(&mut self, events: &async_channel::Sender<Event>) -> Result<()> {
        let Some(id) = self.selected else { return Ok(()) };
        let Kind::Servo { wd, next_snapshot, hints } = &mut self.kind else { return Ok(()) };
        *next_snapshot = None;
        if let Some(call) = hints.as_deref() { let _ = wd.eval(call); }
        if let Ok(page) = wd.snapshot() {
            if page.loading {
                *next_snapshot = Some(Instant::now() + Duration::from_secs(3));
            }
            let _ = events.try_send(Event::Page(id, page));
        }
        Ok(())
    }

    fn page_command(&mut self, msg: CommandMsg, events: &async_channel::Sender<Event>, host: &Sender<HostMsg>) -> Result<()> {
        let selected = self.selected.and_then(|s| self.handle_of(s));
        match &mut self.kind {
            Kind::Cdp { pages, .. } => {
                let page = pages.iter_mut().find(|p| Some(&p.target) == selected.as_ref());
                match (msg, page) {
                    (CommandMsg::Eval(_, reply) | CommandMsg::Screenshot(reply), None) => {
                        let _ = reply.try_send(Err("no page is selected".into()));
                    }
                    (CommandMsg::Eval(script, reply), Some(page)) => page.eval(&script, reply)?,
                    (CommandMsg::Screenshot(reply), Some(page)) => page.screenshot(reply)?,
                    (CommandMsg::Hints(config), _) => {
                        let call = crate::hints::config_call(&config);
                        for page in pages.iter_mut() { page.add_script(&call)?; }
                    }
                    (CommandMsg::Navigate(url), Some(page)) => page.navigate(&url)?,
                    (CommandMsg::Back, Some(page)) => page.history(-1)?,
                    (CommandMsg::Forward, Some(page)) => page.history(1)?,
                    (CommandMsg::Reload, Some(page)) => page.reload()?,
                    (CommandMsg::Snapshot, _) => self.sync(events, host)?,
                    _ => {}
                }
            }
            Kind::Servo { wd, next_snapshot, hints } => {
                match msg {
                    CommandMsg::Eval(script, reply) => {
                        let _ = reply.try_send(wd.eval(&script).map_err(|e| e.to_string()));
                        return Ok(());
                    }
                    CommandMsg::Screenshot(reply) => {
                        let _ = reply.try_send(wd.screenshot().map_err(|e| e.to_string()));
                        return Ok(());
                    }
                    CommandMsg::Hints(config) => {
                        // Servo reads --userscripts once at start: re-apply on every later page.
                        let call = crate::hints::config_call(&config);
                        let _ = wd.eval(&call);
                        *hints = Some(call);
                        return Ok(());
                    }
                    _ => {}
                }
                let throttled = matches!(msg, CommandMsg::Snapshot) && next_snapshot.is_some_and(|next| next > Instant::now());
                if selected.is_some() {
                    match msg {
                        CommandMsg::Navigate(url) => wd.navigate(&url)?,
                        CommandMsg::Back => wd.history(-1)?,
                        CommandMsg::Forward => wd.history(1)?,
                        CommandMsg::Reload => wd.reload()?,
                        _ => {}
                    }
                }
                // A page may have opened or closed a tab: adopt it before reading the page.
                self.sync(events, host)?;
                if !throttled { self.servo_snapshot(events)?; }
            }
        }
        Ok(())
    }

    /// Chromium: page snapshots from every tab (each tagged with its aDOs tab), and target changes.
    pub fn drain_cdp(&mut self, events: &async_channel::Sender<Event>, host: &Sender<HostMsg>) -> Result<()> {
        let Kind::Cdp { pages, .. } = &mut self.kind else { return Ok(()) };
        let mut changed = false;
        for page in pages.iter_mut() {
            let id = self.tabs.iter().find(|t| t.handle == page.target).map(|t| t.id);
            match page.drain() {
                Ok(snapshots) => {
                    for snapshot in snapshots {
                        if let Some(id) = id { let _ = events.try_send(Event::Page(id, snapshot)); }
                    }
                }
                // A closed page's socket ends: the target list decides what happened.
                Err(_) => changed = true,
            }
            let _ = page.loading_tick();
            changed |= std::mem::take(&mut page.targets_changed);
        }
        if changed || self.unpaired_since.is_some() { self.sync(events, host)?; }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn painters() {
        let home = "PainterId: 2, TopLevelBrowsingContext(0,2)";
        assert_eq!(painter(home), "PainterId: 2");
        assert!(launcher("PainterId: 1, TopLevelBrowsingContext(0,1)", home));
        assert!(!launcher("PainterId: 3, TopLevelBrowsingContext(0,3)", home));
        assert!(!launcher("8F2A", "9C1B"));
    }

}
