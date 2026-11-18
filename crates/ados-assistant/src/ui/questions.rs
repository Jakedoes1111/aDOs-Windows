//! Permission requests from the agent: a plain question with the agent's options as
//! buttons. Esc, closing the card, switching chat or backend all answer "deny".
use super::Assistant;
use ados_agents::{PermissionKind, PermissionRequest};
use futures::channel::oneshot;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    ActiveTheme, Sizable,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

pub(super) struct PendingQuestion {
    pub request: PermissionRequest,
    pub reply: oneshot::Sender<Option<String>>,
}

/// The question shown for a permission request.
pub(crate) fn question_text(request: &PermissionRequest) -> String {
    let title = request.title.trim();
    if title.is_empty() {
        "Allow the agent to run a tool?".into()
    } else {
        format!("Allow: {title}?")
    }
}

impl Assistant {
    pub(super) fn permission(
        &mut self,
        request: PermissionRequest,
        reply: oneshot::Sender<Option<String>>,
        cx: &mut Context<Self>,
    ) {
        self.questions.push(PendingQuestion { request, reply });
        if !self.saved.open {
            self.ready = true;
        }
        cx.notify();
    }
    /// Answer question `index` with an option id, or `None` to deny.
    fn answer(&mut self, index: usize, choice: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.questions.len() {
            return;
        }
        let pending = self.questions.remove(index);
        // The agent may have stopped waiting; nothing to tell the user then.
        let _ = pending.reply.send(choice);
        if self.saved.open {
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }
    /// Tab in the composer while a question is open goes to its first option (as in
    /// Sessions), not to Stop below. Tab reaches the textarea as `IndentInline`.
    pub(super) fn tab_to_question(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.questions.is_empty() || !self.input.focus_handle(cx).is_focused(window) {
            return false;
        }
        self.question_focus.focus(window, cx);
        cx.notify();
        true
    }
    pub(super) fn deny_all(&mut self) {
        for pending in self.questions.drain(..) {
            let _ = pending.reply.send(None);
        }
    }
    pub(super) fn questions_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div()
            .id("assistant-questions")
            .max_h(px(220.))
            .overflow_y_scroll()
            .flex_shrink_0();
        for (index, pending) in self.questions.iter().enumerate() {
            let mut options = div().flex().flex_wrap().gap_1();
            for (option_index, option) in pending.request.options.iter().enumerate() {
                let id = option.id.clone();
                let button = Button::new((SharedString::from(format!("permission-{index}")), option_index))
                    .small()
                    .h(px(32.))
                    .label(option.name.clone())
                    .child(super::header::float_mark())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.answer(index, Some(id.clone()), window, cx)
                    }))
                    .when(index == 0 && option_index == 0, |b| b.track_focus(&self.question_focus));
                options = options.child(if option.kind.is_allow() {
                    button.primary()
                } else if matches!(option.kind, PermissionKind::RejectOnce | PermissionKind::RejectAlways) {
                    button.ghost()
                } else {
                    button
                });
            }
            if pending.request.options.is_empty() {
                options = options.child(
                    Button::new(("permission-deny", index))
                        .ghost()
                        .small()
                        .h(px(32.))
                        .label("Deny")
                        .child(super::header::float_mark())
                        .when(index == 0, |b| b.track_focus(&self.question_focus))
                        .on_click(cx.listener(move |this, _, window, cx| this.answer(index, None, window, cx))),
                );
            }
            list = list.child(
                div()
                    .p_2()
                    .border_t_1()
                    .border_color(cx.theme().primary)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().whitespace_normal().child(question_text(&pending.request)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Esc denies"),
                    )
                    .child(options),
            );
        }
        list.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::question_text;
    use ados_agents::PermissionRequest;

    fn request(title: &str) -> PermissionRequest {
        PermissionRequest {
            session_id: "s".into(),
            tool_call_id: "t".into(),
            title: title.into(),
            options: vec![],
        }
    }

    #[test]
    fn question_names_the_tool_call() {
        assert_eq!(question_text(&request("Write fake-note.txt")), "Allow: Write fake-note.txt?");
        assert_eq!(question_text(&request("  ")), "Allow the agent to run a tool?");
    }
}
