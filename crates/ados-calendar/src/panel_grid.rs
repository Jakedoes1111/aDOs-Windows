//! The Day and Week hour grid. Overlapping events cascade (each later one is
//! indented and drawn on top), so every title keeps its start readable; headers
//! and blocks pick shorter text as the panel narrows.
use crate::{model::Occurrence, panel::*, panel_view::when};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use jiff::{civil::Date, Timestamp};

/// Lanes for overlapping events: (lane, lanes in its cluster).
pub(crate) fn lanes(spans: &[(i64, i64)]) -> Vec<(usize, usize)> {
    let mut out = vec![(0, 1); spans.len()];
    let mut cluster: Vec<usize> = vec![];
    let mut lane_ends: Vec<i64> = vec![];
    let mut cluster_end = i64::MIN;
    let close = |cluster: &mut Vec<usize>, lane_ends: &mut Vec<i64>, out: &mut Vec<(usize, usize)>| {
        for &i in cluster.iter() { out[i].1 = lane_ends.len().max(1); }
        cluster.clear();
        lane_ends.clear();
    };
    for (i, &(s, e)) in spans.iter().enumerate() {
        let e = e.max(s + 1);
        if s >= cluster_end { close(&mut cluster, &mut lane_ends, &mut out); }
        let lane = lane_ends.iter().position(|&end| end <= s).unwrap_or_else(|| { lane_ends.push(0); lane_ends.len() - 1 });
        lane_ends[lane] = e;
        out[i].0 = lane;
        cluster.push(i);
        cluster_end = if cluster.len() == 1 { e } else { cluster_end.max(e) };
    }
    close(&mut cluster, &mut lane_ends, &mut out);
    out
}

/// Where lane `lane` of `n` sits in a day column `column` px wide: (left, width) in px.
/// Lane 0 spans the column; each later lane is indented and drawn over it, so an
/// earlier event's title shows in full above the later one's start and beside it.
/// The indent leaves the last lane at least 52px (a short word) when it can.
pub(crate) fn cascade(lane: usize, n: usize, column: f32) -> (f32, f32) {
    if n <= 1 { return (0., column); }
    let room = (column - 52.).max(0.);
    let step = (column * 0.4).min(room / (n - 1) as f32).max(column * 0.12);
    let left = lane as f32 * step;
    (left, (column - left).max(0.))
}

/// The day header for a column `width` px wide: "Mon 16", "M 16", then "16".
pub(crate) fn day_label(d: Date, width: f32) -> String {
    if width >= 64. {
        d.strftime("%a %-d").to_string()
    } else if width >= 40. {
        let day = d.strftime("%a").to_string();
        format!("{} {}", day.chars().next().unwrap_or(' '), d.day())
    } else {
        d.day().to_string()
    }
}

/// Width of the hour labels on the left of the grid.
const GUTTER: f32 = 56.;

