//! Windows: the Browser's engine layer on WebView2 (`webview2-com` 0.39.1, read from source).
//! Same public items as the Linux `runtime.rs`, so the panel code is shared.
//!
//! - One WebView2 controller per tab, each inside a host child window of the aDOs window.
//!   Showing a page moves its host; a floating card cuts a hole in the host with a window
//!   region (`SetWindowRgn`), and if that fails the page hides while the card is open.
//! - Everything runs on the GPUI thread (WebView2 needs the thread that created it, and GPUI's
//!   message loop delivers its callbacks). No extra threads; events reach the panel through
//!   the same channel as on Linux.
//! - aDOs chords win in the page: `AcceleratorKeyPressed` hands them to aDOs, and the engine's
//!   own browser shortcuts are off.
//! - Pages talk back with `chrome.webview.postMessage`: the pointer coming onto a page (aDOs
//!   ends its hover) and links a leader tile hands to its follower. No polling.
use crate::{cutout::Holes, protocol::{self, Page}, state::Engine, xembed::Rect};
use anyhow::{anyhow, Result};
use std::{cell::RefCell, collections::HashMap, rc::{Rc, Weak}, sync::Arc};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    core::{w, Interface, PCWSTR, PWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ, RGN_DIFF},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{GetKeyState, SetFocus, VkKeyScanW, VK_CONTROL, VK_MENU, VK_SHIFT},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SetWindowPos, ShowWindow,
                SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, WINDOW_EX_STYLE, WNDCLASSW,
                WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
            },
        },
    },
};

#[derive(Clone, Debug)]
pub enum Event {
    Ready,
    /// A snapshot of the page in tab `id`.
    Page(u64, Page),
    /// A page opened a tab or window (`window.open`, `target=_blank`): it is now tab `id`.
    Opened { id: u64, opener: Option<u64>, url: String, title: String },
    /// Tab `id` closed itself (`window.close()`).
    Closed(u64),
    /// A click in tile `id` gave it the keyboard.
    TilePressed(u64),
    /// Follower tab `id` opens the address clicked in its leader.
    Followed(u64, String),
    Error(String),
    Failed(String),
    Shortcut(usize),
    /// A press in the page; `behind`: Ctrl held, so a tab it opens waits in the background.
    PagePressed { behind: bool },
    /// The pointer moved onto the page window.
    PointerEntered,
    PageExit,
}
#[derive(Clone, Debug)]
pub enum CommandMsg {
    /// Evaluate a script expression in the page; the reply carries its value as text.
    Eval(String, protocol::Reply),
    /// The page as a base64 PNG, from the engine's own capture.
    Screenshot(protocol::Reply),
    /// The hint script's settings changed (keymap or colours).
    Hints(serde_json::Value),
    Navigate(String),
    Back,
    Forward,
    Reload,
    Snapshot,
    /// Show tab `id`; a restored tab not yet in the engine opens at the address first.
    Select(u64, String),
    /// A new tab `id` at the address, selected.
    NewTab(u64, String),
    /// Close tab `id`.
    Close(u64),
    /// The tabs tiled with the next selected one, in order, with their addresses.
    Tiles(Vec<(u64, String)>),
    /// (follower, leader) pairs.
    Follow(Vec<(u64, u64)>),
    Stop,
}

/// Ids for tabs a page opened; aDOs's own tabs are numbered below this (`tabs.rs`).
const PAGE_OPENED: u64 = 1 << 40;

struct Tab {
    host: HWND,
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
}

/// One aDOs chord in Windows terms, for `AcceleratorKeyPressed`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Chord { ctrl: bool, shift: bool, alt: bool, vk: u32 }

struct Inner {
    parent: HWND,
    events: async_channel::Sender<Event>,
    env: Option<ICoreWebView2Environment>,
    tabs: HashMap<u64, Tab>,
    /// Tabs whose controller is on its way, with the address to load when it arrives.
    creating: HashMap<u64, String>,
    /// Commands sent before WebView2 was ready.
    queued: Vec<CommandMsg>,
    selected: u64,
    /// The tabs tiled with the selected one (empty: it is shown alone).
    tiles: Vec<u64>,
    /// The tab with the keyboard (the selected one, or a pressed tile).
    keyboard: u64,
    geometry: Vec<(u64, Rect)>,
    visible: bool,
    holes: Holes,
    chords: Vec<Option<Chord>>,
    /// Chords at or after this index are the page exits (Esc, Shift+Tab).
    shell_chords: usize,
    hints: serde_json::Value,
    /// (follower, leader) pairs, and each leader's added script (empty while WebView2 adds it).
    follows: Vec<(u64, u64)>,
    lead_scripts: HashMap<u64, String>,
    /// Our own `MoveFocus` is under way: the page getting focus is not a click.
    focusing: bool,
    next_opened: u64,
    stopped: bool,
}

pub struct Runtime {
    inner: Rc<RefCell<Inner>>,
}

