//! Hint mode over the whole window: one key labels every marked aDOs control and, when the
//! Browser is on screen, every clickable of the page; typing a label clicks it.
//! Controls opt in with `ados_core::hints::mark()`; GPUI keeps hitboxes private.
use ados_core::hints::{self as core, Style, Targets};
use gpui_kit::*;
use std::time::Duration;

actions!(ados, [HintMode]);

/// Labels sit this far above the top of what they mark (label height 14, 3 px overlap).
const LABEL_RAISE: f32 = 11.;

pub fn init(cx: &mut App) {
    cx.default_global::<Targets>();
    ados_core::shortcuts::bind(cx, "shell.hints", HintMode);
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind { Ui, Page(usize) }

struct Item { bounds: Bounds<Pixels>, kind: Kind, label: String }

pub struct HintLayer {
    /// Held while hint mode runs, so no text field receives the typed label.
    focus: FocusHandle,
    items: Vec<Item>,
    typed: String,
    active: bool,
    starting: bool,
    epoch: u64,
    key_epoch: u64,
    /// aDOs marks and covering surfaces from the collecting frame.
    ui: Option<(Vec<core::Target>, Vec<core::Target>)>,
    /// The page's clickables; `None` while the page has not answered yet.
    page: Option<Vec<Bounds<Pixels>>>,
    /// One per hint mode run (relabelling keeps it), so a late page answer finds its run.
    session: u64,
    style: Style,
    focus_at_start: Option<FocusHandle>,
    interceptor: Option<Subscription>,
    _activation: Option<Subscription>,
}

impl HintLayer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() { this.cancel(window, cx); }
        });
        Self {
            focus: cx.focus_handle(),
            items: vec![],
            typed: String::new(),
            active: false,
            starting: false,
            epoch: 0,
            key_epoch: 0,
            ui: None,
            page: None,
            session: 0,
            style: Style::default(),
            focus_at_start: None,
            interceptor: None,
            _activation: Some(activation),
        }
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active || self.starting { return self.cancel(window, cx); }
        self.starting = true;
        self.epoch += 1;
        self.session += 1;
        self.typed.clear();
        self.items.clear();
        let (style, problem) = Style::load();
        self.style = style;
        if let Some(problem) = problem { push(window, problem, cx); }
        self.focus_at_start = window.focused(cx);
        window.focus(&self.focus, cx);
        let layer = cx.entity().downgrade();
        self.interceptor = Some(cx.intercept_keystrokes(move |event, window, cx| {
            let keystroke = event.keystroke.clone();
            if matches!(keystroke.key.as_str(), "shift" | "control" | "alt" | "platform" | "function") { return; }
            if layer.update(cx, |h, cx| h.key(&keystroke, window, cx)).is_ok() { cx.stop_propagation(); }
        }));
        // aDOs targets come from one collecting frame, page targets from the page script.
        // aDOs's labels never wait for the page: theirs are added when it answers.
        self.page = Some(vec![]);
        #[cfg(feature = "browser")]
        if let Some(browser) = crate::modules::browser_panel(cx) {
            if let Some(targets) = browser.read(cx).hint_targets(window, cx) {
                browser.read(cx).hints_hold();
                self.page = None;
                let session = self.session;
                cx.spawn_in(window, async move |this, cx| {
                    let found = targets.await;
                    let _ = this.update_in(cx, |h, window, cx| h.page_arrived(session, found, window, cx));
                }).detach();
                // A busy page must not hold its labels open forever.
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(1500)).await;
                    let _ = this.update_in(cx, |h, window, cx| h.page_arrived(session, vec![], window, cx));
                }).detach();
            }
        }
        self.collect_ui(window, cx);
        cx.notify();
    }

    /// Collect aDOs's marks over two forced full frames: a frame already under way when the
    /// key arrived may not carry them. Duplicates from the second frame fall to the dedup.
    fn collect_ui(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ui = None;
        let targets = cx.global_mut::<Targets>();
        targets.collecting = true;
        targets.found.clear();
        targets.covers.clear();
        let layer = cx.entity().downgrade();
        let epoch = self.epoch;
        window.on_next_frame(move |window, _| {
            window.refresh();
            window.on_next_frame(move |window, cx| {
                let targets = cx.global_mut::<Targets>();
                targets.collecting = false;
                let found = (std::mem::take(&mut targets.found), std::mem::take(&mut targets.covers));
                let _ = layer.update(cx, |h, cx| {
                    if h.epoch == epoch { h.ui = Some(found); h.ready(window, cx); }
                });
            });
        });
        window.refresh();
    }

    /// The page answered (or gave up). Before aDOs's labels are up they are simply
    /// included; after, they are added by labelling afresh, unless a label is half typed
    /// (then the page sits this run out, so no label changes under the typing).
    fn page_arrived(&mut self, session: u64, found: Vec<Bounds<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        if self.session != session || self.page.is_some() || !(self.active || self.starting) { return; }
        if self.active && !self.typed.is_empty() {
            self.page = Some(vec![]);
            return;
        }
        let relabel = self.active && !found.is_empty();
        self.page = Some(found);
        if relabel {
            // Retire the running watch and timeout; `ready` starts fresh ones.
            self.epoch += 1;
            self.ready(window, cx);
        } else if self.starting {
            self.ready(window, cx);
        }
    }

    /// aDOs's marks are in: label everything on screen (the page's, if it has answered).
    fn ready(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((ui, covers)) = self.ui.clone() else { return };
        let page = self.page.clone().unwrap_or_default();
        let mut found = Self::reachable(&ui, &covers, cx);
        // Page targets under the assistant card (cut out of the page) cannot be clicked either.
        found.extend(page.iter().enumerate()
            .filter(|(_, b)| !covers.iter().any(|c| c.bounds.contains(&b.center())))
            .map(|(i, b)| (*b, Kind::Page(i))));
        let points: Vec<(f32, f32)> = found.iter().map(|(b, _)| (f32::from(b.origin.x), f32::from(b.origin.y))).collect();
        let kept = core::dedup(&points, core::DEDUP_PX);
        let labels = core::labels(kept.len());
        self.items = kept.into_iter().zip(labels)
            .map(|(i, label)| Item { bounds: found[i].0, kind: found[i].1, label })
            .collect();
        if self.items.is_empty() {
            // The page may still have something to click.
            if self.page.is_none() { return; }
            push(window, "Nothing to click on screen.", cx);
            return self.cancel(window, cx);
        }
        self.starting = false;
        self.active = true;
        self.watch(window, cx);
        #[cfg(feature = "browser")]
        if let Some(browser) = crate::modules::browser_panel(cx) {
            let mut page_labels = vec![None; page.len()];
            for item in &self.items {
                if let Kind::Page(i) = item.kind { page_labels[i] = Some(item.label.clone()); }
            }
            if !page.is_empty() { browser.update(cx, |b, _| b.hints_show(&page_labels)); }
        }
        // Letters typed while labels were being collected still count.
        let typed = std::mem::take(&mut self.typed);
        for ch in typed.chars() {
            if !self.active { break; }
            self.letter(ch, window, cx);
        }
        self.arm(window, cx);
        cx.notify();
    }

    /// Clickable aDOs controls: no mark hidden under a card, menu, toast or modal overlay.
    fn reachable(ui: &[core::Target], covers: &[core::Target], cx: &App) -> Vec<(Bounds<Pixels>, Kind)> {
        let modal = ados_core::overlay::OverlayGuard::any(cx);
        core::reachable(ui, covers, modal).into_iter().map(|i| (ui[i].bounds, Kind::Ui)).collect()
    }

    /// While labels are up, every drawn frame reports the marks it painted (GPUI repaints
    /// only the views that changed, so a frame may carry just some of them). A mark that
    /// is not where a label is means something moved or appeared (a list settled, a panel
    /// opened): collect everything again over full frames and label it afresh, so no
    /// label points at the wrong control. Runs only during hint mode, which ends by itself.
    fn watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets = cx.global_mut::<Targets>();
        targets.collecting = true;
        targets.found.clear();
        targets.covers.clear();
        let (layer, epoch) = (cx.entity().downgrade(), self.epoch);
        window.on_next_frame(move |window, cx| {
            let _ = layer.update(cx, |h, cx| {
                if h.epoch != epoch || !h.active { return; }
                let targets = cx.global_mut::<Targets>();
                let (ui, covers) = (std::mem::take(&mut targets.found), std::mem::take(&mut targets.covers));
                let now = Self::reachable(&ui, &covers, cx);
                let moved = now.iter().any(|(b, _)| {
                    !h.items.iter().any(|i| i.kind == Kind::Ui && core::near(i.bounds, *b))
                });
                if moved {
                    h.epoch += 1;
                    h.items.clear();
                    h.typed.clear();
                    h.starting = true;
                    h.active = false;
                    h.collect_ui(window, cx);
                    cx.notify();
                    return;
                }
                h.watch(window, cx);
            });
        });
    }

    fn arm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.key_epoch += 1;
        let (epoch, key_epoch) = (self.epoch, self.key_epoch);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(core::TIMEOUT_MS)).await;
            let _ = this.update_in(cx, |h, window, cx| {
                if h.epoch == epoch && h.key_epoch == key_epoch { h.cancel(window, cx); }
            });
        }).detach();
    }

    fn key(&mut self, keystroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        let m = keystroke.modifiers;
        let plain = !m.control && !m.alt && !m.shift && !m.platform;
        if !self.focus.is_focused(window) { return self.cancel(window, cx); }
        let letter = keystroke.key.chars().next().filter(|c| keystroke.key.len() == 1 && core::ALPHABET.contains(*c));
        match (plain, keystroke.key.as_str(), letter) {
            (true, "backspace", _) => {
                self.typed.pop();
                self.filtered(window, cx);
            }
            (true, _, Some(ch)) if self.starting => self.typed.push(ch),
            (true, _, Some(ch)) => self.letter(ch, window, cx),
            _ => self.cancel(window, cx),
        }
    }

    fn letter(&mut self, ch: char, window: &mut Window, cx: &mut Context<Self>) {
        self.typed.push(ch);
        let left: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].label.starts_with(&self.typed)).collect();
        match left.as_slice() {
            [] => self.cancel(window, cx),
            [only] if self.items[*only].label == self.typed => self.pick(*only, window, cx),
            _ => self.filtered(window, cx),
        }
    }

    fn filtered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "browser")]
        if let Some(browser) = crate::modules::browser_panel(cx) { browser.read(cx).hints_typed(&self.typed); }
        self.arm(window, cx);
        cx.notify();
    }

    fn pick(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (bounds, kind) = (self.items[index].bounds, self.items[index].kind);
        self.finish(cx);
        self.restore_focus(window, cx);
        #[cfg(feature = "browser")]
        if let Some(browser) = crate::modules::browser_panel(cx) {
            match kind {
                Kind::Page(i) => return browser.update(cx, |b, cx| b.hints_pick(i, window, cx)),
                Kind::Ui => browser.update(cx, |b, _| b.hints_hide(window)),
            }
        }
        let _ = kind;
        // The same synthetic click GPUI uses for accessibility clicks: the centre of the control.
        let at = bounds.center();
        window.defer(cx, move |window, cx| {
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: at, pressed_button: None, modifiers: Modifiers::default() }), cx);
            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1, first_mouse: false,
            }), cx);
            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1,
            }), cx);
        });
    }

    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.active && !self.starting { return; }
        self.finish(cx);
        self.restore_focus(window, cx);
        #[cfg(feature = "browser")]
        if let Some(browser) = crate::modules::browser_panel(cx) { browser.update(cx, |b, _| b.hints_hide(window)); }
        let _ = window;
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.is_focused(window) {
            if let Some(previous) = self.focus_at_start.take() { window.focus(&previous, cx); }
        }
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        self.active = false;
        self.starting = false;
        self.epoch += 1;
        self.items.clear();
        self.typed.clear();
        self.ui = None;
        self.page = None;
        self.interceptor = None;
        cx.global_mut::<Targets>().collecting = false;
        cx.notify();
    }
}

