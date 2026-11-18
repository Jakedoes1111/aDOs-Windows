//! X11 operations stay on the browser thread; no GPUI handles cross threads.
use anyhow::{bail, Context, Result};
use x11rb::{
    connection::Connection,
    protocol::{
        composite::{ConnectionExt as _, Redirect},
        xproto::*,
        Event,
    },
    rust_connection::RustConnection,
    CURRENT_TIME,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// One shown page: a clip window (crops the engine's own toolbar) holding an engine window.
#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    tab: u64,
    clip: u32,
    child: u32,
    clip_top: u32,
    rect: Option<Rect>,
    mapped: bool,
}

/// A page to show: the aDOs tab, its engine window, and the engine toolbar height to crop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub tab: u64,
    pub window: u32,
    pub clip_top: u32,
}

pub struct Embed {
    pub conn: RustConnection,
    pub parent: u32,
    /// The shown page that owns the keyboard (the active tile, or the only page).
    slot: Slot,
    /// The other tiles shown beside it (`show`); empty when one page is shown.
    others: Vec<Slot>,
    /// Clip windows no tile uses now, kept for the next tiling.
    spare: Vec<u32>,
    /// Each tile's rectangle by aDOs tab id; tab 0 is the one page when nothing is tiled.
    rects: Vec<(u64, Rect)>,
    /// A press in a tile that did not own the keyboard made it the active one.
    pressed_tile: Option<u64>,
    focus_proxy: u32,
    focus_requested: bool,
    /// aDOs wants the page to own the keyboard (focus.rs `Owner`). Kept while another app has
    /// X focus, so the page gets it back when aDOs is activated again (F7).
    want_page: bool,
    /// Keys GPUI delivered to the page proxy before the page held X focus (typed straight on
    /// after the chord that asked for it): replayed to the page once it has focus (F4).
    typed: Vec<u8>,
    /// Keys down when the page last took X focus: the engine replays those itself on focus-in.
    down_at_focus: [u8; 32],
    typed_since: Option<std::time::Instant>,
    /// Stop waiting for held keys after this: a stuck or virtual key must not keep the page unfocused.
    focus_deadline: std::time::Instant,
    root: u32,
    pub parent_visible: bool,
    visible: bool,
    shortcuts: Vec<(u8, u16, usize)>,
    /// Engine window chords grabbed and dropped (`engine_keys.rs`): (keycode, modifiers).
    dropped: Vec<(u8, u16)>,
    page_pressed: bool,
    /// The last press in the page was a Ctrl+click or a middle click: a tab it opens is a
    /// background tab (`Event::PagePressed`).
    press_behind: bool,
    /// X time the pointer last entered the page, and a press held until the engine has seen
    /// that hover (G3: servoshell picks the click target from the previous frame's hover).
    entered_at: u32,
    replay: Option<(u32, std::time::Instant)>,
    /// The pointer moved onto the page; GPUI gets no leave event for it.
    pointer_entered: bool,
    /// servoshell gives Tab to the page only once its page widget has egui focus.
    engine_keyboard_ready: bool,
    keyboard_held: bool,
    title_atom: u32,
    /// The floating card's rectangle, cut out of the page (`cutout.rs`).
    hole: crate::cutout::Holes,
    /// No SHAPE extension: hide the page while a card is open instead.
    has_shape: bool,
    /// Every engine window held inside the clip (one per engine window, several tabs can share
    /// one). Only `child` is mapped; the others wait unmapped, never floating on the desktop.
    adopted: Vec<u32>,
    /// Engine windows whose page widget already took the keyboard once (`ready_engine_keyboard`).
    keyboard_ready: Vec<u32>,
}
impl Embed {
    pub fn new(parent: u32) -> Result<Self> {
        let (conn, screen) =
            x11rb::connect(None).context("This browser requires an X11 display")?;
        let root = conn.setup().roots[screen].root;
        let parent_visible =
            conn.get_window_attributes(parent)?.reply()?.map_state == MapState::VIEWABLE;
        conn.change_window_attributes(
            parent,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::FOCUS_CHANGE),
        )?
        .check()?;
        // A new top-level window (the engine's `window.open`, a shortcut that got through) is reported
        // so the runtime can check that it still drives one page.
        conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY))?.check()?;
        let title_atom = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;
        let has_shape = crate::cutout::supported(&conn);
        Ok(Self {
            conn,
            parent,
            slot: Slot::default(),
            others: vec![],
            spare: vec![],
            rects: vec![],
            pressed_tile: None,
            focus_proxy: 0,
            focus_requested: false,
            want_page: false,
            typed: vec![],
            down_at_focus: [0; 32],
            typed_since: None,
            focus_deadline: std::time::Instant::now(),
            root,
            parent_visible,
            visible: false,
            shortcuts: vec![],
            dropped: vec![],
            page_pressed: false,
            press_behind: false,
            entered_at: 0,
            replay: None,
            pointer_entered: false,
            engine_keyboard_ready: false,
            keyboard_held: false,
            title_atom,
            hole: Default::default(),
            has_shape,
            adopted: vec![],
            keyboard_ready: vec![],
        })
    }
    /// Replace the passive grabs with `keys` (canonical aDOs chords). Keys X cannot express are skipped.
    pub fn shortcuts(&mut self, keys: &[String]) -> Result<()> {
        if self.slot.child != 0 {
            let all = self.shortcuts.iter().map(|&(code, mask, _)| (code, mask)).chain(self.dropped.iter().copied());
            for (keycode, modifiers) in all {
                for extra in [ModMask::default(), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2] {
                    self.conn.ungrab_key(keycode, self.slot.child, ModMask::from(modifiers) | extra)?.check()?;
                }
            }
        }
        self.shortcuts.clear();
        self.dropped.clear();
        let setup = self.conn.setup();
        let mapping = self.conn.get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)?.reply()?;
        for (index, key) in keys.iter().enumerate() {
            let Some((modifiers, symbol)) = chord_keysym(key) else { continue };
            if let Some(position) = mapping.keysyms.chunks(usize::from(mapping.keysyms_per_keycode)).position(|syms| syms.contains(&symbol)) {
                self.shortcuts.push((setup.min_keycode + position as u8, u16::from(modifiers), index));
            }
        }
        // aDOs chords win: an engine chord that is also an aDOs chord stays a shortcut.
        for chord in crate::engine_keys::ENGINE_WINDOW_CHORDS {
            let Some((modifiers, symbol)) = chord_keysym(chord) else { continue };
            let Some(position) = mapping.keysyms.chunks(usize::from(mapping.keysyms_per_keycode)).position(|syms| syms.contains(&symbol)) else { continue };
            let entry = (setup.min_keycode + position as u8, u16::from(modifiers));
            if !self.shortcuts.iter().any(|&(code, mask, _)| (code, mask) == entry) && !self.dropped.contains(&entry) {
                self.dropped.push(entry);
            }
        }
        if self.slot.child != 0 { self.grab_shortcuts(None)?; }
        self.conn.flush()?;
        Ok(())
    }
    fn page_has_focus(&self) -> Result<bool> {
        if self.slot.child == 0 { return Ok(false); }
        let mut focus = self.conn.get_input_focus()?.reply()?.focus;
        // X11 uses 0/1 for None/PointerRoot; neither is a real client window.
        if focus <= 1 { return Ok(false); }
        while focus != 0 && focus != self.root && focus != self.parent {
            if focus == self.slot.child { return Ok(true); }
            focus = self.conn.query_tree(focus)?.reply()?.parent;
        }
        Ok(false)
    }
    /// X focus is somewhere in the aDOs window (top-level, focus proxy or page). aDOs never
    /// moves focus between its own windows while another application has it (F7).
    fn app_has_focus(&self) -> Result<bool> {
        let mut focus = self.conn.get_input_focus()?.reply()?.focus;
        // PointerRoot (1): no window manager and no application claimed the keyboard.
        if focus <= 1 { return Ok(focus == 1); }
        while focus != 0 && focus != self.root {
            if focus == self.parent { return Ok(true); }
            focus = self.conn.query_tree(focus)?.reply()?.parent;
        }
        Ok(false)
    }
    /// A press in the page since the last call, and whether it opens links behind (Ctrl or middle button).
    pub fn take_page_press(&mut self) -> Option<bool> { std::mem::take(&mut self.page_pressed).then_some(self.press_behind) }
    pub fn take_pointer_enter(&mut self) -> bool { std::mem::take(&mut self.pointer_entered) }
    /// The page's pixels in `rect` (viewport window pixels), for a comment crop (`shot.rs`).
    pub fn grab(&self, rect: Rect, viewport_h: u32) -> Result<(u32, u32, Vec<u8>)> {
        if self.slot.child == 0 { bail!("the page window is not attached"); }
        crate::shot::grab(&self.conn, self.slot.child, rect, viewport_h, self.slot.clip_top)
    }
    /// When a held press (see `replay`) must be passed on to the page.
    pub fn replay_due(&self) -> Option<std::time::Instant> { self.replay.map(|(_, at)| at) }
    pub fn replay_pointer(&mut self) -> Result<()> {
        if let Some((time, at)) = self.replay {
            if at <= std::time::Instant::now() {
                self.replay = None;
                self.conn.allow_events(Allow::REPLAY_POINTER, time)?.check()?;
                self.conn.flush()?;
            }
        }
        Ok(())
    }
    pub fn shortcut_done(&mut self) -> Result<()> {
        self.keyboard_held = false;
        self.conn.ungrab_keyboard(CURRENT_TIME)?.check()?;
        self.conn.allow_events(Allow::ASYNC_KEYBOARD, CURRENT_TIME)?.check()?;
        self.conn.flush()?;
        Ok(())
    }
    pub fn release_focus(&mut self) -> Result<()> {
        self.focus_requested = false;
        self.want_page = false;
        self.drop_typed();
        if self.page_has_focus()? { self.focus_parent()?; }
        Ok(())
    }
    pub fn overlay(&mut self, open: bool) -> Result<()> {
        if open {
            // The panel decides whether the page hides or is cut around (`visibility.rs`).
            self.release_focus()?;
        }
        Ok(())
    }
    fn windows(&self, pid: u32, marker: &str) -> Result<Vec<u32>> {
        let pid_atom = self.conn.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
        let mut queue = vec![(self.root, 0)];
        let mut found = vec![];
        while let Some((window, depth)) = queue.pop() {
            if window == self.parent {
                continue;
            }
            if depth > 0 {
                let class_match = self
                    .conn
                    .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)?
                    .reply()
                    .map(|p| String::from_utf8_lossy(&p.value).contains(marker))
                    .unwrap_or(false);
                let pid_match = self
                    .conn
                    .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)?
                    .reply()
                    .ok()
                    .and_then(|p| p.value32().and_then(|mut v| v.next()))
                    .is_some_and(|p| p == pid);
                let title_match = self
                    .conn
                    .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 256)?
                    .reply()
                    .map(|p| String::from_utf8_lossy(&p.value).contains(marker))
                    .unwrap_or(false);
                if (class_match && !marker.is_empty())
                    || (pid_match && (marker.is_empty() || title_match))
                {
                    found.push(window);
                    continue;
                }
            }
            if depth < 5 {
                if let Ok(tree) = self.conn.query_tree(window)?.reply() {
                    queue.extend(tree.children.into_iter().map(|w| (w, depth + 1)));
                }
            }
        }
        Ok(found)
    }
    pub fn find_window(&self, pid: u32, marker: &str) -> Result<Option<u32>> {
        for window in self.windows(pid, marker)? {
            let Ok(attrs) = self.conn.get_window_attributes(window)?.reply() else {
                continue;
            };
            let Ok(geometry) = self.conn.get_geometry(window)?.reply() else {
                continue;
            };
            // GTK/Chromium also creates tiny or InputOnly utility windows with
            // the same class. They are not the page drawable and cannot be embedded.
            if attrs.class == WindowClass::INPUT_OUTPUT
                && attrs.map_state == MapState::VIEWABLE
                && geometry.width > 64
                && geometry.height > 64
            {
                return Ok(Some(window));
            }
        }
        Ok(None)
    }
    /// Every page-sized engine window still outside aDOs, oldest first.
    pub fn page_windows(&self, pid: u32, marker: &str) -> Result<Vec<u32>> {
        let mut found = vec![];
        for window in self.windows(pid, marker)? {
            let Ok(attrs) = self.conn.get_window_attributes(window)?.reply() else { continue };
            let Ok(geometry) = self.conn.get_geometry(window)?.reply() else { continue };
            if attrs.class == WindowClass::INPUT_OUTPUT && attrs.map_state == MapState::VIEWABLE
                && geometry.width > 64 && geometry.height > 64
            {
                found.push(window);
            }
        }
        found.sort_unstable();
        Ok(found)
    }
    pub fn close_other_windows(&self, pid: u32, keep: u32) -> Result<()> {
        let protocols = self.conn.intern_atom(false, b"WM_PROTOCOLS")?.reply()?.atom;
        let delete = self
            .conn
            .intern_atom(false, b"WM_DELETE_WINDOW")?
            .reply()?
            .atom;
        for window in self.windows(pid, "")? {
            if window == keep {
                continue;
            }
            let event =
                ClientMessageEvent::new(32, window, protocols, [delete, CURRENT_TIME, 0, 0, 0]);
            self.conn
                .send_event(false, window, EventMask::NO_EVENT, event)?
                .check()?;
        }
        self.conn.flush()?;
        Ok(())
    }
    pub fn chrome_height(&self, child: u32, viewport_height: u32) -> Result<u32> {
        let height = u32::from(self.conn.get_geometry(child)?.reply()?.height);
        let chrome = height.saturating_sub(viewport_height);
        if chrome > 512 {
            bail!("Servo returned an invalid browser chrome height");
        }
        Ok(chrome)
    }
    /// Show `tiles` in the page area; `active` is the tab whose page owns the keyboard. A
    /// window already shown keeps its clip, so switching the active tile moves nothing. Windows
    /// no longer shown are unmapped but stay inside aDOs.
    pub fn show(&mut self, tiles: &[Tile], active: u64) -> Result<()> {
        if self.slot.clip == 0 { self.create_clip()?; }
        let active_window = tiles.iter().find(|t| t.tab == active).or(tiles.first()).map(|t| t.window);
        let old = self.slot.child;
        if old != 0 && Some(old) != active_window { self.step_aside()?; }
        let mut pool: Vec<Slot> = std::iter::once(self.slot).chain(self.others.drain(..)).collect();
        let mut placed: Vec<Option<Slot>> = vec![None; tiles.len()];
        for (i, tile) in tiles.iter().enumerate() {
            if let Some(at) = pool.iter().position(|s| s.child != 0 && s.child == tile.window) {
                let mut slot = pool.remove(at);
                if slot.clip_top != tile.clip_top { slot.rect = None; }
                slot.tab = tile.tab;
                slot.clip_top = tile.clip_top;
                placed[i] = Some(slot);
            }
        }
        let mut free = std::mem::take(&mut self.spare);
        for slot in pool {
            if slot.child != 0 {
                if let Ok(cookie) = self.conn.ungrab_key(Grab::ANY, slot.child, ModMask::ANY) { let _ = cookie.check(); }
                if let Ok(cookie) = self.conn.unmap_window(slot.child) { let _ = cookie.check(); }
            }
            if slot.mapped { self.conn.unmap_window(slot.clip)?.check()?; }
            free.push(slot.clip);
        }
        for (i, tile) in tiles.iter().enumerate() {
            if placed[i].is_some() { continue; }
            let clip = match free.pop() { Some(clip) => clip, None => self.new_clip()? };
            self.adopt(tile.window)?;
            self.conn.reparent_window(tile.window, clip, 0, -(tile.clip_top.min(i16::MAX as u32) as i16))?.check()?;
            placed[i] = Some(Slot { tab: tile.tab, clip, child: tile.window, clip_top: tile.clip_top, rect: None, mapped: false });
        }
        let mut placed: Vec<Slot> = placed.into_iter().flatten().collect();
        let at = tiles.iter().position(|t| t.tab == active).unwrap_or(0).min(placed.len().saturating_sub(1));
        self.slot = if placed.is_empty() { Slot { clip: free.pop().unwrap_or(self.slot.clip), ..Slot::default() } } else { placed.remove(at) };
        self.others = placed;
        self.spare = free;
        if self.slot.child != old && self.slot.child != 0 {
            self.engine_keyboard_ready = self.keyboard_ready.contains(&self.slot.child);
            self.grab_shortcuts(None)?;
        }
        self.place_all()?;
        self.conn.flush()?;
        Ok(())
    }
    /// The page that owned the keyboard steps aside (another tile or tab takes over): the
    /// keyboard comes back to aDOs and its key grabs go; it may already be gone, so errors
    /// from it are ignored.
    fn step_aside(&mut self) -> Result<()> {
        self.release_focus()?;
        if self.engine_keyboard_ready && !self.keyboard_ready.contains(&self.slot.child) { self.keyboard_ready.push(self.slot.child); }
        if let Ok(cookie) = self.conn.ungrab_key(Grab::ANY, self.slot.child, ModMask::ANY) { let _ = cookie.check(); }
        Ok(())
    }
    /// A press in another shown tile: it becomes the active one at once, so the click lands
    /// and the keys typed next go to it (F3). The panel hears it from `take_pressed_tile`.
    fn activate(&mut self, clip: u32) -> Result<()> {
        let Some(i) = self.others.iter().position(|s| s.clip == clip) else { return Ok(()) };
        if self.slot.child != 0 {
            if self.engine_keyboard_ready && !self.keyboard_ready.contains(&self.slot.child) { self.keyboard_ready.push(self.slot.child); }
            if let Ok(cookie) = self.conn.ungrab_key(Grab::ANY, self.slot.child, ModMask::ANY) { let _ = cookie.check(); }
        }
        std::mem::swap(&mut self.slot, &mut self.others[i]);
        self.engine_keyboard_ready = self.keyboard_ready.contains(&self.slot.child);
        self.grab_shortcuts(None)?;
        self.pressed_tile = Some(self.slot.tab);
        Ok(())
    }
    pub fn take_pressed_tile(&mut self) -> Option<u64> { self.pressed_tile.take() }
    /// Take `child` into aDOs without showing it: out of the window manager, inside the clip, unmapped.
    pub fn adopt(&mut self, child: u32) -> Result<()> {
        if self.slot.clip == 0 { self.create_clip()?; }
        if self.adopted.contains(&child) { return Ok(()); }
        self.conn.unmap_window(child)?.check()?;
        // Finish WM withdrawal before embedding. Otherwise xfwm can handle the
        // unmap AFTER our reparent and move Servo back to root, losing the crop.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while self.conn.query_tree(child)?.reply()?.parent != self.root {
            if std::time::Instant::now() >= deadline {
                bail!("The window manager did not release the browser window; use Retry");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        self.conn.change_window_attributes(child, &ChangeWindowAttributesAux::new().override_redirect(1))?.check()?;
        self.conn.reparent_window(child, self.slot.clip, 0, 0)?.check()?;
        self.conn
            .change_window_attributes(
                child,
                &ChangeWindowAttributesAux::new()
                    .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
            )?
            .check()?;
        self.adopted.push(child);
        self.conn.flush()?;
        Ok(())
    }
    /// The first clip, and the focus proxy beside it.
    fn create_clip(&mut self) -> Result<()> {
        self.slot.clip = self.new_clip()?;
        self.focus_proxy = self.conn.generate_id()?;
        self.conn
            .create_window(
                0,
                self.focus_proxy,
                self.parent,
                -1,
                -1,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new().event_mask(EventMask::KEY_PRESS | EventMask::KEY_RELEASE),
            )?
            .check()?;
        self.conn.map_window(self.focus_proxy)?.check()?;
        self.slot.mapped = false;
        Ok(())
    }
    /// A clip window for one shown page (unmapped until it has a page and a place).
    fn new_clip(&mut self) -> Result<u32> {
        let clip = self.conn.generate_id()?;
        self.conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                clip,
                self.parent,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().background_pixel(0x0003_0405)
                    // Entering the page: G3 hover timing (focus) and ending GPUI hover (visual).
                    .event_mask(EventMask::ENTER_WINDOW),
            )?
            .check()?;
        // Keep native pixels in server-owned offscreen storage. Without this,
        // an uncomposited Vulkan parent present can cover the embedded page.
        self.conn.composite_query_version(0, 4)?.reply()?;
        self.conn.composite_redirect_window(clip, Redirect::AUTOMATIC)?.check()?;
        // Reparented clients no longer get focus-on-click from the window manager.
        // Focus the browser, then replay the ORIGINAL click (including drags/wheel).
        self.conn
            .grab_button(
                false,
                clip,
                EventMask::BUTTON_PRESS,
                GrabMode::SYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                x11rb::NONE,
                ButtonIndex::ANY,
                ModMask::ANY,
            )?
            .check()?;
        Ok(clip)
    }
    /// Each shown page's rectangle in the aDOs window, by aDOs tab; tab 0 means the one
    /// page when nothing is tiled.
    pub fn geometry(&mut self, rects: Vec<(u64, Rect)>) -> Result<()> {
        if self.rects == rects { return Ok(()); }
        self.rects = rects;
        self.place_all()
    }
    fn rect_for(&self, tab: u64, active: bool) -> Option<Rect> {
        match self.rects.as_slice() {
            [(0, rect)] => active.then_some(*rect),
            rects => rects.iter().find(|(t, _)| *t == tab).map(|(_, r)| *r),
        }
    }
    fn place_all(&mut self) -> Result<()> {
        let rect = self.rect_for(self.slot.tab, true);
        if place(&self.conn, &mut self.slot, rect)? { apply_cutout(&self.conn, &self.slot, &self.hole, self.has_shape)?; }
        for i in 0..self.others.len() {
            let rect = self.rect_for(self.others[i].tab, false);
            if place(&self.conn, &mut self.others[i], rect)? { apply_cutout(&self.conn, &self.others[i], &self.hole, self.has_shape)?; }
        }
        self.sync_visibility()
    }
    pub fn cutout(&mut self, hole: crate::cutout::Holes) -> Result<()> {
        if self.hole == hole { return Ok(()); }
        self.hole = hole;
        apply_cutout(&self.conn, &self.slot, &self.hole, self.has_shape)?;
        for slot in &self.others { apply_cutout(&self.conn, slot, &self.hole, self.has_shape)?; }
        self.sync_visibility()
    }
    pub fn visibility(&mut self, visible: bool) -> Result<()> {
        self.visible = visible;
        self.sync_visibility()
    }
    pub fn sync_visibility(&mut self) -> Result<()> {
        let shown = self.visible && self.parent_visible && (self.has_shape || self.hole.is_empty());
        for slot in &mut self.others {
            let map = shown && slot.child != 0 && slot.rect.is_some_and(|r| r.w > 0 && r.h > 0);
            if map != slot.mapped {
                if map {
                    self.conn.map_window(slot.child)?.check()?;
                    self.conn.map_window(slot.clip)?.check()?;
                } else {
                    self.conn.unmap_window(slot.child)?.check()?;
                    self.conn.unmap_window(slot.clip)?.check()?;
                }
                slot.mapped = map;
                self.conn.flush()?;
            }
        }
        if self.slot.child == 0 {
            return Ok(());
        }
        let map = shown && self.slot.rect.is_some_and(|r| r.w > 0 && r.h > 0);
        if map != self.slot.mapped {
            if map {
                self.conn.map_window(self.slot.child)?.check()?;
                self.conn.map_window(self.slot.clip)?.check()?;
            } else {
                self.release_focus()?;
                self.conn.unmap_window(self.slot.child)?.check()?;
                self.conn.unmap_window(self.slot.clip)?.check()?;
            }
            self.slot.mapped = map;
            self.conn.flush()?;
        }
        self.try_pending_focus()?;
        Ok(())
    }
    pub fn repaint(&self) -> Result<()> {
        for slot in std::iter::once(&self.slot).chain(&self.others) {
            if slot.child == 0 || !slot.mapped { continue; }
            let geometry = self.conn.get_geometry(slot.child)?.reply()?;
            let expose = ExposeEvent {
                response_type: EXPOSE_EVENT,
                sequence: 0,
                window: slot.child,
                x: 0,
                y: 0,
                width: geometry.width,
                height: geometry.height,
                count: 0,
            };
            self.conn.send_event(false, slot.child, EventMask::EXPOSURE, expose)?.check()?;
        }
        self.conn.flush()?;
        Ok(())
    }
    pub fn focus_content(&mut self) -> Result<()> {
        if self.slot.child != 0 && self.slot.mapped {
            if !self.page_has_focus()? { self.down_at_focus = self.conn.query_keymap()?.reply()?.keys; }
            self.conn
                .set_input_focus(InputFocus::PARENT, self.slot.child, CURRENT_TIME)?
                .check()?;
            self.conn.flush()?;
        }
        Ok(())
    }
    pub fn cancel_focus_page(&mut self) -> Result<()> {
        self.focus_requested = false;
        self.want_page = false;
        self.drop_typed();
        Ok(())
    }
    /// A plain key GPUI delivered to the page proxy (`keysym`): the page gets it as soon as
    /// it holds X focus. The page being shown has an X event (MapNotify); one that never shows
    /// has none, so keys older than `TYPED_WAIT` are dropped.
    pub fn typed(&mut self, keysym: u32) -> Result<()> {
       
        if !self.want_page || self.slot.child == 0 { return Ok(()); }
        let Some(keycode) = self.keycode(keysym)? else { return Ok(()) };
        self.typed.push(keycode);
        self.typed_since.get_or_insert_with(std::time::Instant::now);
        if !self.focus_requested && self.page_has_focus()? { self.replay_typed()?; }
        Ok(())
    }
    fn replay_typed(&mut self) -> Result<()> {
        self.typed_since = None;
        if self.typed.is_empty() { return Ok(()); }
        use x11rb::protocol::xtest::ConnectionExt as _;
        for keycode in std::mem::take(&mut self.typed) {
            // Held when the page took focus: the engine replayed it, and its real release follows.
            if self.down_at_focus[usize::from(keycode / 8)] & (1 << (keycode % 8)) != 0 { continue; }
           
            self.conn.xtest_fake_input(KEY_PRESS_EVENT, keycode, CURRENT_TIME, self.root, 0, 0, 0)?.check()?;
            self.conn.xtest_fake_input(KEY_RELEASE_EVENT, keycode, CURRENT_TIME, self.root, 0, 0, 0)?.check()?;
        }
        self.conn.flush()?;
        Ok(())
    }
    fn drop_typed(&mut self) {
        self.typed.clear();
        self.typed_since = None;
    }
    pub fn typed_due(&self) -> Option<std::time::Instant> { self.typed_since.map(|at| at + crate::focus::TYPED_WAIT) }
    pub fn flush_stale_typed(&mut self) -> Result<()> {
        if self.typed_due().is_some_and(|due| due <= std::time::Instant::now()) { self.drop_typed(); }
        Ok(())
    }
    pub fn focus_page(&mut self) -> Result<()> {
        // A tab can be activated before its first paint remaps the native child.
        // Keep the intent until the mapping receipt, not a guessed delay.
        self.focus_requested = true;
        self.want_page = true;
        self.focus_deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        self.try_pending_focus()
    }
    /// True while a page-focus request waits for the keys of the shortcut that made it
    /// (Ctrl+4, Enter in the palette) to be released. The host loop re-checks it.
    pub fn focus_pending(&self) -> bool {
        self.focus_requested && self.slot.mapped && self.slot.child != 0
    }
    /// Hand the keyboard to the page only once no key is physically down. Focusing the
    /// child while the shortcut's keys are still held makes the engine see them as fresh
    /// presses (winit synthesises presses for held keys on focus-in, then autorepeat
    /// follows), so Ctrl+4 typed a "4" into the page field.
    pub fn try_pending_focus(&mut self) -> Result<()> {
        if self.focus_pending() && !self.app_has_focus()? {
            // Another application has the keyboard: keep `want_page`, the FocusIn brings it back.
            self.focus_requested = false;
            self.drop_typed();
        } else if self.focus_pending() && (std::time::Instant::now() >= self.focus_deadline || !self.keys_down()?) {
            self.focus_content()?;
            self.ready_engine_keyboard()?;
            self.focus_requested = false;
            self.replay_typed()?;
        }
        Ok(())
    }
    /// servoshell (the only engine with cropped chrome, so `clip_top > 0`) gives Tab to its
    /// own hidden egui toolbar until its page widget has egui focus; a click sets that, X
    /// focus alone does not. So keyboard-only users could never Tab into the page. egui
    /// orders the page widget last, so one Shift+Tab with nothing focused selects it. Send
    /// that once per window, past our own Shift+Tab grab, before the user's keys arrive.
    fn ready_engine_keyboard(&mut self) -> Result<()> {
        if self.engine_keyboard_ready || self.slot.clip_top == 0 || self.slot.child == 0 { return Ok(()); }
        self.engine_keyboard_ready = true;
        // The page widget only accepts egui focus once servoshell has seen its window
        // gain focus (a round trip through Servo), so let focus_content settle first.
        std::thread::sleep(std::time::Duration::from_millis(250));
        use x11rb::protocol::xtest::ConnectionExt as _;
        let (Some(shift), Some(tab)) = (self.keycode(0xffe1)?, self.keycode(0xff09)?) else { return Ok(()); };
        self.conn.ungrab_key(tab, self.slot.child, ModMask::ANY)?.check()?;
        for (kind, key) in [(KEY_PRESS_EVENT, shift), (KEY_PRESS_EVENT, tab), (KEY_RELEASE_EVENT, tab), (KEY_RELEASE_EVENT, shift)] {
            self.conn.xtest_fake_input(kind, key, CURRENT_TIME, self.root, 0, 0, 0)?.check()?;
        }
        // Let the server deliver the fake keys before the grab returns.
        self.conn.get_input_focus()?.reply()?;
        std::thread::sleep(std::time::Duration::from_millis(30));
        self.grab_shortcuts(Some(tab))
    }
    /// Passive grabs that hand aDOs shortcuts back to GPUI while the page has X focus.
    fn grab_shortcuts(&self, only: Option<u8>) -> Result<()> {
        let all = self.shortcuts.iter().map(|&(code, mask, _)| (code, mask)).chain(self.dropped.iter().copied());
        for (keycode, modifiers) in all {
            if only.is_some_and(|only| only != keycode) { continue; }
            for extra in [ModMask::default(), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2] {
                self.conn
                    .grab_key(false, self.slot.child, ModMask::from(modifiers) | extra, keycode, GrabMode::ASYNC, GrabMode::SYNC)?
                    .check()?;
            }
        }
        Ok(())
    }
    fn keycode(&self, keysym: u32) -> Result<Option<u8>> {
        let setup = self.conn.setup();
        let mapping = self.conn.get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)?.reply()?;
        Ok(mapping.keysyms.chunks(usize::from(mapping.keysyms_per_keycode))
            .position(|syms| syms.contains(&keysym))
            .map(|position| setup.min_keycode + position as u8))
    }
    fn keys_down(&self) -> Result<bool> {
        let keymap = self.conn.query_keymap()?.reply()?;
        Ok(keymap.keys.iter().any(|&bits| bits != 0))
    }
    pub fn focus_parent(&self) -> Result<()> {
        // End the passive key grab BEFORE focusing GPUI. Otherwise its later
        // NotifyUngrab FocusOut makes GPUI think the address input lost focus.
        if !self.keyboard_held { self.conn.ungrab_keyboard(CURRENT_TIME)?.check()?; }
        if !self.app_has_focus()? { return Ok(()); }
        self.conn
            .set_input_focus(
                InputFocus::PARENT,
                if self.focus_proxy != 0 {
                    self.focus_proxy
                } else {
                    self.parent
                },
                CURRENT_TIME,
            )?
            .check()?;
        // GPUI treats the native NotifyInferior FocusOut (parent -> proxy) as
        // losing activation. Restore its focus bookkeeping while the sibling
        // proxy keeps keys away from the embedded browser under the pointer.
        self.conn
            .send_event(
                false,
                self.parent,
                EventMask::FOCUS_CHANGE,
                FocusInEvent {
                    response_type: FOCUS_IN_EVENT,
                    detail: NotifyDetail::NONLINEAR,
                    sequence: 0,
                    event: self.parent,
                    mode: NotifyMode::NORMAL,
                },
            )?
            .check()?;
        self.conn.flush()?;
        Ok(())
    }
    /// An engine window that had the keyboard was destroyed (a sign-in popup closing itself).
    /// The window manager had seen it as its own client and now activates nothing, so the
    /// keyboard would go nowhere. When no application is active, aDOs takes it back.
    fn reclaim_focus(&mut self) -> Result<()> {
        if self.app_has_focus()? { return Ok(()); }
        let active_atom = self.conn.intern_atom(false, b"_NET_ACTIVE_WINDOW")?.reply()?.atom;
        let active = self.conn.get_property(false, self.root, active_atom, AtomEnum::WINDOW, 0, 1)?.reply()?
            .value32().and_then(|mut v| v.next()).unwrap_or(0);
        if active != 0 && active != self.parent { return Ok(()); }
        // Source 2 (a pager): the window manager's focus-stealing guard must not refuse it.
        let event = ClientMessageEvent::new(32, self.parent, active_atom, [2, CURRENT_TIME, 0, 0, 0]);
        self.conn.send_event(false, self.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)?.check()?;
        self.conn.set_input_focus(InputFocus::PARENT, self.parent, CURRENT_TIME)?.check()?;
        if self.want_page { self.focus_requested = true; self.focus_deadline = std::time::Instant::now() + std::time::Duration::from_secs(1); }
        self.conn.flush()?;
        Ok(())
    }
    pub fn events(&mut self) -> Result<(bool, Vec<usize>)> {
        let mut changed = false;
        let mut shortcuts = vec![];
        while let Some(event) = self.conn.poll_for_event()? {
            match event {
                Event::PropertyNotify(e)
                    if e.window == self.slot.child
                        && (e.atom == self.title_atom
                            || e.atom == u32::from(AtomEnum::WM_NAME)) =>
                {
                    changed = true;
                }
                Event::MapNotify(e) if e.window == self.parent => {
                    self.parent_visible = true;
                    self.sync_visibility()?;
                }
                Event::MapNotify(e) if e.event == self.root && e.window != self.slot.child && !e.override_redirect => {
                    changed = true;
                }
                Event::UnmapNotify(e) if e.window == self.parent => {
                    self.parent_visible = false;
                    self.sync_visibility()?;
                }
                Event::DestroyNotify(e) if e.window == self.parent => {
                    bail!("Browser window was closed")
                }
                // An engine window went away (its last tab closed, a sign-in popup closed
                // itself): the driver lists the tabs again and selects another.
                Event::DestroyNotify(e) if self.adopted.contains(&e.window) => {
                    self.adopted.retain(|w| *w != e.window);
                    self.keyboard_ready.retain(|w| *w != e.window);
                    if e.window == self.slot.child {
                        self.slot.child = 0;
                        if self.slot.mapped { self.conn.unmap_window(self.slot.clip)?.check()?; }
                        self.slot.mapped = false;
                    }
                    for slot in self.others.iter_mut().filter(|s| s.child == e.window) {
                        slot.child = 0;
                        if slot.mapped { self.conn.unmap_window(slot.clip)?.check()?; }
                        slot.mapped = false;
                    }
                    self.reclaim_focus()?;
                    changed = true;
                }
                // The window manager gave aDOs the focus back from another application: it lands
                // on the top-level, so hand it on to the page if the page owned it (F7).
                Event::FocusIn(e) if e.event == self.parent && e.response_type & 0x80 == 0
                    && e.mode == NotifyMode::NORMAL
                    && !matches!(e.detail, NotifyDetail::INFERIOR | NotifyDetail::POINTER | NotifyDetail::POINTER_ROOT | NotifyDetail::NONE)
                    && self.want_page =>
                {
                    self.focus_requested = true;
                    self.focus_deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
                }
                Event::EnterNotify(e) if e.event == self.slot.clip || self.others.iter().any(|s| s.clip == e.event) => {
                    self.entered_at = u32::from(e.time);
                    self.pointer_entered = true;
                }
                Event::ButtonPress(e) if e.event == self.slot.clip || self.others.iter().any(|s| s.clip == e.event) => {
                    if e.event != self.slot.clip { self.activate(e.event)?; }
                    self.page_pressed = true;
                    let state = u16::from(e.state);
                    self.press_behind = e.detail == 2
                        || (state & u16::from(ModMask::CONTROL) != 0 && state & u16::from(ModMask::SHIFT) == 0);
                    self.engine_keyboard_ready = true;
                    self.focus_content()?;
                    let time = u32::from(e.time);
                    if crate::focus::hover_unseen(self.entered_at, time) {
                        self.replay = Some((time, std::time::Instant::now() + crate::focus::HOVER_FRAME));
                    } else {
                        self.conn.allow_events(Allow::REPLAY_POINTER, time)?.check()?;
                        self.conn.flush()?;
                    }
                }
                Event::KeyPress(mut e) if e.event == self.focus_proxy => {
                    e.event = self.parent;
                    e.child = x11rb::NONE;
                    self.conn
                        .send_event(false, self.parent, EventMask::KEY_PRESS, e)?
                        .check()?;
                    self.conn.flush()?;
                }
                Event::KeyRelease(mut e) if e.event == self.focus_proxy => {
                    e.event = self.parent;
                    e.child = x11rb::NONE;
                    self.conn
                        .send_event(false, self.parent, EventMask::KEY_RELEASE, e)?
                        .check()?;
                    self.conn.flush()?;
                }
                Event::KeyPress(e) => {
                    let state = u16::from(e.state) & !u16::from(ModMask::LOCK | ModMask::M2);
                    if let Some((_, _, index)) = self.shortcuts.iter().find(|(code, mask, _)| *code == e.detail && *mask == state) {
                        shortcuts.push(*index);
                        self.conn.grab_keyboard(false, self.focus_proxy, CURRENT_TIME,
                            GrabMode::ASYNC, GrabMode::SYNC)?.reply()?;
                        self.keyboard_held = true;
                        self.release_focus()?;
                    } else if self.dropped.iter().any(|&(code, mask)| code == e.detail && mask == state) {
                        // An engine window chord: swallowed, the engine never sees it.
                        self.conn.allow_events(Allow::ASYNC_KEYBOARD, CURRENT_TIME)?.check()?;
                        self.conn.flush()?;
                    }
                }
                _ => {}
            }
        }
        Ok((changed, shortcuts))
    }
}

