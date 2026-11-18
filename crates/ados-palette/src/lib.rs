//! Floating command palette and lossless multiline quick capture.
mod icons;
mod ranking;
mod state;
mod ui;

use ados_core::{actions::{ActionRegistry, PaletteAction}, capture, overlay::OverlayGuard};
use gpui_kit::*;
use gpui_kit::component::{input::{InputEvent, TextareaState}, notification::Notification, WindowExt};
use state::SavedState;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode { Actions, Capture }

pub struct Palette {
    pub(crate) input: Entity<TextareaState>,
    pub(crate) open: bool,
    pub(crate) mode: Mode,
    previous_focus: Option<FocusHandle>,
    saved: SavedState,
    pub(crate) results: Vec<PaletteAction>,
    pub(crate) selected: Option<usize>,
    pub(crate) first_row: usize,
    pub(crate) visible_rows: usize,
    pub(crate) wheel_remainder: f32,
    pub(crate) error: Option<String>,
    // Ignore stale asynchronous picker responses after close/reopen.
    generation: u64,
    // Shared guard for file/image pickers; true while any picker is in flight.
    picker_in_flight: bool,
    capture_toast_generation: u64,
    _subscriptions: Vec<Subscription>,
}

struct CaptureToast;

actions!(ados_palette, [Next, Previous, Thought, File, Image, Link]);

/// Only palette-scoped keys; the shell decides how to open this entity.
pub fn init(cx: &mut App) {
    use ados_core::shortcuts::bind;
    bind(cx, "palette.next", Next);
    bind(cx, "palette.previous", Previous);
    bind(cx, "palette.thought", Thought);
    bind(cx, "palette.file", File);
    bind(cx, "palette.image", Image);
    bind(cx, "palette.link", Link);
}