impl Runtime {
    pub fn start(
        parent: u32,
        _engine: Engine,
        tab: u64,
        url: String,
        shortcuts: Vec<String>,
        hints: serde_json::Value,
    ) -> Result<(Arc<Self>, async_channel::Receiver<Event>)> {
        let (events, receiver) = async_channel::unbounded();
        let (chords, shell_chords) = chords(shortcuts);
        let inner = Rc::new(RefCell::new(Inner {
            parent: HWND(parent as usize as *mut _),
            events,
            env: None,
            tabs: HashMap::new(),
            creating: HashMap::from([(tab, url)]),
            queued: vec![],
            selected: tab,
            tiles: vec![],
            keyboard: tab,
            geometry: vec![],
            visible: false,
            holes: Holes::default(),
            chords,
            shell_chords,
            hints,
            follows: vec![],
            lead_scripts: HashMap::new(),
            focusing: false,
            next_opened: PAGE_OPENED,
            stopped: false,
        }));
        register_host_class();
        let folder = wide(&data_folder().display().to_string());
        let weak = Rc::downgrade(&inner);
        let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |result, env| {
            let Some(inner) = weak.upgrade() else { return Ok(()) };
            match (result, env) {
                (Ok(()), Some(env)) => {
                    let (tabs, queued) = {
                        let mut i = inner.borrow_mut();
                        i.env = Some(env);
                        (i.creating.clone(), std::mem::take(&mut i.queued))
                    };
                    for (id, url) in tabs { create_tab(&inner, id, Some(url), None); }
                    for msg in queued { command(&inner, msg); }
                }
                (result, _) => {
                    let detail = result.err().map(|e| e.message()).unwrap_or_default();
                    eprintln!("WebView2 did not start: {detail}");
                    let _ = inner.borrow().events.try_send(Event::Failed(format!(
                        "The Browser needs Microsoft Edge WebView2, which did not start ({detail}). It is part of Windows 11; on Windows 10 install the WebView2 Runtime."
                    )));
                }
            }
            Ok(())
        }));
        // SAFETY: the folder buffer outlives the call; the handler is a COM object WebView2 keeps.
        unsafe { CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), PCWSTR(folder.as_ptr()), None::<&ICoreWebView2EnvironmentOptions>, &handler) }
            .map_err(|e| anyhow!("The Browser could not start WebView2: {}", e.message()))?;
        Ok((Arc::new(Self { inner }), receiver))
    }

    pub fn command(&self, msg: CommandMsg) {
        command(&self.inner, msg);
    }
    pub fn geometry(&self, rects: Vec<(u64, Rect)>) {
        self.inner.borrow_mut().geometry = rects;
        apply(&self.inner);
    }
    pub fn visible(&self, visible: bool) {
        if self.inner.borrow().visible == visible { return; }
        self.inner.borrow_mut().visible = visible;
        apply(&self.inner);
    }
    pub fn shortcut_done(&self) {}
    pub fn shortcuts(&self, keys: Vec<String>) {
        let (chords, shell) = chords(keys);
        let mut i = self.inner.borrow_mut();
        i.chords = chords;
        i.shell_chords = shell;
    }
    /// Linux replays a key typed before the page held X focus; on Windows focus moves at once.
    pub fn typed(&self, _keysym: u32) {}
    pub fn focus_page(&self) {
        let controller = { let i = self.inner.borrow(); i.tabs.get(&i.keyboard).map(|t| t.controller.clone()) };
        if let Some(controller) = controller {
            self.inner.borrow_mut().focusing = true;
            // SAFETY: a live controller on its own thread.
            let _ = unsafe { controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC) };
            self.inner.borrow_mut().focusing = false;
        }
    }
    pub fn focus_address(&self) {
        let parent = self.inner.borrow().parent;
        // SAFETY: the aDOs window, on its own thread.
        let _ = unsafe { SetFocus(Some(parent)) };
    }
    pub fn overlay(&self, open: bool) {
        if open { self.focus_address(); }
    }
    pub fn cutout(&self, holes: Holes) {
        if self.inner.borrow().holes == holes { return; }
        self.inner.borrow_mut().holes = holes;
        apply(&self.inner);
    }
    /// Save `rect` of the page (viewport pixels) as a PNG at `path`.
    pub fn shot(&self, rect: Rect, _viewport_h: u32, path: std::path::PathBuf) -> async_channel::Receiver<Result<std::path::PathBuf, String>> {
        let (tx, rx) = async_channel::bounded(1);
        let webview = { let i = self.inner.borrow(); i.tabs.get(&i.selected).map(|t| t.webview.clone()) };
        let Some(webview) = webview else {
            let _ = tx.try_send(Err("no page is selected".into()));
            return rx;
        };
        capture(&webview, move |result| {
            let saved = result.and_then(|b64| crop_png(&b64, rect, &path).map_err(|e| e.to_string())).map(|()| path);
            let _ = tx.try_send(saved);
        });
        rx
    }
    pub fn repaint(&self) {}
    pub fn stop(&self) {
        stop(&self.inner);
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop();
    }
}