/// X modifiers and keysym for a canonical chord such as `ctrl-shift-k` or `ctrl--`.
pub(crate) fn chord_keysym(chord: &str) -> Option<(ModMask, u32)> {
    let (mods, key) = match chord.strip_suffix("--") {
        Some(rest) => (rest, "-"),
        None => chord.rsplit_once('-').unwrap_or(("", chord)),
    };
    let mut modifiers = ModMask::default();
    for part in mods.split('-').filter(|p| !p.is_empty()) {
        match part {
            "ctrl" => modifiers |= ModMask::CONTROL,
            "shift" => modifiers |= ModMask::SHIFT,
            "alt" => modifiers |= ModMask::M1,
            "super" => modifiers |= ModMask::M4,
            _ => return None,
        }
    }
    let symbol = match key {
        "space" => 0x20,
        "escape" => 0xff1b,
        "tab" => 0xff09,
        "enter" => 0xff0d,
        "backspace" => 0xff08,
        "delete" => 0xffff,
        "insert" => 0xff63,
        "home" => 0xff50,
        "end" => 0xff57,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        k if k.len() == 1 => u32::from(k.chars().next()?),
        k => {
            let n: u32 = k.strip_prefix('f')?.parse().ok()?;
            if !(1..=24).contains(&n) { return None; }
            0xffbe + n - 1
        }
    };
    Some((modifiers, symbol))
}

