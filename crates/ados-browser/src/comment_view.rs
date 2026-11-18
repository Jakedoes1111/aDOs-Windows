//! Comment mode on screen: the toolbar controls and the card above the page. The card
//! sits in the panel's own layout (the page shrinks under it), so the native page never
//! covers it, and it holds the keyboard until Enter, Ctrl+Enter or Esc.
use super::*;
use gpui_kit::component::{
    button::{Button, ButtonVariants}, input::{Escape, Input}, ActiveTheme, Icon, IconName,
};

// Lucide icons outside gpui-kit's default bundle, embedded here (the shell's asset source is not ours).
fn comment_icon() -> Icon { Icon::default().data(include_bytes!("../icons/message-square-plus.svg")) }
fn send_icon() -> Icon { Icon::default().data(include_bytes!("../icons/send.svg")) }
use ados_core::hints::marked;

impl BrowserPanel {
    /// The toolbar's comment icon, and "Send N comments" while some wait.
    pub(super) fn comment_buttons(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![marked(Button::new("browser-comment").tab_index(2).icon(comment_icon()).ghost().size(px(32.))
            .toggled(self.commenting())
            .tooltip(ados_core::shortcuts::tooltip(if self.commenting() { "Stop commenting" } else { "Comment on the page" }, "browser.comment", cx))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_comment(window, cx)))).into_any_element()];
        let waiting = self.comment.queue.len();
        if waiting > 0 {
            let label = if waiting == 1 { "Send 1 comment".to_string() } else { format!("Send {waiting} comments") };
            out.push(marked(Button::new("browser-comment-send").tab_index(2).icon(send_icon()).primary().h(px(32.))
                .label(label).tooltip("Send the waiting comments to the assistant")
                .on_click(cx.listener(|this, _, window, cx| this.send_queue(window, cx)))).into_any_element());
        }
        out
    }

    /// The strip between toolbar and page while commenting: a hint line, or the card.
    pub(super) fn comment_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.commenting() { return None; }
        let theme = cx.theme();
        let (muted, fg, border, panel) = (theme.muted_foreground, theme.foreground, theme.border, theme.popover);
        let Some(card) = &self.comment.card else {
            let line = if self.comment.labels.is_some() {
                "Type a label to pick it · Esc hides the labels".to_string()
            } else {
                let keys = hints::config(cx)["hints"].as_array().and_then(|a| a.first()).and_then(|v| v.as_str())
                    .map(str::to_uppercase).unwrap_or_else(|| "F".into());
                format!("Click what you mean, or select text first · {keys} labels the page · Esc leaves")
            };
            return Some(div().flex().items_center().gap_2().px_3().h(px(32.)).flex_shrink_0()
                .border_b_1().border_color(border).text_sm().text_color(muted)
                .child(comment_icon().size(px(16.)).text_color(rgb(ados_core::brand::BLUE)))
                .child(line).into_any_element());
        };
        let waiting = self.comment.queue.len();
        let footer = if waiting == 0 {
            "Enter sends · Ctrl+Enter adds to a batch · Esc cancels".to_string()
        } else {
            format!("Enter sends all {} · Ctrl+Enter adds to the batch · Esc cancels", waiting + 1)
        };
        Some(div().id("browser-comment-card").p_2().flex_shrink_0().border_b_1().border_color(border)
            .key_context("BrowserComment")
            .capture_action(cx.listener(|this, _: &Escape, window, cx| { this.cancel_card(window, cx); cx.stop_propagation(); }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" { this.cancel_card(window, cx); cx.stop_propagation(); }
            }))
            .child(div().flex().flex_col().gap_2().p_3().rounded(px(10.)).border_1().border_color(border).bg(panel).text_color(fg)
                .child(div().flex().items_center().gap_2().text_sm()
                    .child(div().flex_shrink_0().min_w(px(22.)).h(px(22.)).px_1().rounded(px(11.)).bg(rgb(ados_core::brand::BLUE))
                        .flex().items_center().justify_center().text_xs().text_color(gpui_kit::white()).child(card.number.to_string()))
                    .child(div().flex_1().min_w_0().truncate().text_color(muted).child(card.picked.describe())))
                .child(div().flex().items_center().gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&card.input).w_full()))
                    .child(Button::new("browser-comment-queue").icon(IconName::Plus).ghost().size(px(32.))
                        .tooltip("Add to batch · Ctrl+Enter")
                        .on_click(cx.listener(|this, _, window, cx| this.queue_card(window, cx))))
                    .child(Button::new("browser-comment-go").icon(send_icon()).primary().size(px(32.))
                        .tooltip(if waiting == 0 { "Send to assistant · Enter".to_string() } else { format!("Send {} comments · Enter", waiting + 1) })
                        .on_click(cx.listener(|this, _, window, cx| this.send_card(window, cx)))))
                .child(div().text_xs().text_color(muted).child(footer)))
            .into_any_element())
    }
}