fn stop(inner: &Rc<RefCell<Inner>>) {
    let tabs: Vec<Tab> = {
        let mut i = inner.borrow_mut();
        if i.stopped { return; }
        i.stopped = true;
        i.creating.clear();
        i.tabs.drain().map(|(_, t)| t).collect()
    };
    for tab in tabs { close(tab); }
}

fn command(inner: &Rc<RefCell<Inner>>, msg: CommandMsg) {
    if inner.borrow().stopped { return; }
    if inner.borrow().env.is_none() && !matches!(msg, CommandMsg::Stop) {
        // Commands for the first tab can wait for WebView2; the address is kept in `creating`.
        let mut i = inner.borrow_mut();
        match msg {
            CommandMsg::Navigate(url) => { let s = i.selected; i.creating.insert(s, url); }
            CommandMsg::Select(id, url) | CommandMsg::NewTab(id, url) => { i.selected = id; i.keyboard = id; i.creating.entry(id).or_insert(url); }
            msg => i.queued.push(msg),
        }
        return;
    }
    let selected = { let i = inner.borrow(); i.tabs.get(&i.selected).map(|t| t.webview.clone()) };
    match msg {
        CommandMsg::Eval(script, reply) => match selected {
            Some(webview) => eval(&webview, &script, move |result| { let _ = reply.try_send(result); }),
            None => { let _ = reply.try_send(Err("no page is selected".into())); }
        },
        CommandMsg::Screenshot(reply) => match selected {
            Some(webview) => capture(&webview, move |result| { let _ = reply.try_send(result); }),
            None => { let _ = reply.try_send(Err("no page is selected".into())); }
        },
        CommandMsg::Hints(config) => {
            let call = crate::hints::config_call(&config);
            inner.borrow_mut().hints = config;
            let pages: Vec<ICoreWebView2> = inner.borrow().tabs.values().map(|t| t.webview.clone()).collect();
            for webview in pages {
                add_script(&webview, &call);
                eval(&webview, &call, |_| {});
            }
        }
        CommandMsg::Navigate(url) => match selected {
            Some(webview) => navigate(&webview, &url),
            None => { let mut i = inner.borrow_mut(); let s = i.selected; i.creating.insert(s, url); }
        },
        CommandMsg::Back => if let Some(w) = selected { let _ = unsafe { w.GoBack() }; },
        CommandMsg::Forward => if let Some(w) = selected { let _ = unsafe { w.GoForward() }; },
        CommandMsg::Reload => if let Some(w) = selected { let _ = unsafe { w.Reload() }; },
        CommandMsg::Snapshot => { let s = inner.borrow().selected; snapshot(inner, s); }
        CommandMsg::Select(id, url) | CommandMsg::NewTab(id, url) => {
            let known = { let mut i = inner.borrow_mut(); i.selected = id; i.keyboard = id; i.tabs.contains_key(&id) };
            if known {
                apply(inner);
                let _ = inner.borrow().events.try_send(Event::Ready);
                snapshot(inner, id);
            } else {
                create_tab(inner, id, Some(url), None);
            }
        }
        CommandMsg::Close(id) => {
            let tab = { let mut i = inner.borrow_mut(); i.creating.remove(&id); i.lead_scripts.remove(&id); i.tiles.retain(|t| *t != id); i.tabs.remove(&id) };
            if let Some(tab) = tab { close(tab); }
        }
        CommandMsg::Tiles(tiles) => {
            let missing: Vec<(u64, String)> = {
                let mut i = inner.borrow_mut();
                i.tiles = tiles.iter().map(|(id, _)| *id).collect();
                tiles.into_iter().filter(|(id, _)| !i.tabs.contains_key(id) && !i.creating.contains_key(id)).collect()
            };
            for (id, url) in missing { create_tab(inner, id, Some(url), None); }
            apply(inner);
        }
        CommandMsg::Follow(pairs) => follow(inner, pairs),
        CommandMsg::Stop => stop(inner),
    }
}
/// WebView2's profile and cache: machine-only, so `%LOCALAPPDATA%\ados\webview2` (`ADOS_HOME` wins).
fn data_folder() -> std::path::PathBuf {
    match std::env::var_os("LOCALAPPDATA") {
        Some(dir) if !dir.is_empty() && std::env::var_os("ADOS_HOME").is_none() => std::path::PathBuf::from(dir).join("ados").join("webview2"),
        _ => ados_core::paths::home().join("webview2"),
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

const HOST_CLASS: PCWSTR = w!("AdosBrowserPage");

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the default handling for our own plain child window.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn register_host_class() {
    // SAFETY: registering a window class with a static name; a second registration fails harmlessly.
    unsafe {
        let instance = GetModuleHandleW(None).ok().map(|h| windows::Win32::Foundation::HINSTANCE(h.0)).unwrap_or_default();
        let class = WNDCLASSW { lpfnWndProc: Some(host_proc), hInstance: instance, lpszClassName: HOST_CLASS, ..Default::default() };
        RegisterClassW(&class);
    }
}

/// A new tab: a hidden host window, then a WebView2 controller inside it. A popup passes the
/// page's request (`popup`) instead of an address; the new page goes into it.
fn create_tab(inner: &Rc<RefCell<Inner>>, id: u64, url: Option<String>, popup: Option<(ICoreWebView2NewWindowRequestedEventArgs, ICoreWebView2Deferral)>) {
    let (parent, env) = {
        let mut i = inner.borrow_mut();
        if let Some(url) = &url { i.creating.insert(id, url.clone()); }
        (i.parent, i.env.clone())
    };
    let Some(env) = env else { return };
    // SAFETY: a plain child window of the aDOs window, destroyed in `close`.
    let host = unsafe {
        CreateWindowExW(WINDOW_EX_STYLE(0), HOST_CLASS, w!(""), WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS, 0, 0, 0, 0, Some(parent), None, None, None)
    };
    let host = match host {
        Ok(host) => host,
        Err(e) => return report(inner, format!("The Browser could not make a page window: {}", e.message())),
    };
    let weak = Rc::downgrade(inner);
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |result, controller| {
        let Some(inner) = weak.upgrade() else { return Ok(()) };
        let controller = match (result, controller) {
            (Ok(()), Some(c)) => c,
            (result, _) => {
                let detail = result.err().map(|e| e.message()).unwrap_or_default();
                // SAFETY: our own host window.
                let _ = unsafe { DestroyWindow(host) };
                report(&inner, format!("The Browser could not open a page: {detail}"));
                return Ok(());
            }
        };
        let webview = unsafe { controller.CoreWebView2() }?;
        let wanted = inner.borrow_mut().creating.remove(&id);
        if inner.borrow().stopped || (wanted.is_none() && popup.is_none()) {
            // Closed (or the engine stopped) while the controller was on its way.
            close(Tab { host, controller, webview });
            return Ok(());
        }
        setup(&inner, id, &controller, &webview);
        if let Some((args, deferral)) = &popup {
            // SAFETY: completing the page's own request with the new page.
            unsafe {
                let _ = args.SetNewWindow(&webview);
                let _ = deferral.Complete();
            }
        } else if let Some(url) = wanted {
            navigate(&webview, &url);
        }
        let selected = {
            let mut i = inner.borrow_mut();
            i.tabs.insert(id, Tab { host, controller, webview });
            i.selected == id
        };
        apply(&inner);
        if selected { let _ = inner.borrow().events.try_send(Event::Ready); }
        Ok(())
    }));
    // SAFETY: `host` is a live child window; WebView2 calls the handler on this thread.
    if let Err(e) = unsafe { env.CreateCoreWebView2Controller(host, &handler) } {
        let _ = unsafe { DestroyWindow(host) };
        report(inner, format!("The Browser could not open a page: {}", e.message()));
    }
}