/// Put `slot` at `rect` (the clip there, the engine window inside it raised by its toolbar
/// height). True when it moved.
fn place(conn: &RustConnection, slot: &mut Slot, rect: Option<Rect>) -> Result<bool> {
    let rect = rect.filter(|r| r.w > 0 && r.h > 0);
    if slot.child == 0 || slot.rect == rect { return Ok(false); }
    slot.rect = rect;
    let Some(rect) = rect else { return Ok(false) };
    conn.configure_window(slot.clip, &ConfigureWindowAux::new().x(rect.x).y(rect.y).width(rect.w).height(rect.h).border_width(0))?.check()?;
    conn.configure_window(slot.child, &ConfigureWindowAux::new().x(0).y(-(slot.clip_top as i32)).width(rect.w).height(rect.h + slot.clip_top).border_width(0))?.check()?;
    Ok(true)
}

/// Cut the floating card (window coordinates) out of one shown page.
fn apply_cutout(conn: &RustConnection, slot: &Slot, holes: &crate::cutout::Holes, has_shape: bool) -> Result<()> {
    match slot.rect {
        Some(rect) if slot.child != 0 && has_shape && rect.w > 0 && rect.h > 0 => {
            let local = crate::cutout::Holes {
                rects: holes.rects.iter().map(|h| Rect { x: h.x - rect.x, y: h.y - rect.y, ..*h }).collect(),
                modal: holes.modal,
            };
            crate::cutout::apply(conn, slot.clip, rect, &local)
        }
        _ => Ok(()),
    }
}
