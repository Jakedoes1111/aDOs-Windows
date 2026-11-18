//! Prompts other modules hand over through `ados_core::ask` (the Browser's comments).
//! The owner's rule: they go into the open chat only if aDOs started it in this run,
//! otherwise into a new chat. Never into an existing chat aDOs did not start.
use super::Assistant;
use gpui_kit::*;

pub(super) fn register(entity: &Entity<Assistant>, cx: &mut App) {
    let target = entity.downgrade();
    ados_core::ask::set_handler(cx, move |text, window, cx| {
        let _ = target.update(cx, |this, cx| this.receive(text, window, cx));
    });
}

/// Whether `chat` may receive a handed-over prompt, given the sessions aDOs started.
pub(super) fn may_write(chat: Option<&str>, started: &[String]) -> bool {
    chat.is_some_and(|id| started.iter().any(|s| s == id))
}

impl Assistant {
    fn receive(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let started: Vec<String> = ados_agents::started::list().into_iter().map(|s| s.id).collect();
        if self.saved.chat.is_some() && !may_write(self.saved.chat.as_deref(), &started) {
            self.new_chat(window, cx);
        } else {
            self.open(window, cx);
        }
        if !self.can_send() {
            // A reply is still streaming: the block waits in the composer for the owner.
            let current = self.input.read(cx).value().to_string();
            let joined = if current.trim().is_empty() { text } else { format!("{}\n\n{text}", current.trim_end()) };
            self.input.update(cx, |input, cx| input.set_value(joined, window, cx));
            self.save_draft(cx);
            return cx.notify();
        }
        self.steps.clear();
        self.error = None;
        if self.replaying {
            // The chat is still opening: send once it is open.
            self.unsent = Some(text.clone());
            self.queued = Some((text, vec![]));
        } else {
            self.append_row(true, text.clone(), cx);
            self.reading_scroll.scroll_to_bottom();
            self.send_text(text, vec![], window, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::may_write;
    #[test]
    fn only_chats_ados_started() {
        let started = vec!["a".to_string()];
        assert!(may_write(Some("a"), &started));
        assert!(!may_write(Some("owner-chat"), &started));
        assert!(!may_write(None, &started));
    }
}