/// Settings, scripts and events of a new page.
fn setup(inner: &Rc<RefCell<Inner>>, id: u64, controller: &ICoreWebView2Controller, webview: &ICoreWebView2) {
    // SAFETY: COM calls on live objects, on their own thread. Each handler holds only a weak
    // link to the runtime, so a page never keeps a stopped engine alive.
    unsafe {
        if let Ok(settings) = webview.Settings() {
            let _ = settings.SetIsStatusBarEnabled(false);
            // The engine's own shortcuts (new window, find, print, DevTools) stay off; aDOs owns the keys.
            if let Ok(s3) = settings.cast::<ICoreWebView2Settings3>() { let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false); }
        }
        let (hints, leads) = { let i = inner.borrow(); (i.hints.clone(), i.follows.iter().any(|(_, l)| *l == id)) };
        add_script(webview, &crate::hints::source(&hints));
        add_script(webview, POINTER_JS);
        if leads { lead(inner, id, webview); }
        let mut token = 0i64;
        let w = Rc::downgrade(inner);
        let _ = webview.add_SourceChanged(&SourceChangedEventHandler::create(Box::new(move |_, _| { changed(&w, id); Ok(()) })), &mut token);
        let w = Rc::downgrade(inner);
        let _ = webview.add_HistoryChanged(&HistoryChangedEventHandler::create(Box::new(move |_, _| { changed(&w, id); Ok(()) })), &mut token);
        let w = Rc::downgrade(inner);
        let _ = webview.add_DocumentTitleChanged(&DocumentTitleChangedEventHandler::create(Box::new(move |_, _| { changed(&w, id); Ok(()) })), &mut token);
        let w = Rc::downgrade(inner);
        let _ = webview.add_NavigationCompleted(&NavigationCompletedEventHandler::create(Box::new(move |_, _| { changed(&w, id); Ok(()) })), &mut token);
        let w = Rc::downgrade(inner);
        let _ = webview.add_WindowCloseRequested(&WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
            if let Some(inner) = w.upgrade() { let _ = inner.borrow().events.try_send(Event::Closed(id)); }
            Ok(())
        })), &mut token);
        let w = Rc::downgrade(inner);
        let _ = webview.add_NewWindowRequested(&NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
            let (Some(inner), Some(args)) = (w.upgrade(), args) else { return Ok(()) };
            let mut uri = PWSTR::null();
            let url = if args.Uri(&mut uri).is_ok() { take_pwstr(uri) } else { String::new() };
            let deferral = args.GetDeferral()?;
            let (new_id, events) = {
                let mut i = inner.borrow_mut();
                let n = i.next_opened;
                i.next_opened += 1;
                (n, i.events.clone())
            };
            if key_down(VK_CONTROL.0) { let _ = events.try_send(Event::PagePressed { behind: true }); }
            create_tab(&inner, new_id, None, Some((args, deferral)));
            let _ = events.try_send(Event::Opened { id: new_id, opener: Some(id), url, title: String::new() });
            Ok(())
        })), &mut token);
        let w = Rc::downgrade(inner);
        let _ = webview.add_WebMessageReceived(&WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let (Some(inner), Some(args)) = (w.upgrade(), args) else { return Ok(()) };
            let mut text = PWSTR::null();
            if args.TryGetWebMessageAsString(&mut text).is_ok() { message(&inner, id, &take_pwstr(text)); }
            Ok(())
        })), &mut token);
        let w = Rc::downgrade(inner);
        let _ = controller.add_GotFocus(&FocusChangedEventHandler::create(Box::new(move |_, _| {
            let Some(inner) = w.upgrade() else { return Ok(()) };
            let (ours, events, tile) = {
                let mut i = inner.borrow_mut();
                let tile = i.keyboard != id;
                i.keyboard = id;
                (i.focusing, i.events.clone(), tile)
            };
            if !ours {
                if tile { let _ = events.try_send(Event::TilePressed(id)); }
                let _ = events.try_send(Event::PagePressed { behind: key_down(VK_CONTROL.0) });
            }
            Ok(())
        })), &mut token);
        let w = Rc::downgrade(inner);
        let _ = controller.add_AcceleratorKeyPressed(&AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let (Some(inner), Some(args)) = (w.upgrade(), args) else { return Ok(()) };
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
            let mut vk = 0u32;
            args.KeyEventKind(&mut kind)?;
            args.VirtualKey(&mut vk)?;
            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN { return Ok(()); }
            let pressed = Chord { ctrl: key_down(VK_CONTROL.0), shift: key_down(VK_SHIFT.0), alt: key_down(VK_MENU.0), vk };
            let (hit, events) = {
                let i = inner.borrow();
                let hit = i.chords.iter().position(|c| *c == Some(pressed)).map(|index| if index < i.shell_chords { Event::Shortcut(index) } else { Event::PageExit });
                (hit, i.events.clone())
            };
            if let Some(event) = hit {
                args.SetHandled(true)?;
                let _ = events.try_send(event);
            }
            Ok(())
        })), &mut token);
    }
}

