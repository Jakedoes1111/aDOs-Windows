mod park;
mod links;

use crate::{input, pty::Backend, terminal::TerminalElement};
use ados_core::{
    hub::{ContextHub, Snapshot},
    ids,
};
use alacritty_terminal::{
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point as TermPoint, Side},
    selection::{Selection, SelectionType},
    term::TermMode,
    vte::ansi::{ClearMode, Handler},
};
use gpui_kit::component::{
    dock::{BasePanel, Panel, PanelEvent},
    menu::ContextMenuExt,
    v_flex, ActiveTheme,
};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct TerminalPanel {
    pub(crate) focus: FocusHandle,
    pub(crate) backend: Option<Backend>,
    pub(crate) error: Option<String>,
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) cell_size: Size<Pixels>,
    pub(crate) font_size: f32,
    receiver_task: Option<Task<()>>,
    snapshot_task: Option<Task<()>>,
    last_publish: Instant,
    last_text: String,
    history_path: Option<PathBuf>,
    history_write: Arc<parking_lot::Mutex<u64>>,
    frame_pending: bool,
    selecting: bool,
    scroll_pixels: f32,
    copy_generation: u64,
    copied: bool,
    /// Out of every layout: the shell stops when this timer fires, unless a job runs (`park.rs`).
    park_task: Option<Task<()>>,
    /// The transcript of a stopped shell, restored when the panel is back in a layout.
    parked: Option<String>,
}
impl TerminalPanel {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let history_path = ados_core::paths::home().join("terminal-history.txt");
        let (history, error) = match std::fs::read_to_string(&history_path) {
            Ok(text) => (text, None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
            Err(e) => (
                String::new(),
                Some(format!("Could not restore terminal history: {e}")),
            ),
        };
        let mut panel = Self {
            focus: cx.focus_handle(),
            backend: None,
            error,
            bounds: Bounds::default(),
            cell_size: size(px(9.), px(20.)),
            font_size: std::fs::read_to_string(ados_core::paths::home().join("state/terminal-font.txt"))
                .ok().and_then(|s| s.trim().parse::<f32>().ok()).filter(|v| v.is_finite()).unwrap_or(14.).clamp(8., 32.),
            receiver_task: None,
            snapshot_task: None,
            last_publish: Instant::now() - Duration::from_secs(1),
            last_text: String::new(),
            history_path: Some(history_path),
            history_write: Arc::default(),
            frame_pending: false,
            selecting: false,
            scroll_pixels: 0.,
            copy_generation: 0,
            copied: false,
            park_task: None,
            parked: None,
        };
        let storage_error = panel.error.take();
        panel.start(&history, window, cx);
        if panel.error.is_none() {
            panel.error = storage_error;
        }
        crate::set_target(cx.entity().downgrade(), cx);
        let owner = cx.entity_id();
        cx.on_release(move |_, cx| {
            // Layout loading constructs the replacement before the old panel is released.
            // The old instance must not erase the replacement's context or active module.
            if cx
                .try_global::<crate::Target>()
                .is_some_and(|target| target.0.entity_id() == owner)
            {
                ContextHub::remove(ids::TERMINAL, cx);
            }
        })
        .detach();
        panel
    }
    fn start(&mut self, history: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.receiver_task.take();
        self.backend.take();
        match Backend::new(history) {
            Ok((backend, receiver)) => {
                self.backend = Some(backend);
                self.error = None;
                self.receiver_task = Some(cx.spawn_in(window, async move |weak, cx| {
                    while receiver.recv().await.is_ok() {
                        if weak
                            .update_in(cx, |panel, window, cx| {
                                panel.changed(window, cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
            }
            Err(e) => {
                self.error = Some(format!(
                    "Could not start the terminal: {e}. Press Enter to retry."
                ))
            }
        }
    }
    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.frame_pending {
            self.frame_pending = true;
            cx.on_next_frame(window, |panel, _, cx| {
                panel.frame_pending = false;
                cx.notify();
            });
        }
        if self.snapshot_task.is_none() {
            let delay = Duration::from_secs(1).saturating_sub(self.last_publish.elapsed());
            self.snapshot_task = Some(cx.spawn(async move |weak, cx| {
                cx.background_executor().timer(delay).await;
                let _ = weak.update(cx, |panel, cx| {
                    panel.snapshot_task = None;
                    panel.publish(cx);
                });
            }));
        }
    }
    fn publish(&mut self, cx: &mut Context<Self>) {
        self.last_publish = Instant::now();
        let Some(backend) = &self.backend else {
            return;
        };
        let text = backend.snapshot(200);
        if text == self.last_text {
            return;
        }
        self.last_text = text.clone();
        let selection = backend.term.lock().selection_to_string().map(Into::into);
        ContextHub::publish(
            Snapshot {
                module: ids::TERMINAL.into(),
                title: "Terminal".into(),
                location: std::env::home_dir().map(|h| h.display().to_string().into()),
                text: text.clone().into(),
                selection,
            },
            cx,
        );
        if !backend.term.lock().mode().contains(TermMode::ALT_SCREEN) {
            if let Some(path) = self.history_path.clone() {
                // Persist logical lines so a narrower or wider panel reflows instead
                // of keeping the hard wraps of the width it was written at.
                let history = backend.history(2000);
                let history_write = self.history_write.clone();
                let generation = {
                    let mut generation = history_write.lock();
                    *generation += 1;
                    *generation
                };
                let write = cx.background_executor().spawn(async move {
                    let current = history_write.lock();
                    if *current != generation {
                        return Ok(());
                    }
                    save_history(&path, &history)
                });
                cx.spawn(async move |weak, cx| {
                    if let Err(error) = write.await {
                        let _ = weak.update(cx, |panel, cx| {
                            panel.error = Some(format!("Could not save terminal history: {error}"));
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
        }
    }
    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        if let Some(backend) = &self.backend {
            let mut term = backend.term.lock();
            term.clear_screen(ClearMode::Saved);
            term.clear_screen(ClearMode::All);
            term.selection = None;
            term.scroll_display(Scroll::Bottom);
        }
        // Let the interactive shell redraw its prompt without sending a destructive command.
        self.send(vec![0x0c], cx);
        cx.notify();
    }
    fn send(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if let Some(backend) = &self.backend {
            if let Err(e) = backend.write(bytes) {
                self.error = Some(format!("Could not send terminal input: {e}"));
                cx.notify();
            }
        }
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.modifiers.control && matches!(key.key.as_str(), "=" | "+" | "-" | "0") {
            self.font_size = match key.key.as_str() {
                "=" | "+" => (self.font_size + 1.).min(32.),
                "-" => (self.font_size - 1.).max(8.),
                _ => 14.,
            };
            let dir = ados_core::paths::home().join("state");
            if let Err(error) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join("terminal-font.txt"), self.font_size.to_string())) {
                self.error = Some(format!("Could not save terminal font size: {error}"));
            }
            cx.notify();
        } else if key.modifiers.control && key.modifiers.shift && key.key.eq_ignore_ascii_case("v")
        {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                let bracketed = self
                    .backend
                    .as_ref()
                    .is_some_and(|b| b.term.lock().mode().contains(TermMode::BRACKETED_PASTE));
                self.send(input::paste(&text, bracketed), cx);
            }
        } else if key.key == "enter" && self.backend.as_ref().is_none_or(Backend::exited) {
            self.start("", window, cx);
            cx.notify();
        } else if key.modifiers.shift && matches!(key.key.as_str(), "pageup" | "pagedown") {
            if let Some(backend) = &self.backend {
                backend.term.lock().scroll_display(if key.key == "pageup" {
                    Scroll::PageUp
                } else {
                    Scroll::PageDown
                });
                cx.notify();
            }
        } else if let Some(backend) = &self.backend {
            let bytes = input::encode(key, *backend.term.lock().mode());
            if !bytes.is_empty() {
                {
                    let mut term = backend.term.lock();
                    term.scroll_display(Scroll::Bottom);
                    term.selection = None;
                }
                self.send(bytes, cx);
                cx.notify();
            }
        }
        cx.stop_propagation();
    }
    /// Copy the selection to the clipboard and confirm it briefly, for both the
    /// Ctrl+Shift+C binding and the terminal's menu.
    fn copy_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self
            .backend
            .as_ref()
            .and_then(|backend| backend.term.lock().selection_to_string())
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if let Some(backend) = &self.backend {
            backend.term.lock().selection = None;
        }
        self.selecting = false;
        cx.notify();
        self.copy_generation += 1;
        let generation = self.copy_generation;
        self.copied = true;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update_in(cx, |panel, _, cx| {
                if panel.copy_generation == generation { panel.copied = false; cx.notify(); }
            });
        })
        .detach();
    }
    fn position(&self, position: Point<Pixels>) -> (TermPoint, Side) {
        let Some(backend) = &self.backend else {
            return (TermPoint::new(Line(0), Column(0)), Side::Left);
        };
        let term = backend.term.lock();
        let x = ((position.x - self.bounds.origin.x) / self.cell_size.width).max(0.);
        let y = ((position.y - self.bounds.origin.y) / self.cell_size.height).max(0.);
        let row = (y.floor() as usize).min(term.screen_lines() - 1) as i32
            - term.grid().display_offset() as i32;
        let col = (x.floor() as usize).min(term.columns() - 1);
        (
            TermPoint::new(Line(row), Column(col)),
            if x.fract() < 0.5 {
                Side::Left
            } else {
                Side::Right
            },
        )
    }
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        ContextHub::set_active(ids::TERMINAL, cx);
        if self.ctrl_click(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        let (point, side) = self.position(event.position);
        if let Some(backend) = &self.backend {
            let kind = match event.click_count {
                2 => SelectionType::Semantic,
                3.. => SelectionType::Lines,
                _ => SelectionType::Simple,
            };
            backend.term.lock().selection = Some(Selection::new(kind, point, side));
            self.selecting = true;
            cx.notify();
        }
        cx.stop_propagation();
    }
    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (point, side) = self.position(event.position);
        if let Some(backend) = &self.backend {
            if let Some(selection) = &mut backend.term.lock().selection {
                selection.update(point, side);
            }
            cx.notify();
        }
    }
    fn scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_pixels += f32::from(event.delta.pixel_delta(self.cell_size.height).y);
        let lines = (self.scroll_pixels / f32::from(self.cell_size.height)).trunc() as i32;
        self.scroll_pixels -= lines as f32 * f32::from(self.cell_size.height);
        if lines == 0 {
            return;
        }
        if let Some(backend) = &self.backend {
            let mode = *backend.term.lock().mode();
            if mode.contains(TermMode::ALT_SCREEN) && !event.modifiers.shift {
                let key = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
                self.send(key.repeat(lines.unsigned_abs().min(100) as usize), cx);
            } else {
                backend.term.lock().scroll_display(Scroll::Delta(lines));
            }
            cx.notify();
        }
        cx.stop_propagation();
    }
}
impl Drop for TerminalPanel {
    fn drop(&mut self) {
        // Flush the last partial second on orderly close. Never persist an alternate
        // screen (an editor's unsaved buffer is not a shell session).
        if let (Some(backend), Some(path)) = (&self.backend, &self.history_path) {
            if !backend.term.lock().mode().contains(TermMode::ALT_SCREEN) {
                let mut generation = self.history_write.lock();
                *generation += 1;
                if let Err(error) = save_history(path, &backend.history(2000)) {
                    eprintln!("Could not save terminal history on close: {error}");
                }
            }
        }
    }
}
fn save_history(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let temp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temp)?;
    // Terminal output can contain credentials. Also tighten pre-existing temp files.
    // (Windows: the data folder is under the owner's profile, which only they can open.)
    #[cfg(unix)]
    file.set_permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600))?;
    file.write_all(text.as_bytes())?;
    std::fs::rename(temp, path)
}
impl Focusable for TerminalPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl EventEmitter<PanelEvent> for TerminalPanel {}
impl BasePanel for TerminalPanel {
    fn panel_name(&self) -> &'static str {
        ids::TERMINAL
    }
}
impl Panel for TerminalPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some("Terminal".into())
    }
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Terminal"
    }
}
impl TerminalPanel {
    pub fn focus_primary(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let exited = self.backend.as_ref().is_some_and(Backend::exited);
        let status = self
            .error
            .clone()
            .or_else(|| exited.then(|| "Shell exited. Press Enter to restart.".into()));
        let border = if self.focus.is_focused(window) {
            cx.theme().ring
        } else {
            cx.theme().border
        };
        let menu_focus = self.focus.clone();
        v_flex()
            .id("terminal-panel")
            .relative()
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .border_1()
            .border_color(border)
            .rounded_lg()
            .p_1()
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::key))
            .on_action(cx.listener(|panel, _: &crate::OpenTerminalLink, window, cx| panel.open_link(window, cx)))
            .on_action(
                cx.listener(|panel, _: &crate::CopyTerminal, window, cx| {
                    panel.copy_selection(window, cx)
                }),
            )
            .capture_any_mouse_down(cx.listener(|s, _, w, cx| s.focus_primary(w, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|panel, _, _, _| panel.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|panel, _, _, _| panel.selecting = false),
            )
            .on_scroll(cx.listener(Self::scroll))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(TerminalElement { panel: cx.entity() }),
            )
            .children(status.map(|text| {
                div()
                    .text_sm()
                    .p_2()
                    .text_color(cx.theme().muted_foreground)
                    .child(text)
            }))
            .when(self.copied, |view| view.child(
                div().absolute().top_2().right_2().px_2().py_1()
                    .rounded_md().border_1().border_color(cx.theme().border)
                    .bg(cx.theme().background).text_color(cx.theme().foreground)
                    .text_sm().child("Copied")
            ))
            .context_menu(move |menu, _, _| {
                menu.action_context(menu_focus.clone())
                    .menu("Copy", Box::new(crate::CopyTerminal))
            })
    }
}
