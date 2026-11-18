//! Comment mode: point at an element or a text selection in the page and tell the
//! assistant about it. The page side is `comment.js`, injected through `CommandMsg::Eval`
//! only while commenting; the block the agent gets is built in `comment_block.rs`.
//! Keys: the registry chord or toolbar icon enters; a click or a hint label picks; Esc leaves.
use super::*;
use crate::comment_block::{self as blocks, Comment, Picked};
use gpui_kit::component::input::{InputEvent, InputState};
use std::time::Duration;

const SCRIPT: &str = include_str!("../comment.js");
/// After a page press, ask the page for its pick this often, this many times (a click lands on release).
const TAKE_EVERY: Duration = Duration::from_millis(75);
const TAKE_TRIES: usize = 20;

actions!(ados_browser, [CommentMode]);

#[derive(Default)]
pub(crate) struct Comments {
    /// Picking: the page outlines what is under the mouse and blocks its clicks.
    pub on: bool,
    pub card: Option<Card>,
    /// Written but not sent yet ("Send 3 comments").
    pub queue: Vec<Comment>,
    /// Last pin number used on `page`.
    next: usize,
    page: String,
    pub labels: Option<(Vec<String>, String)>,
    serial: u64,
    taking: bool,
}

pub(crate) struct Card {
    pub picked: Picked,
    pub input: Entity<InputState>,
    pub number: usize,
    image: Option<std::path::PathBuf>,
    serial: u64,
    /// Esc goes back to picking (an element pick), else leaves comment mode.
    resume: bool,
    _enter: Subscription,
}

/// `(() => { const c = <comment.js>; return <call>; })()`: install if needed, then call.
fn with_script(call: &str) -> String {
    format!("(() => {{ const c = {SCRIPT}; return {call}; }})()")
}

fn panel(cx: &App) -> Option<Entity<BrowserPanel>> {
    cx.global::<Target>().0.as_ref().and_then(WeakEntity::upgrade)
}