/// Every page tells aDOs when the pointer comes onto it: GPUI hears nothing over a page window,
/// so aDOs ends its own hover and tooltips then (as X11's EnterNotify does on Linux).
const POINTER_JS: &str = r#"(() => { const w = window;
 if (w.__adosPointer || !w.chrome || !w.chrome.webview) return; w.__adosPointer = true; let inside = false;
 document.addEventListener('mouseover', () => { if (!inside) { inside = true; w.chrome.webview.postMessage('{"enter":true}'); } }, true);
 document.addEventListener('mouseout', e => { if (!e.relatedTarget) inside = false; }, true); })()"#;

/// A leader tile holds plain link clicks and posts them to aDOs, which opens them in its
/// follower. Same rules as Linux's `follow.rs` (no modifier, no other target, not a download,
/// not a same-page `#` jump, not handled by the page's own script); the page posts instead of
/// queueing, so nothing is polled.
const LEAD_JS: &str = r#"(() => { const w = window; w.__adosLead = true;
 if (w.__adosFollowOn || !w.chrome || !w.chrome.webview) return; w.__adosFollowOn = true;
 w.addEventListener('click', e => {
  if (!w.__adosLead || e.defaultPrevented || e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
  const a = e.target && e.target.closest && e.target.closest('a[href]');
  if (!a || a.hasAttribute('download')) return;
  const t = (a.getAttribute('target') || '').toLowerCase();
  if (t && t !== '_self' && t !== '_top' && t !== '_parent') return;
  const u = new URL(a.href, location.href);
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return;
  if (u.hash && u.href.split('#')[0] === location.href.split('#')[0]) return;
  e.preventDefault(); w.chrome.webview.postMessage(JSON.stringify({ follow: u.href })); }); })()"#;

