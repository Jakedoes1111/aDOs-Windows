//! Chat actions: drafts, new/open chat, send.
use super::Assistant;
use gpui_kit::*;
use std::path::PathBuf;

impl Assistant {
    pub(super) fn save_draft(&mut self, cx: &App) {
        self.saved.drafts.insert(
            self.saved.chat.clone().unwrap_or_else(|| "new".into()),
            self.input.read(cx).value().to_string(),
        );
        self.persist();
    }
    pub(super) fn restore_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .saved
            .drafts
            .get(self.saved.chat.as_deref().unwrap_or("new"))
            .cloned()
            .unwrap_or_default();
        self.input
            .update(cx, |input, cx| {
                let end = text.len();
                input.set_value(text, window, cx);
                input.set_selected_range(end..end, cx);
            });
    }
    fn leave_chat(&mut self, cx: &mut Context<Self>) {
        self.save_draft(cx);
        self.deny_all();
        if self.running {
            self.stop_turn();
        }
        self.turn = None;
        self.running = false;
        self.queued = None;
        self.reset_reading(cx);
        self.steps.clear();
        self.attached.clear();
        self.dismissed_selection = None;
        self.switcher = false;
        self.notice = None;
    }
    /// A new chat: the agent's `session/new` (home folder) runs on the first send.
    pub fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_chat(cx);
        self.saved.chat = None;
        self.saved.chat_cwd = None;
        self.live_session = None;
        self.restore_draft(window, cx);
        self.persist();
        self.open(window, cx);
    }
    pub(super) fn open_chat(&mut self, id: String, cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_chat(cx);
        self.saved.chat = Some(id);
        self.saved.chat_cwd = Some(cwd);
        self.live_session = None;
        self.restore_draft(window, cx);
        self.persist();
        self.restore_chat(window, cx);
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    pub(super) fn can_send(&self) -> bool {
        !self.running && self.queued.is_none()
    }
    pub(super) fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_send() {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.steps.clear();
        self.error = None;
        let context = self.context(cx);
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.save_draft(cx);
        if self.replaying {
            // The saved chat is still opening: send once it is open (or forgotten).
            self.unsent = Some(text.clone());
            self.queued = Some((text, context));
        } else {
            self.append_row(true, text.clone(), cx);
            self.reading_scroll.scroll_to_bottom();
            self.send_text(text, context, window, cx);
        }
        cx.notify();
    }
}