impl Palette {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx: &mut Context<Self>| {
            let (saved, error) = match SavedState::load() {
                Ok(saved) => (saved, None),
                Err(e) => (SavedState::default(), Some(format!("Could not restore palette: {e}"))),
            };
            let input = cx.new(|cx| TextareaState::new(window, cx)
                .auto_grow(1, 6).submit_on_enter(true)
                .placeholder("Search or capture…"));
            let subscriptions = vec![
                cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                    InputEvent::Change => {
                        if this.mode == Mode::Capture {
                            this.saved.capture_draft = this.input.read(cx).value().to_string();
                            this.persist();
                        }
                        this.refresh(cx);
                    }
                    InputEvent::PressEnter { secondary: false, shift: false } if this.open => this.submit(window, cx),
                    _ => {}
                }),
                cx.observe_global::<ActionRegistry>(|this, cx| this.refresh(cx)),
                cx.on_release(|this, cx| {
                    if this.open { OverlayGuard::close_cut(cx); }
                }),
            ];
            Self { input, open: false, mode: Mode::Actions, previous_focus: None,
                saved, results: vec![], selected: None, first_row: 0, visible_rows: 8, wheel_remainder: 0.0,
                error, generation: 0, picker_in_flight: false, capture_toast_generation: 0, _subscriptions: subscriptions }
        })
    }

    pub fn is_open(&self) -> bool { self.open }

    pub fn open_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_mode(Mode::Actions, window, cx);
    }

    pub fn open_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_mode(Mode::Capture, window, cx);
    }

    fn open_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            self.previous_focus = window.focused(cx);
            self.open = true;
            self.generation = self.generation.wrapping_add(1);
            OverlayGuard::open_cut(cx);
        }
        self.set_mode(mode, window, cx);
        self.focus_input(window, cx);
        self.refresh(cx);
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open { return; }
        self.open = false;
        self.generation = self.generation.wrapping_add(1);
        OverlayGuard::close_cut(cx);
        if let Some(focus) = self.previous_focus.take() { window.focus(&focus, cx); }
        else { window.blur(cx); }
        cx.notify();
    }

    pub(crate) fn focus_input(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.input.focus_handle(cx), cx);
    }

    fn persist(&mut self) {
        if let Err(e) = self.saved.save() {
            self.error = Some(format!("Could not save palette state: {e}"));
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value();
        self.results = ranking::ordered(ActionRegistry::all(cx), &query, &self.saved.recent);
        add_open_typed(&mut self.results, &query);
        // A typed query selects its best match; empty recency stays non-destructive.
        self.selected = ranking::preselected(&self.results, &query);
        self.first_row = 0;
        self.wheel_remainder = 0.0;
        self.reveal_selected();
        cx.notify();
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        let text = if mode == Mode::Capture { self.saved.capture_draft.clone() } else { String::new() };
        self.input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            if mode == Mode::Capture { input.select_all(window, cx); }
        });
        self.refresh(cx);
    }

    pub(crate) fn switch_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if self.mode == Mode::Actions { Mode::Capture } else { Mode::Actions };
        self.set_mode(mode, window, cx);
        self.focus_input(window, cx);
    }

    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Mode::Actions {
            if let Some(action) = self.selected.and_then(|selected| self.results.get(selected)).cloned() {
                self.saved.ran(&action.id);
                self.clear_input(window, cx);
                if let Some(error) = &self.error {
                    window.push_notification(Notification::error(error.clone()), cx);
                }
                self.close(window, cx);
                // The palette entity's update ends before the action executes; actions may reopen it.
                // Guard panics with catch_unwind; run returns () so returned errors are not available.
                let run = action.run;
                let palette = cx.entity().downgrade();
                let own = self.input.focus_handle(cx);
                window.defer(cx, move |window, cx| {
                    // The focus just given back may belong to something not drawn yet (the
                    // assistant card hides while an overlay is open). An action dispatched from
                    // there reaches only the window root, never the shell, so it starts from the
                    // palette's own input (drawn last frame, inside the shell) instead.
                    let restored = window.focused(cx);
                    let lost = window.context_stack().len() <= 1;
                    if lost { window.focus(&own, cx); }
                    let result = catch_unwind(AssertUnwindSafe(|| (run)(window, cx)));
                    if lost {
                        match &restored { Some(restored) => window.focus(restored, cx), None => window.blur(cx) }
                    }
                    if let Err(payload) = result {
                        // An action may reopen the palette before failing. Close that
                        // instance too, balancing OverlayGuard and restoring focus.
                        let _ = palette.update(cx, |this, cx| this.close(window, cx));
                        let msg = payload
                            .downcast_ref::<&'static str>()
                            .map(|s| *s)
                            .or_else(|| payload.downcast_ref::<String>().map(|s| s.as_str()))
                            .unwrap_or("Action panicked");
                        window.push_notification(Notification::error(format!("Action failed: {msg}")), cx);
                    }
                });
                return;
            }
            // Scrolling can clear selection. Make Enter visibly select the first
            // result rather than silently ignoring it or executing an unseen row.
            if !self.results.is_empty() {
                self.selected = Some(0);
                self.reveal_selected();
                cx.notify();
                return;
            }
        }
        self.save_capture(window, cx);
    }

    fn clear_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Mode::Capture { self.saved.capture_draft.clear(); }
        self.persist();
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn save_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        self.capture_text(&text, true, window, cx);
    }

    fn capture_text(&mut self, text: &str, clear: bool, window: &mut Window, cx: &mut Context<Self>) {
        match capture::save(text) {
            Ok(capture) => {
                self.error = None;
                if clear { self.clear_input(window, cx); }
                // State errors remain visible via notification even though the capture succeeded.
                if let Some(error) = &self.error {
                    window.push_notification(Notification::error(error.clone()), cx);
                }
                self.close(window, cx);
                // The notification API has no duration setter. Remove just this
                // notification after two seconds, not unrelated errors or Undo.
                self.capture_toast_generation = self.capture_toast_generation.wrapping_add(1);
                let generation = self.capture_toast_generation;
                window.push_notification(Notification::success(format!("Captured {}", capture.kind.label())).id::<CaptureToast>(), cx);
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        if this.capture_toast_generation == generation {
                            window.remove_notification::<CaptureToast>(cx);
                        }
                    });
                }).detach();
            }
            Err(e) => { self.error = Some(format!("Could not capture: {e}")); cx.notify(); }
        }
    }

    pub(crate) fn footer(&mut self, kind: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::Capture { self.set_mode(Mode::Capture, window, cx); }
        self.focus_input(window, cx);
        cx.notify();
        if kind != 1 && kind != 2 { return; }
        if self.picker_in_flight { return; }
        self.picker_in_flight = true;
        cx.notify();
        let generation = self.generation;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true, directories: false, multiple: false,
            prompt: Some(if kind == 2 { "Choose an image" } else { "Choose a file" }.into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = picker.await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Always release the guard, even if the generation is stale.
                this.picker_in_flight = false;
                if !this.open || this.generation != generation {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            if let Some(text) = path.to_str() {
                                // Capturing a file must not discard a thought already in the editor.
                                this.capture_text(text, false, window, cx);
                            } else {
                                this.error = Some("This file path is not valid UTF-8.".into());
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(e)) => this.error = Some(format!("Could not open file picker: {e}")),
                    Err(e) => this.error = Some(format!("File picker did not respond: {e}")),
                }
                if this.open { this.focus_input(window, cx); }
                cx.notify();
            });
        }).detach();
    }

    pub(crate) fn move_selection(&mut self, backwards: bool, cx: &mut Context<Self>) {
        if self.mode != Mode::Actions { return; }
        let count = self.results.len();
        if count == 0 { return; }
        self.selected = Some(match self.selected {
            Some(selected) if backwards => (selected + count - 1) % count,
            Some(selected) => (selected + 1) % count,
            None if backwards => (self.first_row + self.visible_rows).min(count) - 1,
            None => self.first_row,
        });
        self.reveal_selected();
        cx.notify();
    }

    fn reveal_selected(&mut self) {
        if let Some(selected) = self.selected {
            if selected < self.first_row { self.first_row = selected; }
            if selected >= self.first_row + self.visible_rows {
                self.first_row = selected + 1 - self.visible_rows;
            }
        }
    }

    pub(crate) fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke.key;
        let mods = event.keystroke.modifiers;
        let mut handled = true;
        match key.as_str() {
            "escape" => self.close(window, cx),
            "tab" => self.switch_mode(window, cx),
            "1" | "2" | "3" | "4" if mods.control => {
                self.footer(key.as_bytes()[0] as usize - b'1' as usize, window, cx);
            }
            "down" | "up" | "n" | "p" if self.mode == Mode::Actions
                && (key == "down" || key == "up" || mods.control) => {
                self.move_selection(key == "up" || key == "p", cx);
            }
            _ => handled = false,
        }
        if handled { cx.stop_propagation(); window.prevent_default(); }
    }
}

/// "Open URL or search" for the typed text: first when it is a web address, else last.
fn add_open_typed(results: &mut Vec<PaletteAction>, query: &str) {
    let text = query.trim();
    if text.is_empty() { return; }
    let typed = text.to_string();
    let action = PaletteAction {
        id: "browser.open_typed".into(),
        title: format!("Open URL or search: {text}").into(),
        keywords: "browser".into(),
        shortcut: None,
        run: std::rc::Rc::new(move |window, cx| ados_core::links::open_url(&typed, window, cx)),
    };
    if text.starts_with("http://") || text.starts_with("https://") { results.insert(0, action) } else { results.push(action) }
}