/// New follower pairs from the panel: new leaders start holding their links, old ones stop.
fn follow(inner: &Rc<RefCell<Inner>>, pairs: Vec<(u64, u64)>) {
    let (start, stop): (Vec<(u64, ICoreWebView2)>, Vec<(ICoreWebView2, String)>) = {
        let mut i = inner.borrow_mut();
        i.follows = pairs;
        let leaders: Vec<u64> = i.follows.iter().map(|(_, l)| *l).collect();
        let gone: Vec<u64> = i.lead_scripts.keys().filter(|l| !leaders.contains(l)).copied().collect();
        let mut stop = vec![];
        for l in gone {
            let script = i.lead_scripts.remove(&l).unwrap_or_default();
            if let Some(tab) = i.tabs.get(&l) { stop.push((tab.webview.clone(), script)); }
        }
        let start = leaders.into_iter()
            .filter(|l| !i.lead_scripts.contains_key(l))
            .filter_map(|l| i.tabs.get(&l).map(|t| (l, t.webview.clone())))
            .collect();
        (start, stop)
    };
    for (webview, script) in stop {
        let script = wide(&script);
        // SAFETY: the buffer outlives the call.
        let _ = unsafe { webview.RemoveScriptToExecuteOnDocumentCreated(PCWSTR(script.as_ptr())) };
        eval(&webview, "window.__adosLead = false", |_| {});
    }
    for (id, webview) in start { lead(inner, id, &webview); }
}

/// Make tab `id` a leader: now, and on every page it loads from here on.
fn lead(inner: &Rc<RefCell<Inner>>, id: u64, webview: &ICoreWebView2) {
    if inner.borrow().lead_scripts.contains_key(&id) { return; }
    inner.borrow_mut().lead_scripts.insert(id, String::new());
    let weak = Rc::downgrade(inner);
    let done = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(move |result, script_id| {
        let Some(inner) = weak.upgrade() else { return Ok(()) };
        // Still a leader: keep the id so the script can be removed later.
        if let Some(slot) = inner.borrow_mut().lead_scripts.get_mut(&id).filter(|_| result.is_ok()) { *slot = script_id; }
        Ok(())
    }));
    let script = wide(LEAD_JS);
    // SAFETY: the buffer outlives the call.
    let _ = unsafe { webview.AddScriptToExecuteOnDocumentCreated(PCWSTR(script.as_ptr()), &done) };
    eval(webview, LEAD_JS, |_| {});
}

/// A message a page posted (`chrome.webview.postMessage`). Pages can post too, so only these
/// two shapes are read, and a followed link must be http(s).
fn message(inner: &Rc<RefCell<Inner>>, id: u64, text: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else { return };
    let events = inner.borrow().events.clone();
    if value["enter"].as_bool() == Some(true) {
        let _ = events.try_send(Event::PointerEntered);
        return;
    }
    let Some(url) = value["follow"].as_str().filter(|u| u.starts_with("http://") || u.starts_with("https://")) else { return };
    let (follower, leader) = {
        let i = inner.borrow();
        let follower = i.follows.iter().find(|(_, l)| *l == id)
            .and_then(|(f, _)| i.tabs.get(f).map(|t| (*f, t.webview.clone())));
        (follower, i.tabs.get(&id).map(|t| t.webview.clone()))
    };
    match (follower, leader) {
        (Some((follower, webview)), _) => {
            eprintln!("Browser follower tab {follower} opens {url}");
            let _ = events.try_send(Event::Followed(follower, url.to_string()));
            navigate(&webview, url);
        }
        // No follower page to open it in (closed, not made yet): the leader goes there itself.
        (None, Some(webview)) => navigate(&webview, url),
        (None, None) => {}
    }
}

fn key_down(vk: u16) -> bool {
    // SAFETY: reads this thread's keyboard state.
    unsafe { GetKeyState(i32::from(vk)) < 0 }
}

/// The page in tab `id` changed (address, title, history, load end): send a fresh snapshot.
fn changed(inner: &Weak<RefCell<Inner>>, id: u64) {
    if let Some(inner) = inner.upgrade() { snapshot(&inner, id); }
}

fn snapshot(inner: &Rc<RefCell<Inner>>, id: u64) {
    let (webview, events) = { let i = inner.borrow(); (i.tabs.get(&id).map(|t| t.webview.clone()), i.events.clone()) };
    let Some(webview) = webview else { return };
    eval(&webview, protocol::SNAPSHOT, move |result| {
        if let Ok(value) = result.and_then(|text| serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string())) {
            let _ = events.try_send(Event::Page(id, Page::from_value(&value)));
        }
    });
}

