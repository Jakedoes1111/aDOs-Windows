use crate::{
    driver::{Driver, Kind},
    engine::launch,
    protocol::{self, Cdp, Page, WebDriver},
    state::Engine,
    xembed::{Embed, Rect, Tile},
};
use anyhow::{bail, Context, Result};
use rustix::event::{poll, PollFd, PollFlags};
use std::{
    io::{Read, Write},
    net::TcpListener,
    os::unix::net::UnixStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub enum Event {
    Ready,
    /// A snapshot of the page in tab `id`.
    Page(u64, Page),
    /// A page opened a tab or window (`window.open`, `target=_blank`): it is now tab `id`.
    /// `opener` is the tab selected when it appeared; the address and title may be empty yet.
    Opened { id: u64, opener: Option<u64>, url: String, title: String },
    /// Tab `id` closed itself (`window.close()`, the end of a sign-in).
    Closed(u64),
    /// A press in tile `id` made it the tile with the keyboard (`xembed.rs`).
    TilePressed(u64),
    /// Follower tab `id` opens the address clicked in its leader.
    Followed(u64, String),
    Error(String),
    Failed(String),
    Shortcut(usize),
    /// A press in the page; `behind`: Ctrl+click or middle click, so a tab it opens waits in the background.
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
    /// Close tab `id`; a `Select` follows unless it was not the selected one.
    Close(u64),
    /// The tabs tiled with the next selected one, in order, with their addresses (sent
    /// before `Select`). Without the selected tab, it is shown alone.
    Tiles(Vec<(u64, String)>),
    /// (follower, leader) pairs.
    Follow(Vec<(u64, u64)>),
    Stop,
}
pub(crate) enum HostMsg {
    /// Show these engine windows in the page area (one, or every tile), each cropped by the
    /// engine's toolbar height; the tab given owns the keyboard.
    Show(Vec<Tile>, u64),
    /// Take this engine window into aDOs without showing it.
    Adopt(u32),
    /// Where each shown tab's page goes; tab 0 is the one page when nothing is tiled.
    Geometry(Vec<(u64, Rect)>),
    Visible(bool),
    FocusAddress(mpsc::SyncSender<()>),
    /// The user's keymap changed: replace the native grabs.
    Shortcuts(Vec<String>),
    ShortcutDone,
    FocusPage,
    /// A plain key GPUI gave the page proxy before the page held X focus (keysym).
    Typed(u32),
    Repaint,
    Overlay(bool, mpsc::SyncSender<()>),
    /// The floating card's rectangle in page-window pixels, cut out of the page.
    Cutout(crate::cutout::Holes),
    /// Crop of the page window saved as PNG at the path (comment targets, `shot.rs`).
    Shot(Rect, u32, std::path::PathBuf, async_channel::Sender<Result<std::path::PathBuf, String>>),
    Stop,
}

// A pipe wakes poll immediately, so there is no idle timer and X11 never waits on HTTP.
pub(crate) struct Sender<T> {
    tx: mpsc::Sender<T>,
    wake: Arc<Mutex<UnixStream>>,
}
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            wake: self.wake.clone(),
        }
    }
}
impl<T> Sender<T> {
    pub(crate) fn send(&self, msg: T) {
        if self.tx.send(msg).is_ok() {
            if let Ok(mut wake) = self.wake.lock() {
                let _ = wake.write(&[1]);
            }
        }
    }
}
fn pipe<T>() -> Result<(Sender<T>, mpsc::Receiver<T>, UnixStream)> {
    let (read, write) = UnixStream::pair()?;
    read.set_nonblocking(true)?;
    write.set_nonblocking(true)?;
    let (tx, rx) = mpsc::channel();
    Ok((
        Sender {
            tx,
            wake: Arc::new(Mutex::new(write)),
        },
        rx,
        read,
    ))
}
fn drain_wake(wake: &mut UnixStream) {
    let mut buf = [0; 1024];
    while wake.read(&mut buf).is_ok_and(|n| n > 0) {}
}
fn report(tx: &async_channel::Sender<Event>, error: impl std::fmt::Display) {
    let error = error.to_string();
    eprintln!("Browser error: {error}");
    let _ = tx.try_send(Event::Error(plain(&error)));
}
/// `poll`, where a signal (the engine's SIGCHLD, any child's) is not an error: `None`, look again.
fn wait(fds: &mut [PollFd<'_>], timeout: Option<&rustix::event::Timespec>) -> Result<Option<usize>> {
    match poll(fds, timeout) {
        Ok(n) => Ok(Some(n)),
        Err(rustix::io::Errno::INTR) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
/// The engine or the host stopped for good: the detail goes to the log, the owner reads a sentence.
fn failed(tx: &async_channel::Sender<Event>, error: impl std::fmt::Display) {
    eprintln!("Browser stopped: {error}");
    let _ = tx.try_send(Event::Failed(STOPPED.into()));
}
pub(crate) const STOPPED: &str = "The browser engine stopped. Retry starts it again.";
const SLOW: &str = "The page took too long to load.";
/// An engine error as a plain sentence for the panel (network library wording stays in the log).
pub(crate) fn plain(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        SLOW.into()
    } else if lower.contains("connection refused") || lower.contains("connection reset") || lower.contains("broken pipe") {
        STOPPED.into()
    } else {
        error.to_string()
    }
}
fn is_timeout(error: &anyhow::Error) -> bool { plain(&error.to_string()) == SLOW }

pub struct Runtime {
    driver: Sender<CommandMsg>,
    host: Sender<HostMsg>,
    stopping: Arc<AtomicBool>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}
impl Runtime {
    pub fn start(
        parent: u32,
        engine: Engine,
        tab: u64,
        url: String,
        shortcuts: Vec<String>,
        hints: serde_json::Value,
    ) -> Result<(Arc<Self>, async_channel::Receiver<Event>)> {
        let (events, receiver) = async_channel::unbounded();
        let (driver, driver_rx, driver_wake) = pipe()?;
        let (host, host_rx, host_wake) = pipe()?;
        let stopping = Arc::new(AtomicBool::new(false));
        let host_thread = {
            let driver = driver.clone();
            let events = events.clone();
            let stopping = stopping.clone();
            thread::Builder::new()
                .name("ados-browser-x11".into())
                .spawn(move || {
                    if let Err(e) =
                        host_loop(parent, shortcuts, host_rx, host_wake, &driver, &events, &stopping)
                    {
                        driver.send(CommandMsg::Stop);
                        failed(&events, e);
                    }
                })?
        };
        let driver_thread = {
            let host = host.clone();
            let stopping = stopping.clone();
            thread::Builder::new()
                .name("ados-browser-engine".into())
                .spawn(move || {
                    if let Err(e) = driver_loop(
                        parent,
                        engine,
                        tab,
                        url,
                        hints,
                        driver_rx,
                        driver_wake,
                        &host,
                        &events,
                        &stopping,
                    ) {
                        failed(&events, e);
                    }
                    host.send(HostMsg::Stop);
                })?
        };
        Ok((
            Arc::new(Self {
                driver,
                host,
                stopping,
                threads: Mutex::new(vec![driver_thread, host_thread]),
            }),
            receiver,
        ))
    }
    pub fn command(&self, msg: CommandMsg) {
        self.driver.send(msg);
    }
    pub fn geometry(&self, rects: Vec<(u64, Rect)>) {
        self.host.send(HostMsg::Geometry(rects));
    }
    pub fn visible(&self, visible: bool) {
        self.host.send(HostMsg::Visible(visible));
    }
    pub fn shortcut_done(&self) { self.host.send(HostMsg::ShortcutDone); }
    pub fn shortcuts(&self, keys: Vec<String>) { self.host.send(HostMsg::Shortcuts(keys)); }
    pub fn typed(&self, keysym: u32) { self.host.send(HostMsg::Typed(keysym)); }
    pub fn focus_page(&self) {
        self.host.send(HostMsg::FocusPage);
    }
    pub fn focus_address(&self) {
        let (tx, rx) = mpsc::sync_channel(0);
        self.host.send(HostMsg::FocusAddress(tx));
        let _ = rx.recv_timeout(Duration::from_millis(500));
    }
    pub fn overlay(&self, open: bool) {
        let (tx, rx) = mpsc::sync_channel(0);
        self.host.send(HostMsg::Overlay(open, tx));
        // The native focus transition must precede GPUI input focus. A dead host
        // cannot stall the UI indefinitely.
        let _ = rx.recv_timeout(Duration::from_millis(500));
    }
    pub fn cutout(&self, hole: crate::cutout::Holes) {
        self.host.send(HostMsg::Cutout(hole));
    }
    /// Save `rect` of the page window (window pixels) as a PNG at `path`.
    pub fn shot(&self, rect: Rect, viewport_h: u32, path: std::path::PathBuf) -> async_channel::Receiver<Result<std::path::PathBuf, String>> {
        let (tx, rx) = async_channel::bounded(1);
        self.host.send(HostMsg::Shot(rect, viewport_h, path, tx));
        rx
    }
    pub fn repaint(&self) {
        self.host.send(HostMsg::Repaint);
    }
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        self.host.send(HostMsg::Stop);
        self.driver.send(CommandMsg::Stop);
        if let Ok(mut threads) = self.threads.lock() {
            for t in threads.drain(..) {
                let _ = t.join();
            }
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop();
    }
}

fn host_loop(
    parent: u32,
    shortcuts: Vec<String>,
    rx: mpsc::Receiver<HostMsg>,
    mut wake: UnixStream,
    driver: &Sender<CommandMsg>,
    events: &async_channel::Sender<Event>,
    stopping: &AtomicBool,
) -> Result<()> {
    let mut embed = Embed::new(parent)?;
    // Native-only exits: do not reserve these keys in other modules.
    let mut shell_shortcuts = shortcuts.len();
    let with_exits = |mut keys: Vec<String>| {
        keys.extend(["escape".into(), "shift-tab".into()]);
        keys
    };
    embed.shortcuts(&with_exits(shortcuts))?;
    let mut repaint_at: Option<Instant> = None;
    while !stopping.load(Ordering::Acquire) {
        drain_wake(&mut wake);
        while let Ok(msg) = rx.try_recv() {
            match msg {
                HostMsg::Show(tiles, active) => {
                    embed.show(&tiles, active)?;
                    let _ = events.try_send(Event::Ready);
                }
                HostMsg::Adopt(window) => {
                    if let Err(e) = embed.adopt(window) { report(events, e); }
                }
                HostMsg::Geometry(rect) => embed.geometry(rect)?,
                HostMsg::Visible(visible) => embed.visibility(visible)?,
                HostMsg::FocusAddress(done) => {
                    embed.cancel_focus_page()?;
                    embed.focus_parent()?;
                    let _ = done.send(());
                }
                HostMsg::ShortcutDone => embed.shortcut_done()?,
                HostMsg::Shortcuts(keys) => {
                    shell_shortcuts = keys.len();
                    embed.shortcuts(&with_exits(keys))?;
                }
                HostMsg::Overlay(open, done) => {
                    embed.overlay(open)?;
                    let _ = done.send(());
                },
                HostMsg::Cutout(hole) => embed.cutout(hole)?,
                HostMsg::Shot(rect, viewport_h, path, reply) => match embed.grab(rect, viewport_h) {
                    // Encoding runs off the X thread.
                    Ok((w, h, rgb)) => { let _ = thread::Builder::new().name("ados-browser-shot".into()).spawn(move || {
                        let _ = reply.try_send(crate::shot::write_png(&path, w, h, &rgb).map(|()| path).map_err(|e| e.to_string()));
                    }); }
                    Err(e) => { let _ = reply.try_send(Err(e.to_string())); }
                },
                HostMsg::FocusPage => embed.focus_page()?,
                HostMsg::Typed(keysym) => embed.typed(keysym)?,
                HostMsg::Repaint => repaint_at = Some(Instant::now() + Duration::from_millis(40)),
                HostMsg::Stop => {
                    embed.visibility(false)?;
                    return Ok(());
                }
            }
        }
        if repaint_at.is_some_and(|at| at <= Instant::now()) {
            embed.repaint()?;
            repaint_at = None;
        }
        embed.try_pending_focus()?;
        embed.replay_pointer()?;
        embed.flush_stale_typed()?;
        // Read X events last, right before sleeping: a round trip (repaint, focus, the
        // messages above) can pull keys into the connection's buffer, where `poll` cannot
        // see them. They then sat there until the next wake, so hint labels typed into the
        // focus proxy reached aDOs only after hint mode had ended, landing in the note.
        let (changed, shortcuts) = match embed.events() {
            Ok(events) => events,
            Err(error) => {
                driver.send(CommandMsg::Stop);
                return Err(error);
            }
        };
        if embed.take_pointer_enter() {
            let _ = events.try_send(Event::PointerEntered);
        }
        if let Some(tab) = embed.take_pressed_tile() {
            let _ = events.try_send(Event::TilePressed(tab));
        }
        if let Some(behind) = embed.take_page_press() {
            let _ = events.try_send(Event::PagePressed { behind });
        }
        if changed {
            driver.send(CommandMsg::Snapshot);
        }
        for index in shortcuts {
            let _ = events.try_send(if index < shell_shortcuts {
                Event::Shortcut(index)
            } else {
                Event::PageExit
            });
        }
        // Only while a page-focus request waits for held keys: re-check soon. Idle stays blocking.
        let focus_retry = embed.focus_pending().then(|| Instant::now() + Duration::from_millis(15));
        let timeout = [repaint_at, focus_retry, embed.replay_due(), embed.typed_due()]
            .into_iter()
            .flatten()
            .min()
            .map(|at| at.saturating_duration_since(Instant::now()))
            .map(|d| rustix::event::Timespec {
                tv_sec: d.as_secs() as i64,
                tv_nsec: i64::from(d.subsec_nanos()),
            });
        let mut fds = [
            PollFd::new(&wake, PollFlags::IN),
            PollFd::new(embed.conn.stream(), PollFlags::IN),
        ];
        wait(&mut fds, timeout.as_ref())?;
    }
    let _ = embed.visibility(false);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn driver_loop(
    parent: u32,
    engine: Engine,
    tab: u64,
    url: String,
    hints: serde_json::Value,
    rx: mpsc::Receiver<CommandMsg>,
    mut wake: UnixStream,
    host: &Sender<HostMsg>,
    events: &async_channel::Sender<Event>,
    stopping: &AtomicBool,
) -> Result<()> {
    // The port is selected by the OS. Only a dedicated disposable browser profile is used.
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let marker = format!("ados-browser-{}", std::process::id());
    let mut process = launch(engine, &url, port, &marker, &hints)?;
    let finder = Embed::new(parent)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let agent = protocol::http_agent();
    let mut driver = None;
    let mut child = None;
    let mut last_clip = None;
    let mut last_error = String::from("the browser window did not become ready");
    while (driver.is_none() || child.is_none()) && Instant::now() < deadline {
        if stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        if process.0.try_wait()?.is_some() {
            bail!("Browser exited during startup; see ADOS_HOME/browser/engine.log");
        }
        if child.is_none() && (engine == Engine::Chromium || driver.is_some()) {
            if let Some(window) = finder.find_window(process.0.id(), &marker)? {
                // WebDriver's initial about:blank viewport can precede Servo's
                // toolbar layout. Never freeze that transient zero crop forever.
                let clip_top = match driver.as_ref() {
                    Some(Kind::Servo { wd, .. }) => {
                        let clip = wd.viewport_height().and_then(|height| finder.chrome_height(window, height));
                        match clip {
                            Ok(clip) if clip > 0 && last_clip == Some((window, clip)) => clip,
                            Ok(clip) => { last_clip = Some((window, clip)); thread::sleep(Duration::from_millis(100)); continue; }
                            Err(error) => { last_error = error.to_string(); thread::sleep(Duration::from_millis(100)); continue; }
                        }
                    }
                    _ => 0,
                };
                if engine == Engine::Servo {
                    finder.close_other_windows(process.0.id(), window)?;
                }
                eprintln!("Browser attached window {window:#x}, chrome crop {clip_top}px");
                host.send(HostMsg::Show(vec![Tile { tab, window, clip_top }], tab));
                child = Some((window, clip_top));
            }
        }
        if driver.is_none() {
            let connected = match engine {
                Engine::Chromium => Cdp::connect(port, &agent, None)
                    .and_then(|mut cdp| cdp.add_script(&crate::hints::source(&hints)).map(|_| cdp))
                    .map(|cdp| Kind::Cdp { pages: vec![cdp], port, agent: agent.clone(), hints: crate::hints::source(&hints) }),
                Engine::Servo => WebDriver::connect(port, &agent, &marker).map(|wd| Kind::Servo {
                    wd, next_snapshot: None, hints: None,
                }),
            };
            match connected {
                Ok(connected) => driver = Some(connected),
                Err(error) => last_error = error.to_string(),
            }
        }
        if driver.is_none() || child.is_none() {
            thread::sleep(Duration::from_millis(100));
        }
    }
    let Some((window, clip_top)) = child else {
        bail!("{last_error}; use Retry or see browser/engine.log");
    };
    let kind = driver.context("Browser control endpoint did not become ready")?;
    // Chromium windows carry the class marker; Servo's are found by process id.
    let class = if engine == Engine::Chromium { marker.clone() } else { String::new() };
    let mut driver = Driver::new(kind, tab, window, clip_top, finder, process.0.id(), class)?;
    if let Err(e) = driver.command(CommandMsg::Navigate(url), events, host) { report(events, e); }
    while !stopping.load(Ordering::Acquire) {
        drain_wake(&mut wake);
        // The engine died (crash, killed): stop, so the panel can start it again.
        if let Some(status) = process.0.try_wait()? { bail!("the engine exited ({status})"); }
        // Coalesce property-change bursts from Servo into a single snapshot.
        let mut snapshot = false;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, CommandMsg::Stop) {
                return Ok(());
            }
            if matches!(msg, CommandMsg::Snapshot) {
                snapshot = true;
                continue;
            }
            if let Err(e) = driver.command(msg, events, host) {
                report(events, e);
            }
        }
        if snapshot { snapshot_now(&mut driver, events, host); }
        if let Err(e) = driver.tick(events) { report(events, e); }
        let retry = driver.retry_due().into_iter().chain(driver.timer_due()).min();
        let timespec = |d: Duration| rustix::event::Timespec { tv_sec: d.as_secs() as i64, tv_nsec: i64::from(d.subsec_nanos()) };
        if let Kind::Cdp { .. } = &driver.kind {
            driver.drain_cdp(events, host)?;
        }
        match &mut driver.kind {
            Kind::Cdp { pages, .. } => {
                let timeout = pages.iter().filter_map(|p| p.timeout()).chain(retry).min().map(timespec);
                let mut fds = vec![PollFd::new(&wake, PollFlags::IN)];
                fds.extend(pages.iter().map(|p| PollFd::new(p.socket.get_ref(), PollFlags::IN)));
                wait(&mut fds, timeout.as_ref())?;
            }
            Kind::Servo { next_snapshot, .. } => {
                let timeout = next_snapshot
                    .map(|next| next.saturating_duration_since(Instant::now()))
                    .into_iter()
                    .chain(retry)
                    .min()
                    .map(timespec);
                if wait(&mut [PollFd::new(&wake, PollFlags::IN)], timeout.as_ref())? == Some(0) {
                    snapshot_now(&mut driver, events, host);
                }
            }
        }
    }
    Ok(())
}

/// Read the page. A busy page that does not answer in time is read again later (it is not an
/// error the owner has to act on); other errors are reported.
fn snapshot_now(driver: &mut Driver, events: &async_channel::Sender<Event>, host: &Sender<HostMsg>) {
    match driver.command(CommandMsg::Snapshot, events, host) {
        Err(e) if is_timeout(&e) => {
            eprintln!("Browser snapshot: {e}");
            if let Kind::Servo { next_snapshot, .. } = &mut driver.kind { *next_snapshot = Some(Instant::now() + Duration::from_secs(3)); }
        }
        Err(e) => report(events, e),
        Ok(()) => {}
    }
}

#[cfg(test)]
mod plain_tests {
    use super::*;
    #[test]
    fn engine_errors_read_as_sentences() {
        assert_eq!(plain("timeout: global"), SLOW);
        assert_eq!(plain("io: Connection refused (os error 111)"), STOPPED);
        assert_eq!(plain("Up to 4 tiles."), "Up to 4 tiles.");
    }
}