pub(super) fn init(cx: &mut App) {
    ados_core::shortcuts::bind(cx, "browser.comment", CommentMode);
    type Run = fn(&mut BrowserPanel, &mut Window, &mut Context<BrowserPanel>);
    let list: [(&'static str, &'static str, &'static str, Run); 4] = [
        ("browser.comment", "Comment on the page", "browser comment point element annotate assistant", |p, w, cx| p.toggle_comment(w, cx)),
        ("browser.comment_send", "Send queued comments to assistant", "browser comment send batch assistant", |p, w, cx| p.send_queue(w, cx)),
        ("browser.send_page", "Send page to assistant", "browser page assistant ask agent", |p, w, cx| p.send_page(false, w, cx)),
        ("browser.send_selection", "Send selection to assistant", "browser selection text assistant ask agent", |p, w, cx| p.send_page(true, w, cx)),
    ];
    for (id, title, keywords, run) in list {
        ados_core::actions::register(id, title, keywords, None, cx, move |window, cx| match panel(cx) {
            Some(panel) if panel.read(cx).ready => panel.update(cx, |p, cx| run(p, window, cx)),
            _ => window.push_notification("Open the Browser module first", cx),
        });
    }
}

impl BrowserPanel {
    fn run_quiet(&self, script: String) {
        if let Some(runtime) = &self.runtime {
            let (tx, _) = async_channel::bounded(1);
            runtime.command(CommandMsg::Eval(script, tx));
        }
    }
    pub(super) fn commenting(&self) -> bool {
        self.comment.on || self.comment.card.is_some()
    }

    pub(super) fn toggle_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.commenting() { return self.leave_comment(window, cx); }
        let Some(runtime) = self.runtime.clone().filter(|_| self.ready) else {
            return window.push_notification("The page is not ready yet", cx);
        };
        self.comment.on = true;
        self.hold_keys(window, cx);
        // Text already selected is the target: no pick needed.
        let (tx, rx) = async_channel::bounded(1);
        runtime.command(CommandMsg::Eval(with_script("c.selection() || (c.start(), '')"), tx));
        cx.spawn_in(window, async move |this, cx| {
            let reply = rx.recv().await;
            let _ = this.update_in(cx, |this, window, cx| match reply {
                Ok(Ok(text)) => {
                    if let Some(picked) = Picked::parse(&text) { this.open_card(picked, false, window, cx); }
                }
                _ => {
                    this.comment.on = false;
                    this.release_keys(window, cx);
                    window.push_notification("Comment mode could not start on this page", cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn leave_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.comment.labels.take().is_some() { self.run_quiet("window.__adosHints && window.__adosHints.hide()".into()); }
        self.comment.on = false;
        self.comment.card = None;
        self.run_quiet("window.__adosComment && window.__adosComment.stop()".into());
        self.release_keys(window, cx);
        cx.notify();
    }

    /// aDOs keeps the keyboard while commenting; the page still gets the mouse.
    fn hold_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hint_hold.set(true);
        if let Some(runtime) = &self.runtime { runtime.focus_address(); }
        match &self.comment.card {
            Some(card) => card.input.update(cx, |input, cx| input.focus(window, cx)),
            None => self.focus.focus(window, cx),
        }
    }
    fn release_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hint_hold.set(false);
        self.focus_primary(window, cx);
    }
    /// `focus_primary` while commenting: the card's field, or the panel itself.
    pub(super) fn comment_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hold_keys(window, cx);
    }

    /// A press in the page while picking: the page records the clicked element on release.
    pub(super) fn comment_pressed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hold_keys(window, cx);
        if self.comment.card.is_some() || self.comment.taking { return; }
        let Some(runtime) = self.runtime.clone() else { return };
        self.comment.taking = true;
        cx.spawn_in(window, async move |this, cx| {
            let mut picked = None;
            for _ in 0..TAKE_TRIES {
                cx.background_executor().timer(TAKE_EVERY).await;
                let (tx, rx) = async_channel::bounded(1);
                runtime.command(CommandMsg::Eval("window.__adosComment ? window.__adosComment.take() : ''".into(), tx));
                if let Ok(Ok(text)) = rx.recv().await {
                    if let Some(p) = Picked::parse(&text) { picked = Some(p); break; }
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.comment.taking = false;
                if let Some(picked) = picked.filter(|_| this.comment.on && this.comment.card.is_none()) {
                    this.open_card(picked, true, window, cx);
                }
            });
        })
        .detach();
    }

    /// Keys while picking: Esc leaves; the hint chord labels the page; label letters pick.
    pub(super) fn comment_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.comment.on || self.comment.card.is_some() { return; }
        let k = &event.keystroke;
        let m = k.modifiers;
        let plain = !m.control && !m.alt && !m.platform;
        if let Some((labels, typed)) = &mut self.comment.labels {
            match k.key.as_str() {
                "escape" => { self.comment.labels = None; self.run_quiet("window.__adosHints && window.__adosHints.hide()".into()); }
                "backspace" => { typed.pop(); let t = typed.clone(); self.hints_typed(&t); }
                key if plain && key.len() == 1 && ados_core::hints::ALPHABET.contains(key) => {
                    typed.push_str(key);
                    let left: Vec<usize> = (0..labels.len()).filter(|&i| labels[i].starts_with(typed.as_str())).collect();
                    match left.as_slice() {
                        [] => { self.comment.labels = None; self.run_quiet("window.__adosHints && window.__adosHints.hide()".into()); }
                        [one] if labels[*one] == *typed => { let i = *one; self.comment.labels = None; self.comment_hint(i, window, cx); }
                        _ => { let t = typed.clone(); self.hints_typed(&t); }
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
            return cx.notify();
        }
        if k.key == "escape" {
            cx.stop_propagation();
            return self.leave_comment(window, cx);
        }
        let mut chord = String::new();
        for (on, name) in [(m.control, "ctrl-"), (m.alt, "alt-"), (m.shift, "shift-"), (m.platform, "super-")] {
            if on { chord.push_str(name); }
        }
        chord.push_str(&k.key);
        let hint_keys = hints::config(cx)["hints"].as_array().cloned().unwrap_or_default();
        if hint_keys.iter().any(|c| c.as_str() == Some(chord.as_str())) {
            cx.stop_propagation();
            self.comment_labels(window, cx);
        }
    }

    fn comment_labels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(runtime) = self.runtime.clone() else { return };
        let (tx, rx) = async_channel::bounded(1);
        runtime.command(CommandMsg::Eval("window.__adosHints ? window.__adosHints.collect() : ''".into(), tx));
        cx.spawn_in(window, async move |this, cx| {
            let count = match rx.recv().await {
                Ok(Ok(text)) => serde_json::from_str::<serde_json::Value>(&text).ok()
                    .and_then(|v| v["items"].as_array().map(Vec::len)).unwrap_or(0),
                _ => 0,
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.comment.on || this.comment.card.is_some() { return; }
                if count == 0 { return window.push_notification("Nothing on this page to label", cx); }
                let labels = ados_core::hints::labels(count);
                this.hints_show(&labels.iter().cloned().map(Some).collect::<Vec<_>>());
                this.comment.labels = Some((labels, String::new()));
                cx.notify();
            });
        })
        .detach();
    }

    /// Hint target `index` (page labels, or aDOs's window-wide hint mode) is the pick.
    pub(super) fn comment_hint(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(runtime) = self.runtime.clone() else { return };
        let (tx, rx) = async_channel::bounded(1);
        let script = format!("(() => {{ const r = window.__adosComment ? window.__adosComment.hint({index}) : ''; \
            window.__adosHints && window.__adosHints.hide(); return r; }})()");
        runtime.command(CommandMsg::Eval(script, tx));
        cx.spawn_in(window, async move |this, cx| {
            let reply = rx.recv().await;
            let _ = this.update_in(cx, |this, window, cx| match reply.ok().and_then(Result::ok).as_deref().and_then(Picked::parse) {
                Some(picked) if this.comment.on => this.open_card(picked, true, window, cx),
                _ => this.hold_keys(window, cx),
            });
        })
        .detach();
    }

    /// "Send page / selection to assistant": the same card, no pick.
    pub(super) fn send_page(&mut self, selection: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(runtime) = self.runtime.clone() else { return };
        if self.commenting() { self.leave_comment(window, cx); }
        let (tx, rx) = async_channel::bounded(1);
        runtime.command(CommandMsg::Eval(with_script(if selection { "c.selection()" } else { "c.page()" }), tx));
        cx.spawn_in(window, async move |this, cx| {
            let reply = rx.recv().await;
            let _ = this.update_in(cx, |this, window, cx| match reply.ok().and_then(Result::ok).as_deref().and_then(Picked::parse) {
                Some(picked) => this.open_card(picked, false, window, cx),
                None if selection => window.push_notification("Select some text in the page first", cx),
                None => window.push_notification("Could not read this page", cx),
            });
        })
        .detach();
    }

    fn open_card(&mut self, picked: Picked, resume: bool, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Comment for the assistant"));
        let enter = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary, .. } = event {
                if *secondary { this.queue_card(window, cx) } else { this.send_card(window, cx) }
            }
        });
        if self.comment.page != picked.url {
            self.comment.page = picked.url.clone();
            self.comment.next = 0;
        }
        let number = self.comment.next + self.comment.queue.iter().filter(|c| c.picked.url == picked.url).count() + 1;
        self.comment.serial += 1;
        let serial = self.comment.serial;
        // Crop now, before the card's strip moves the page.
        if let (Some(runtime), Some(target)) = (&self.runtime, &picked.target) {
            let viewport_h = (picked.h * if picked.dpr > 0. { picked.dpr } else { 1. }).round() as u32;
            let rx = runtime.shot(shot::to_window(target.viewport(), picked.dpr), viewport_h, shot::path_for(number));
            cx.spawn_in(window, async move |this, cx| {
                let Ok(result) = rx.recv().await else { return };
                let _ = this.update(cx, |this, _| match result {
                    Ok(path) => {
                        if let Some(card) = this.comment.card.as_mut().filter(|c| c.serial == serial) { card.image = Some(path); }
                    }
                    Err(e) => eprintln!("aDOs browser: no crop for comment {number}: {e}"),
                });
            })
            .detach();
        }
        self.comment.card = Some(Card { picked, input, number, image: None, serial, resume, _enter: enter });
        self.hold_keys(window, cx);
        cx.notify();
    }

    pub(super) fn cancel_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.comment.card.take() {
            Some(card) if card.resume && self.comment.on => {
                self.run_quiet("window.__adosComment && window.__adosComment.resume()".into());
                self.hold_keys(window, cx);
                cx.notify();
            }
            _ => self.leave_comment(window, cx),
        }
    }

    fn take_card(&mut self, cx: &App) -> Option<Comment> {
        let card = self.comment.card.take()?;
        let comment = card.input.read(cx).value().to_string();
        Some(Comment { number: card.number, picked: card.picked, comment, image: card.image })
    }

    /// Ctrl+Enter: keep this comment for one send with the next ones; pick again.
    pub(super) fn queue_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let resume = self.comment.card.as_ref().is_some_and(|c| c.resume);
        let Some(comment) = self.take_card(cx) else { return };
        self.comment.queue.push(comment);
        if resume && self.comment.on {
            self.run_quiet("window.__adosComment && window.__adosComment.resume()".into());
            self.hold_keys(window, cx);
        } else {
            self.leave_comment(window, cx);
        }
        cx.notify();
    }

    pub(super) fn send_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(comment) = self.take_card(cx) else { return };
        let mut all = std::mem::take(&mut self.comment.queue);
        all.push(comment);
        self.send_comments(all, window, cx);
    }

    pub(super) fn send_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.comment.queue.is_empty() {
            return window.push_notification("No comments are waiting to be sent", cx);
        }
        let all = std::mem::take(&mut self.comment.queue);
        self.send_comments(all, window, cx);
    }

    fn send_comments(&mut self, all: Vec<Comment>, window: &mut Window, cx: &mut Context<Self>) {
        let text = blocks::block(&all);
        // Leave first: the assistant then takes the keyboard for its composer.
        self.leave_comment(window, cx);
        if !ados_core::ask::send(text, window, cx) {
            self.comment.queue = all;
            return window.push_notification("The assistant is not available, so the comments were kept", cx);
        }
        let pins: Vec<serde_json::Value> = all.iter()
            .filter(|c| c.picked.url == self.comment.page)
            .filter_map(|c| c.picked.target.as_ref().map(|t| (c, t.page())))
            .map(|(c, r)| serde_json::json!([c.number, c.comment.trim(), [r[0], r[1]]]))
            .collect();
        self.comment.next += pins.len();
        if !pins.is_empty() {
            self.run_quiet(format!("window.__adosComment && window.__adosComment.pin({})", serde_json::Value::Array(pins)));
        }
        cx.notify();
    }

    /// A new document: its pins are gone, numbering restarts, picking stops.
    pub(super) fn comment_page(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.comment.page != url {
            self.comment.page = url.to_string();
            self.comment.next = 0;
            if self.comment.on && self.comment.card.is_none() { self.leave_comment(window, cx); }
        }
    }
}