impl CalendarPanel {
    /// `width` is the block's width in px: from 100px titles end in "…" when cut; below
    /// that they are clipped, since an ellipsis would leave nothing but "…".
    fn event_block(&self, o: &Occurrence, compact: bool, width: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.selected.as_deref() == Some(key(o).as_str());
        let blue = rgb(ados_core::brand::BLUE);
        let fill = if o.editable() { cx.theme().primary.opacity(0.22) } else { cx.theme().sidebar };
        let occ = o.clone();
        let tip = format!("{}\n{}", o.title, when(o, &self.tz));
        // The start of the title always shows; the tooltip has the whole title and time.
        let ellipsis = width >= 100.;
        div().id(SharedString::from(format!("ev-{}", key(o))))
            .size_full().overflow_hidden().rounded(px(6.)).px_1().text_xs()
            .when(compact, |el| el.flex().items_center())
            .when(!compact, |el| el.py_0p5())
            .bg(fill).border_1()
            .border_color(if selected { blue.into() } else { cx.theme().border })
            .when(selected, |el| el.border_2())
            .text_color(cx.theme().foreground)
            .child(div().font_weight(FontWeight::MEDIUM).whitespace_nowrap().overflow_hidden().line_height(px(16.))
                .when(ellipsis, |el| el.text_ellipsis())
                .child(o.title.clone()))
            .when(!compact, |el| el.child(div().text_color(cx.theme().muted_foreground).whitespace_nowrap().overflow_hidden()
                .when(ellipsis, |el| el.text_ellipsis())
                .child(match (&o.location, &o.calendar) {
                    (Some(l), _) => format!("{} · {l}", when(o, &self.tz)),
                    (None, Some(c)) => format!("{} · {c}", when(o, &self.tz)),
                    _ => when(o, &self.tz),
                })))
            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, e: &ClickEvent, w, cx| {
                this.select(&occ, w, cx);
                if e.click_count() >= 2 { this.open_edit(w, cx); }
            }))
            .child(ados_core::hints::mark())
    }

    pub(crate) fn grid(&self, days: Vec<Date>, items: &[Occurrence], cx: &mut Context<Self>) -> impl IntoElement {
        let today = self.today();
        let blue = rgb(ados_core::brand::BLUE);
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let background = cx.theme().background;
        let gutter = px(GUTTER);
        let column = (self.width.get() - GUTTER) / days.len().max(1) as f32;
        let header = h_flex().flex_shrink_0().border_b_1().border_color(border)
            .child(div().w(gutter).flex_shrink_0())
            .children(days.iter().map(|d| {
                let is_today = *d == today;
                div().flex_1().min_w_0().h(px(32.)).flex().items_center().justify_center().text_sm()
                    .overflow_hidden().whitespace_nowrap()
                    .border_l_1().border_color(border)
                    .text_color(if is_today { blue.into() } else { muted })
                    .when(is_today, |el| el.font_weight(FontWeight::SEMIBOLD).border_b_2().border_color(blue))
                    .child(day_label(*d, column))
            }));
        let all_day: Vec<Vec<&Occurrence>> = days.iter().map(|d| {
            let (s, e) = (crate::model::day_start(*d, &self.tz), crate::model::day_start(d.tomorrow().unwrap_or(*d), &self.tz));
            items.iter().filter(|o| o.all_day && o.start < e && o.end > s).collect()
        }).collect();
        let has_all_day = all_day.iter().any(|v| !v.is_empty());
        let all_day_row = h_flex().flex_shrink_0().border_b_1().border_color(border)
            .child(div().w(gutter).flex_shrink_0().px_1().text_xs().text_color(muted).whitespace_nowrap().child("All day"))
            .children(all_day.into_iter().map(|list| v_flex().flex_1().min_w_0().p_0p5().gap_0p5().border_l_1().border_color(border)
                .children(list.into_iter().map(|o| div().h(px(22.)).child(self.event_block(o, true, column - 4., cx))))));
        let now = Timestamp::now().to_zoned(self.tz.clone());
        let columns = days.iter().map(|d| {
            let start = crate::model::day_start(*d, &self.tz);
            let end = crate::model::day_start(d.tomorrow().unwrap_or(*d), &self.tz);
            let mut timed: Vec<&Occurrence> = items.iter().filter(|o| !o.all_day && o.start < end && (o.end > start || o.start >= start)).collect();
            timed.sort_by_key(|o| o.start);
            let spans: Vec<(i64, i64)> = timed.iter().map(|o| (o.start.max(start).as_second(), o.end.min(end).as_second().max(o.start.max(start).as_second() + 900))).collect();
            let layout = lanes(&spans);
            let is_today = *d == today;
            div().relative().flex_1().min_w_0().h_full().border_l_1().border_color(border)
                .when(is_today, |el| el.bg(cx.theme().primary.opacity(0.05)))
                .children((1..24).map(|h| div().absolute().top(px(h as f32 * HOUR)).left_0().right_0().h(px(1.)).bg(border.opacity(0.6))))
                .when(is_today, |el| {
                    let y = (now.hour() as f32 + now.minute() as f32 / 60.) * HOUR;
                    el.child(div().absolute().top(px(y)).left_0().right_0().h(px(2.)).bg(blue))
                })
                .children(timed.iter().zip(spans.iter()).zip(layout).map(|((o, &(s, e)), (lane, n))| {
                    let top = (s - start.as_second()) as f32 / 3600. * HOUR;
                    let height = ((e - s) as f32 / 3600. * HOUR).max(22.);
                    let (left, width) = cascade(lane, n, column - 1.);
                    // The background behind each block keeps a lower block's text from
                    // showing through the translucent fill of the one drawn over it.
                    div().absolute().top(px(top)).h(px(height))
                        .left(px(left)).w(px(width)).p(px(1.))
                        .child(div().size_full().rounded(px(6.)).bg(background).child(self.event_block(o, height < 36., width - 2., cx)))
                }))
        }).collect::<Vec<_>>();
        v_flex().size_full()
            .child(header)
            .when(has_all_day, |el| el.child(all_day_row))
            .child(div().id("cal-hours").track_scroll(&self.scroll).overflow_y_scroll().flex_1().min_h_0()
                // 8px above 00:00 so the first hour label is never cut under the header.
                .child(h_flex().w_full().h(px(24. * HOUR + 8.)).pt(px(8.))
                    .child(div().relative().w(gutter).flex_shrink_0().h_full()
                        .children((1..24).map(|h| div().absolute().top(px(h as f32 * HOUR - 8.)).right(px(8.)).text_xs().text_color(muted)
                            .child(format!("{h:02}:00")))))
                    .children(columns)))
    }
}

#[cfg(test)]
mod tests {
    use super::{cascade, day_label, lanes};

    #[test]
    fn overlapping_events_cascade_and_keep_room_for_titles() {
        // 09:00–09:45 and 09:30–09:45 overlap: two lanes.
        let layout = lanes(&[(9 * 3600, 9 * 3600 + 2700), (9 * 3600 + 1800, 9 * 3600 + 2700)]);
        assert_eq!(layout, vec![(0, 2), (1, 2)]);
        // A 75px week column: the later event keeps 52px, enough for "Team".
        assert_eq!(cascade(0, 2, 75.), (0., 75.));
        assert_eq!(cascade(1, 2, 75.), (23., 52.));
        // A wide Day column indents by 40%, so the earlier title shows beside it.
        assert_eq!(cascade(1, 2, 300.), (120., 180.));
        // Three lanes in 75px: the last still has 52px.
        assert_eq!(cascade(2, 3, 75.), (23., 52.));
        assert_eq!(cascade(0, 1, 75.), (0., 75.));
    }

    #[test]
    fn day_headers_shorten_with_the_column() {
        let d = jiff::civil::date(2026, 11, 16);
        assert_eq!(day_label(d, 120.), "Mon 16");
        assert_eq!(day_label(d, 50.), "M 16");
        assert_eq!(day_label(d, 30.), "16");
    }
}