/// Show each page that has a place, at that place, minus the floating cards over it.
fn apply(inner: &Rc<RefCell<Inner>>) {
    // Collected first: a WebView2 call may run a page event, which needs the runtime.
    let (pages, holes) = {
        let i = inner.borrow();
        let shown: Vec<u64> = if i.tiles.contains(&i.selected) { i.tiles.clone() } else { vec![i.selected] };
        let pages: Vec<(HWND, ICoreWebView2Controller, Option<Rect>)> = i.tabs.iter().map(|(id, tab)| {
            let rect = shown.contains(id)
                .then(|| i.geometry.iter().find(|(g, _)| g == id || (*g == 0 && *id == i.selected)).map(|(_, r)| *r))
                .flatten()
                .filter(|r| i.visible && r.w > 0 && r.h > 0);
            (tab.host, tab.controller.clone(), rect)
        }).collect();
        (pages, i.holes.clone())
    };
    for (host, controller, rect) in pages {
        let tab = Shown { host, controller };
        // SAFETY: our own host windows and their controllers, on their own thread.
        unsafe {
            match rect {
                Some(r) if cut(tab.host, r, &holes) => {
                    let _ = SetWindowPos(tab.host, None, r.x, r.y, r.w as i32, r.h as i32, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
                    let _ = tab.controller.SetBounds(RECT { left: 0, top: 0, right: r.w as i32, bottom: r.h as i32 });
                    let _ = tab.controller.SetIsVisible(true);
                }
                _ => {
                    let _ = tab.controller.SetIsVisible(false);
                    let _ = ShowWindow(tab.host, SW_HIDE);
                }
            }
        }
    }
}

/// Cut the floating cards out of the page at `r` (window pixels). False when Windows refused
/// the region: the page then hides while the cards are open.
struct Shown { host: HWND, controller: ICoreWebView2Controller }

fn cut(host: HWND, r: Rect, holes: &Holes) -> bool {
    // SAFETY: GDI regions we own until SetWindowRgn takes the one it is given.
    unsafe {
        let inside: Vec<Rect> = holes.rects.iter().filter(|h| overlaps(**h, r)).copied().collect();
        if inside.is_empty() {
            SetWindowRgn(host, None, true);
            return true;
        }
        let region = CreateRectRgn(0, 0, r.w as i32, r.h as i32);
        if region.is_invalid() { return false; }
        for h in inside {
            let hole = CreateRectRgn(h.x - r.x, h.y - r.y, h.x - r.x + h.w as i32, h.y - r.y + h.h as i32);
            let _ = CombineRgn(Some(region), Some(region), Some(hole), RGN_DIFF);
            let _ = DeleteObject(HGDIOBJ(hole.0));
        }
        if SetWindowRgn(host, Some(region), true) == 0 {
            let _ = DeleteObject(HGDIOBJ(region.0));
            return false;
        }
        true
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w as i32 && b.x < a.x + a.w as i32 && a.y < b.y + b.h as i32 && b.y < a.y + a.h as i32
}

fn close(tab: Tab) {
    // SAFETY: closing our own controller and host window.
    unsafe {
        let _ = tab.controller.Close();
        let _ = DestroyWindow(tab.host);
    }
}

fn report(inner: &Rc<RefCell<Inner>>, error: String) {
    eprintln!("Browser error: {error}");
    let _ = inner.borrow().events.try_send(Event::Error(error));
}

fn navigate(webview: &ICoreWebView2, url: &str) {
    let url = wide(url);
    // SAFETY: the buffer outlives the call.
    let _ = unsafe { webview.Navigate(PCWSTR(url.as_ptr())) };
}

fn add_script(webview: &ICoreWebView2, script: &str) {
    let script = wide(script);
    let done = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|_, _| Ok(())));
    // SAFETY: the buffer outlives the call.
    let _ = unsafe { webview.AddScriptToExecuteOnDocumentCreated(PCWSTR(script.as_ptr()), &done) };
}

/// Run `script` in the page; the reply is its value as text (a string as itself, anything else as JSON).
fn eval(webview: &ICoreWebView2, script: &str, reply: impl FnOnce(Result<String, String>) + 'static) {
    let reply = Rc::new(RefCell::new(Some(reply)));
    let later = reply.clone();
    let handler = ExecuteScriptCompletedHandler::create(Box::new(move |result, json| {
        if let Some(reply) = later.borrow_mut().take() {
            reply(result.map_err(|e| e.message()).map(|()| reply_text(&json)));
        }
        Ok(())
    }));
    let script = wide(script);
    // SAFETY: the buffer outlives the call.
    if let Err(e) = unsafe { webview.ExecuteScript(PCWSTR(script.as_ptr()), &handler) } {
        if let Some(reply) = reply.borrow_mut().take() { reply(Err(e.message())); }
    }
}

fn reply_text(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(v) => v.to_string(),
        Err(_) => json.to_string(),
    }
}

