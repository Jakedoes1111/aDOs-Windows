use super::Assistant;
use gpui_kit::component::{
    clipboard::Clipboard,
    text::{TextView, TextViewState},
    ActiveTheme, Sizable,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

pub(super) struct Row {
    pub user: bool,
    pub text: String,
    /// The agent's visible reasoning for this reply, shown muted above it.
    pub thought: String,
    pub view: Entity<TextViewState>,
}
impl Row {
    fn new(user: bool, text: String, cx: &mut App) -> Self {
        let view = cx.new(|cx| TextViewState::markdown(&crate::literal_tildes(&text), cx));
        Self { user, text, thought: String::new(), view }
    }
}
/// The end of a long thought, so the card shows what the agent is thinking now.
fn last_chars(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.trim().to_owned();
    }
    format!("…{}", text.chars().skip(count - limit).collect::<String>().trim())
}

/// Estimate how many wrapped rows `text` needs in the 440px card. Deliberately
/// conservative: fewer characters per row than the real width, so the card
/// reserves at least the space the renderer will need.
pub(super) fn estimated_rows(text: &str) -> f32 {
    text.lines()
        .map(|line| (line.chars().count() / 45 + 1) as f32)
        .sum::<f32>()
        .max(1.)
}
impl Assistant {
    pub(super) fn append_row(&mut self, user: bool, text: String, cx: &mut Context<Self>) {
        self.rows.push(Row::new(user, text, cx));
        self.reading_scroll.scroll_to_bottom();
    }
    pub(super) fn reset_reading(&mut self, _cx: &mut Context<Self>) {
        self.rows.clear();
        self.partial.clear();
        self.streaming_row = None;
        self.reading_scroll.set_offset(point(px(0.), px(0.)));
    }
    pub(super) fn message_started(&mut self, cx: &mut Context<Self>) {
        self.partial.clear();
        self.replaying_user = false;
        self.append_row(false, String::new(), cx);
        self.streaming_row = Some(self.rows.len() - 1);
    }
    /// A chunk of a replayed user message (`session/load`).
    pub(super) fn user_chunk(&mut self, text: &str, cx: &mut Context<Self>) {
        self.streaming_row = None;
        match self.rows.last_mut() {
            Some(row) if row.user && self.replaying_user => row.text.push_str(text),
            _ => self.append_row(true, text.to_owned(), cx),
        }
        self.replaying_user = true;
    }
    pub(super) fn thought_delta(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.streaming_row.is_none() {
            self.message_started(cx);
        }
        if let Some(row) = self.streaming_row.and_then(|index| self.rows.get_mut(index)) {
            row.thought.push_str(text);
        }
    }
    pub(super) fn text_delta(&mut self, index: usize, text: &str, cx: &mut Context<Self>) {
        self.replaying_user = false;
        if self.streaming_row.is_none() {
            self.message_started(cx);
        }
        self.partial.entry(index).or_default().push_str(text);
        self.update_partial(cx);
    }
    fn update_partial(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.streaming_row {
            if let Some(row) = self.rows.get_mut(index) {
                let joined = self
                    .partial
                    .values()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let (notices, reply) = crate::split_notice(&joined);
                if let Some(notice) = notices.last() {
                    self.notice = Some(notice.clone());
                }
                row.text = reply;
                row.view.update(cx, |view, cx| view.set_text(&crate::literal_tildes(&row.text), cx));
                if self.reading_scroll.offset().y <= -self.reading_scroll.max_offset().y + px(24.) {
                    self.reading_scroll.scroll_to_bottom();
                }
            }
        }
    }
    pub(super) fn reading(&self, cx: &mut Context<Self>) -> AnyElement {
        div().id("assistant-messages").size_full().overflow_y_scroll()
            .track_scroll(&self.reading_scroll)
            .child(div().flex().flex_col().w_full().children(
                self.rows.iter().enumerate().map(|(index, row)| div()
                        .w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .px_3()
                        .py_2()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when(row.user, |el| el.bg(cx.theme().muted.opacity(0.3)))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(if row.user { "You".to_owned() } else { self.backend_name() }),
                        )
                        .when(!row.user && !row.thought.is_empty(), |el| el.child(
                            div()
                                .text_xs()
                                .italic()
                                .text_color(cx.theme().muted_foreground)
                                .whitespace_normal()
                                .child(last_chars(&row.thought, 240)),
                        ))
                        .when(row.user, |el| el.child(
                            div().w_full().min_w_0().flex_shrink_0().child(ados_core::links::linked_text(&format!("assistant-user-{index}"), &row.text, cx))
                        ))
                        .when(!row.user, |el| el.child(
                            TextView::new(&row.view)
                                .selectable(true)
                                .scrollable(false)
                                .on_link_click(|url, _, window, cx| ados_core::links::open_url(url, window, cx))
                                .code_block_actions(|code, _, _| {
                                    Clipboard::new("code-copy")
                                        .with_size(px(32.))
                                        .value(code.code().to_string())
                                        .tooltip("Copy code")
                                }),
                        ))
                        .when(!row.user && !row.text.is_empty(), |el| {
                            el.child(
                                div().flex().justify_end().child(
                                    Clipboard::new(("reply-copy", index))
                                        .with_size(px(32.))
                                        .value(row.text.clone())
                                        .tooltip("Copy reply"),
                                ),
                            )
                        })
                        .into_any_element()
                )
            ))
            .into_any_element()
    }
    pub(super) fn content_height(&self, line_height: f32) -> f32 {
        self.rows
            .iter()
            .map(|r| (estimated_rows(&r.text) + if r.thought.is_empty() { 0. } else { 2. }) * line_height + if r.user { 48. } else { 80. })
            .sum::<f32>()
            .max(65.)
    }
}
