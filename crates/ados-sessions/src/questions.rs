//! Permission requests: the same plain question and option buttons as the assistant
//! card. Esc denies; a turn that ends or a backend that stops denies what is left.
use crate::panel::SessionsPanel;
use ados_agents::{PermissionKind, PermissionRequest};
use futures::channel::oneshot;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    ActiveTheme, Sizable,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

pub(crate) struct PendingQuestion {
    /// Session key (`backend/id`).
    pub key: String,
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

impl SessionsPanel {
    pub(crate) fn has_question(&self, key: &str) -> bool {
        self.questions.iter().any(|q| q.key == key)
    }

    /// Answer the `nth` question of the open session with an option id, or `None` to deny.
    pub(crate) fn answer(&mut self, nth: usize, choice: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.saved.selected.clone() else { return };
        let Some(index) = self.questions.iter().enumerate().filter(|(_, q)| q.key == key).nth(nth).map(|(i, _)| i) else {
            return;
        };
        let pending = self.questions.remove(index);
        // The agent may have stopped waiting; nothing to tell the user then.
        let _ = pending.reply.send(choice);
        self.composer.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(crate) fn deny_where(&mut self, which: impl Fn(&str) -> bool) {
        let (deny, keep): (Vec<_>, Vec<_>) = self.questions.drain(..).partition(|q| which(&q.key));
        self.questions = keep;
        for pending in deny {
            let _ = pending.reply.send(None);
        }
    }

    pub(crate) fn deny_all(&mut self) {
        self.deny_where(|_| true);
    }

    pub(crate) fn questions_view(&self, key: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let mine: Vec<_> = self.questions.iter().filter(|q| q.key == key).collect();
        if mine.is_empty() {
            return None;
        }
        let mut list = div().flex().flex_col().flex_shrink_0();
        for (nth, pending) in mine.into_iter().enumerate() {
            let mut options = div().flex().flex_wrap().gap_1();
            for (option_ix, option) in pending.request.options.iter().enumerate() {
                let id = option.id.clone();
                let button = Button::new((SharedString::from(format!("session-permission-{nth}")), option_ix))
                    .small()
                    .h(px(32.))
                    .label(option.name.clone())
                    .child(ados_core::hints::mark())
                    .on_click(cx.listener(move |this, _, window, cx| this.answer(nth, Some(id.clone()), window, cx)))
                    .when(nth == 0 && option_ix == 0, |b| b.track_focus(&self.question_focus));
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
                    Button::new(("session-permission-deny", nth))
                        .ghost()
                        .small()
                        .h(px(32.))
                        .label("Deny")
                        .child(ados_core::hints::mark())
                        .when(nth == 0, |b| b.track_focus(&self.question_focus))
                        .on_click(cx.listener(move |this, _, window, cx| this.answer(nth, None, window, cx))),
                );
            }
            list = list.child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(cx.theme().primary)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().whitespace_normal().child(question_text(&pending.request)))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Tab to choose · Esc denies"))
                    .child(options),
            );
        }
        Some(list.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::question_text;
    use ados_agents::PermissionRequest;

    #[test]
    fn question_names_the_tool_call() {
        let request = |title: &str| PermissionRequest {
            session_id: "s".into(),
            tool_call_id: "t".into(),
            title: title.into(),
            options: vec![],
        };
        assert_eq!(question_text(&request("Write fake-note.txt")), "Allow: Write fake-note.txt?");
        assert_eq!(question_text(&request(" ")), "Allow the agent to run a tool?");
    }
}