/// The page as a base64 PNG through the DevTools protocol (the same call Chromium gets on Linux).
fn capture(webview: &ICoreWebView2, reply: impl FnOnce(Result<String, String>) + 'static) {
    let reply = Rc::new(RefCell::new(Some(reply)));
    let later = reply.clone();
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, json| {
        if let Some(reply) = later.borrow_mut().take() {
            let data = result.map_err(|e| e.message()).and_then(|()| {
                serde_json::from_str::<serde_json::Value>(&json).ok()
                    .and_then(|v| v["data"].as_str().map(str::to_string))
                    .ok_or_else(|| "WebView2 returned no image".to_string())
            });
            reply(data);
        }
        Ok(())
    }));
    let (method, params) = (wide("Page.captureScreenshot"), wide(r#"{"format":"png"}"#));
    // SAFETY: the buffers outlive the call.
    if let Err(e) = unsafe { webview.CallDevToolsProtocolMethod(PCWSTR(method.as_ptr()), PCWSTR(params.as_ptr()), &handler) } {
        if let Some(reply) = reply.borrow_mut().take() { reply(Err(e.message())); }
    }
}

/// Crop a base64 PNG of the viewport to `rect` and save it (comment targets).
fn crop_png(b64: &str, rect: Rect, path: &std::path::Path) -> Result<()> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or_else(|| anyhow!("image too large"))?];
    let info = reader.next_frame(&mut buf)?;
    let bpp = info.color_type.samples();
    let left = rect.x.max(0) as u32;
    let top = rect.y.max(0) as u32;
    let right = (rect.x + rect.w as i32).clamp(0, info.width as i32) as u32;
    let bottom = (rect.y + rect.h as i32).clamp(0, info.height as i32) as u32;
    anyhow::ensure!(right > left && bottom > top, "the target is outside the visible page");
    let mut rgb = Vec::with_capacity(((right - left) * (bottom - top) * 3) as usize);
    for y in top..bottom {
        let row = &buf[y as usize * info.line_size..];
        for x in left..right {
            let p = &row[x as usize * bpp..];
            if bpp >= 3 { rgb.extend_from_slice(&p[..3]); } else { rgb.extend_from_slice(&[p[0], p[0], p[0]]); }
        }
    }
    crate::shot::write_png(path, right - left, bottom - top, &rgb)
}

/// aDOs's chords (the shell's, then the page exits Esc and Shift+Tab) as Windows keys.
fn chords(mut keys: Vec<String>) -> (Vec<Option<Chord>>, usize) {
    let shell = keys.len();
    keys.extend(["escape".into(), "shift-tab".into()]);
    (keys.iter().map(|k| chord(k)).collect(), shell)
}

/// `ctrl-shift-k` → Ctrl+Shift+K. `super` chords never reach a page on Windows: none.
fn chord(text: &str) -> Option<Chord> {
    let (mods, key) = match text.strip_suffix("--") {
        Some(rest) => (rest, "-"),
        None => text.rsplit_once('-').unwrap_or(("", text)),
    };
    let mut c = Chord { ctrl: false, shift: false, alt: false, vk: 0 };
    for part in mods.split('-').filter(|p| !p.is_empty()) {
        match part {
            "ctrl" => c.ctrl = true,
            "shift" => c.shift = true,
            "alt" => c.alt = true,
            _ => return None,
        }
    }
    c.vk = match key {
        "space" => 0x20,
        "escape" => 0x1b,
        "tab" => 0x09,
        "enter" => 0x0d,
        "backspace" => 0x08,
        "delete" => 0x2e,
        "insert" => 0x2d,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        k if k.len() == 1 => {
            let ch = k.chars().next()?;
            if ch.is_ascii_alphanumeric() {
                u32::from(ch.to_ascii_uppercase())
            } else {
                // The key that types this character on the owner's keyboard layout.
                // SAFETY: a pure lookup in the current layout.
                let scan = unsafe { VkKeyScanW(ch as u16) };
                if scan == -1 { return None; }
                u32::from(scan as u16 & 0xff)
            }
        }
        k => {
            let n: u32 = k.strip_prefix('f')?.parse().ok()?;
            if !(1..=24).contains(&n) { return None; }
            0x70 + n - 1
        }
    };
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chords_in_windows_keys() {
        assert!(chord("ctrl-l") == Some(Chord { ctrl: true, shift: false, alt: false, vk: 'L' as u32 }));
        assert!(chord("ctrl-shift-tab") == Some(Chord { ctrl: true, shift: true, alt: false, vk: 0x09 }));
        assert!(chord("alt-space") == Some(Chord { ctrl: false, shift: false, alt: true, vk: 0x20 }));
        assert!(chord("f5") == Some(Chord { ctrl: false, shift: false, alt: false, vk: 0x74 }));
        assert!(chord("ctrl-7") == Some(Chord { ctrl: true, shift: false, alt: false, vk: '7' as u32 }));
        assert!(chord("super-k").is_none());
        let (list, shell) = chords(vec!["ctrl-k".into()]);
        assert_eq!((list.len(), shell), (3, 1));
        assert!(list[1] == Some(Chord { ctrl: false, shift: false, alt: false, vk: 0x1b }));
    }
    #[test]
    fn overlap_is_strict() {
        let page = Rect { x: 0, y: 0, w: 100, h: 100 };
        assert!(overlaps(Rect { x: 90, y: 90, w: 20, h: 20 }, page));
        assert!(!overlaps(Rect { x: 100, y: 0, w: 20, h: 20 }, page));
    }
    #[test]
    fn script_values_as_text() {
        assert_eq!(reply_text("\"hi\""), "hi");
        assert_eq!(reply_text("true"), "true");
        assert_eq!(reply_text("{\"a\":1}"), "{\"a\":1}");
    }
}