fn push(window: &mut Window, text: impl Into<SharedString>, cx: &mut App) {
    use gpui_kit::component::WindowExt;
    window.push_notification(text.into(), cx);
}

impl Render for HintLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.active { return div().track_focus(&self.focus).into_any_element(); }
        let (bg, fg, dim) = (Style::hsla(self.style.background), Style::hsla(self.style.text), Style::hsla(self.style.typed));
        let typed = self.typed.clone();
        let labels = self.items.iter()
            .filter(|i| i.kind == Kind::Ui && i.label.starts_with(&typed))
            .map(|i| {
                // Just above the control's top-left corner (3 px overlap), so its text stays readable.
                div().absolute().left(i.bounds.origin.x).top((i.bounds.origin.y - px(LABEL_RAISE)).max(px(0.)))
                    .h(px(14.)).min_w(px(11.)).px(px(3.)).rounded(px(4.)).bg(bg)
                    .flex().items_center().justify_center()
                    .text_size(px(12.)).line_height(px(14.)).font_weight(FontWeight::BOLD).text_color(fg)
                    .child(div().text_color(dim).child(typed.clone()))
                    .child(i.label[typed.len()..].to_string())
            });
        let layer = cx.entity().downgrade();
        let scroll_layer = layer.clone();
        deferred(
            div().absolute().top_0().left_0().size_full()
                .track_focus(&self.focus)
                .children(labels)
                // Any click or scroll ends hint mode; nothing is left behind.
                .child(canvas(|_, _, _| (), move |_, _, window, _| {
                    let layer = layer.clone();
                    window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            let layer = layer.clone();
                            window.defer(cx, move |window, cx| { let _ = layer.update(cx, |h, cx| h.cancel(window, cx)); });
                        }
                    });
                    let layer = scroll_layer.clone();
                    window.on_mouse_event(move |_: &ScrollEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            let layer = layer.clone();
                            window.defer(cx, move |window, cx| { let _ = layer.update(cx, |h, cx| h.cancel(window, cx)); });
                        }
                    });
                }).absolute().size_full()),
        )
        .with_priority(10)
        .into_any_element()
    }
}
