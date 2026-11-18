//! Drawing: toolbar, Day/Week hour grid, Agenda list, event form card.
use crate::{
    model::Occurrence,
    panel::*,
    store::ViewKind,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::Input,
    menu::DropdownMenu,
    h_flex, v_flex, ActiveTheme, IconName, Selectable,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use jiff::{civil::Date, tz::TimeZone, ToSpan};

/// Below this panel width the toolbar folds its view buttons into a menu.
const NARROW: f32 = 560.;

/// The skipped-events note as one short line, with a key that changes whenever the
/// counts do (a dismissed note comes back then): "Work: 1 event skipped".
pub(crate) fn skipped_note(subs: &[crate::subs::Loaded]) -> Option<(String, String)> {
    let parts: Vec<String> = subs.iter().filter(|s| s.skipped > 0)
        .map(|s| format!("{}: {} event{} skipped", s.name, s.skipped, if s.skipped == 1 { "" } else { "s" }))
        .collect();
    if parts.is_empty() { return None; }
    let line = parts.join(" · ");
    Some((line.clone(), line))
}

/// A popup menu that hides the browser's native page while it is open (as the shell's menus do).
fn guard(menu: gpui_kit::component::menu::PopupMenu, cx: &mut Context<gpui_kit::component::menu::PopupMenu>) -> gpui_kit::component::menu::PopupMenu {
    ados_core::overlay::OverlayGuard::open(cx);
    let open = std::rc::Rc::new(std::cell::Cell::new(true));
    let dismissed = open.clone();
    cx.subscribe(&cx.entity(), move |_, _, _: &DismissEvent, cx| {
        if dismissed.replace(false) { ados_core::overlay::OverlayGuard::close(cx); }
    }).detach();
    cx.on_release(move |_, cx| {
        if open.replace(false) { ados_core::overlay::OverlayGuard::close(cx); }
    }).detach();
    menu
}

/// `09:30–09:45 Standup · Room 2 (Work)` — one line for the agenda text and the assistant.
pub(crate) fn describe(o: &Occurrence, tz: &TimeZone) -> String {
    let mut s = format!("{} {}", when(o, tz), o.title);
    if let Some(l) = &o.location { s.push_str(&format!(" · {l}")); }
    if let Some(c) = &o.calendar { s.push_str(&format!(" ({c})")); }
    s
}

pub(crate) fn when(o: &Occurrence, tz: &TimeZone) -> String {
    if o.all_day { return "All day".into(); }
    let a = o.start.to_zoned(tz.clone());
    let b = o.end.to_zoned(tz.clone());
    if b.timestamp() <= a.timestamp() { return a.strftime("%H:%M").to_string(); }
    format!("{}–{}", a.strftime("%H:%M"), b.strftime("%H:%M"))
}

impl CalendarPanel {
    fn heading(&self) -> String {
        let (a, b) = self.range();
        match self.view {
            ViewKind::Day => a.strftime("%A %-d %B %Y").to_string(),
            _ => {
                let last = b.yesterday().unwrap_or(b);
                if a.month() == last.month() {
                    format!("{}–{} {}", a.day(), last.day(), last.strftime("%B %Y"))
                } else {
                    format!("{} – {}", a.strftime("%-d %b"), last.strftime("%-d %b %Y"))
                }
            }
        }
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tip = |title: &str, id: &str, cx: &App| ados_core::shortcuts::tooltip(title, id, cx);
        let unit = if self.view == ViewKind::Day { "day" } else { "week" };
        // Narrow panels fold Day/Week/Agenda into one menu button, so the heading
        // shrinks first and Refresh and + always stay on screen.
        let narrow = self.width.get() < NARROW;
        let view_button = |id: &'static str, label: &'static str, kind: ViewKind, sid: &str, cx: &mut Context<Self>| {
            ados_core::hints::marked(Button::new(id).label(label).ghost().h(px(32.)).flex_shrink_0().selected(self.view == kind)
                .tooltip(tip(&format!("{label} view"), sid, cx))
                .on_click(cx.listener(move |this, _, w, cx| this.set_view(kind, w, cx))))
        };
        let icon = |id: &'static str, icon: IconName| Button::new(id).icon(icon).ghost().size(px(32.)).flex_shrink_0();
        let bar = h_flex().h(px(48.)).flex_shrink_0().px_2().gap_1().items_center()
            .border_b_1().border_color(cx.theme().border)
            .child(ados_core::hints::marked(icon("cal-prev", IconName::ChevronLeft)
                .tooltip(tip(&format!("Previous {unit}"), "calendar.previous", cx))
                .on_click(cx.listener(|this, _, w, cx| { this.go(-1, cx); this.focus.focus(w, cx); }))))
            .child(ados_core::hints::marked(icon("cal-today", IconName::Calendar)
                .tooltip(tip("Today", "calendar.today", cx))
                .on_click(cx.listener(|this, _, w, cx| { this.go_today(cx); this.focus.focus(w, cx); }))))
            .child(ados_core::hints::marked(icon("cal-next", IconName::ChevronRight)
                .tooltip(tip(&format!("Next {unit}"), "calendar.next", cx))
                .on_click(cx.listener(|this, _, w, cx| { this.go(1, cx); this.focus.focus(w, cx); }))))
            .child(div().flex_1().min_w_0().px_1().truncate().text_color(cx.theme().foreground).child(self.heading()));
        let bar = if narrow {
            let label = match self.view { ViewKind::Day => "Day", ViewKind::Week => "Week", ViewKind::Agenda => "Agenda" };
            let (view, focus) = (self.view, self.focus.clone());
            bar.child(ados_core::hints::marked(Button::new("cal-views").label(label).icon(IconName::ChevronDown).ghost().h(px(32.)).flex_shrink_0()
                .tooltip("Change view")
                .dropdown_menu(move |menu, _, cx| {
                    guard(menu.menu_with_check("Day", view == ViewKind::Day, Box::new(DayView))
                        .menu_with_check("Week", view == ViewKind::Week, Box::new(WeekView))
                        .menu_with_check("Agenda", view == ViewKind::Agenda, Box::new(AgendaView))
                        .action_context(focus.clone()), cx)
                })))
        } else {
            bar.child(view_button("cal-day", "Day", ViewKind::Day, "calendar.day", cx))
                .child(view_button("cal-week", "Week", ViewKind::Week, "calendar.week", cx))
                .child(view_button("cal-agenda", "Agenda", ViewKind::Agenda, "calendar.agenda", cx))
        };
        bar.child(ados_core::hints::marked(icon("cal-refresh", IconName::RefreshCw).loading(self.refreshing)
                .tooltip(tip("Refresh subscriptions", "calendar.refresh", cx))
                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))))
            .child(ados_core::hints::marked(icon("cal-new", IconName::Plus)
                .tooltip(tip("New event", "calendar.new", cx))
                .on_click(cx.listener(|this, _, w, cx| this.open_new(w, cx)))))
    }

    fn status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut problems: Vec<String> = self.error.iter().chain(self.subs_error.iter()).cloned().collect();
        problems.extend(self.subs.iter().filter_map(|s| s.error.clone()));
        let skipped = skipped_note(&self.subs).filter(|(key, _)| self.skipped_dismissed.as_deref() != Some(key.as_str()));
        let muted = cx.theme().muted_foreground;
        v_flex().flex_shrink_0().px_3().text_sm()
            .when(!problems.is_empty() || skipped.is_some() || self.refreshing, |el| el.py_1().border_b_1().border_color(cx.theme().border))
            .children(problems.into_iter().map(|p| h_flex().gap_2().items_center()
                .child(div().size(px(6.)).flex_shrink_0().rounded_full().bg(rgb(ados_core::brand::RED)))
                .child(div().flex_1().min_w_0().text_color(cx.theme().foreground).child(p))))
            .when_some(skipped, |el, (key, line)| el.child(h_flex().gap_1().items_center().text_xs().text_color(muted)
                .child(div().id("cal-skipped").flex_1().min_w_0().truncate().child(line)
                    .tooltip(|w, cx| gpui_kit::component::tooltip::Tooltip::new(
                        "aDOs cannot read some repeat rules or time zones yet, so those events are not shown.").build(w, cx)))
                .child(ados_core::hints::marked(Button::new("cal-skipped-hide").icon(IconName::Close).ghost().size(px(32.)).flex_shrink_0()
                    .tooltip("Hide until the subscriptions change")
                    .on_click(cx.listener(move |this, _, _, cx| this.dismiss_skipped(key.clone(), cx)))))))
            .when(self.refreshing, |el| el.child(div().text_color(muted).child("Refreshing subscriptions…")))
    }

    fn agenda(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let today = self.today();
        let blue = rgb(ados_core::brand::BLUE);
        let (from, _) = self.range();
        let days: Vec<Date> = (0..7).filter_map(|i| from.checked_add(i.days()).ok()).collect();
        div().id("cal-agenda").track_scroll(&self.agenda_scroll).overflow_y_scroll().size_full().px_3().py_2()
            .children(days.into_iter().map(|d| {
                let items = self.occurrences(d, d.tomorrow().unwrap_or(d));
                let is_today = d == today;
                v_flex().pb_3()
                    .child(div().text_sm().py_1().font_weight(FontWeight::SEMIBOLD)
                        .text_color(if is_today { blue.into() } else { cx.theme().muted_foreground })
                        .child(if is_today { format!("Today · {}", d.strftime("%A %-d %B")) } else { d.strftime("%A %-d %B").to_string() }))
                    .when(items.is_empty(), |el| el.child(div().text_sm().pl_2().text_color(cx.theme().muted_foreground).child("Nothing scheduled")))
                    .children(items.into_iter().map(|o| {
                        let selected = self.selected.as_deref() == Some(key(&o).as_str());
                        let occ = o.clone();
                        h_flex().id(SharedString::from(format!("ag-{}", key(&o)))).min_h(px(32.)).px_2().gap_3().rounded(px(8.)).items_center()
                            .when(selected, |el| el.bg(cx.theme().primary.opacity(0.18)).border_1().border_color(blue))
                            .child(div().w(px(112.)).flex_shrink_0().whitespace_nowrap().text_sm().text_color(cx.theme().muted_foreground).child(when(&o, &self.tz)))
                            .child(div().flex_1().min_w_0().text_ellipsis().text_color(cx.theme().foreground).child(o.title.clone()))
                            .when_some(o.location.clone(), |el, l| el.child(div().text_sm().text_color(cx.theme().muted_foreground).child(l)))
                            .when_some(o.calendar.clone(), |el, c| el.child(div().text_xs().px_1p5().rounded(px(4.)).border_1()
                                .border_color(cx.theme().border).text_color(cx.theme().muted_foreground).child(c)))
                            .on_click(cx.listener(move |this, e: &ClickEvent, w, cx| {
                                this.select(&occ, w, cx);
                                if e.click_count() >= 2 { this.open_edit(w, cx); }
                            }))
                            .children(crate::links::event_url(&o).map(|url| {
                                ados_core::hints::marked(Button::new(SharedString::from(format!("ag-link-{}", key(&o))))
                                    .icon(gpui_kit::assets::IconName::ExternalLink).ghost().size(px(32.)).flex_shrink_0()
                                    .tooltip(format!("Open {url} in the Browser"))
                                    .on_click(move |_, window, cx| ados_core::links::open_url(&url, window, cx)))
                            }))
                            .child(ados_core::hints::mark())
                    }))
            }))
    }

    fn form_card(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let form = self.form.as_ref()?;
        let label = |text: &'static str| div().w(px(80.)).flex_shrink_0().whitespace_nowrap().text_sm().text_color(cx.theme().muted_foreground).child(text);
        let row = |name: &'static str, input: &Entity<gpui_kit::component::input::InputState>| {
            h_flex().gap_2().items_center().child(label(name)).child(div().flex_1().child(Input::new(input)))
        };
        Some(div().absolute().inset_0().flex().justify_center().items_start().pt(px(64.)).bg(cx.theme().background.opacity(0.5))
            .child(v_flex().key_context("CalendarForm").w(px(420.)).p_4().gap_2().rounded(px(10.))
                .bg(cx.theme().sidebar).border_1().border_color(cx.theme().border).shadow_md()
                .on_action(cx.listener(|this, _: &CancelForm, w, cx| this.cancel_form(w, cx)))
                .child(div().text_color(cx.theme().foreground).font_weight(FontWeight::SEMIBOLD)
                    .child(if form.editing.is_some() { "Edit event" } else { "New event" }))
                .child(row("Title", &form.title))
                .child(row("Start", &form.start))
                .child(row("Duration", &form.duration))
                .child(row("Location", &form.location))
                .child(row("Notes", &form.notes))
                .child(div().text_xs().text_color(cx.theme().muted_foreground)
                    .child("Start without a time makes an all-day event; then duration is days, like 1d."))
                .when_some(form.error.clone(), |el, e| el.child(h_flex().gap_2().items_center().text_sm()
                    .child(div().size(px(6.)).rounded_full().bg(rgb(ados_core::brand::RED)))
                    .child(div().text_color(cx.theme().foreground).child(e))))
                .child(h_flex().justify_end().gap_2()
                    .child(Button::new("cal-cancel").label("Cancel").ghost().h(px(32.)).tooltip("Close without saving · Esc")
                        .on_click(cx.listener(|this, _, w, cx| this.cancel_form(w, cx))))
                    .child(Button::new("cal-save").label("Save").primary().h(px(32.)).tooltip("Save · Enter")
                        .on_click(cx.listener(|this, _, w, cx| this.save_form(w, cx)))))))
    }
}

impl Render for CalendarPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (a, b) = self.range();
        let body = match self.view {
            ViewKind::Agenda => self.agenda(cx).into_any_element(),
            _ => {
                let days: Vec<Date> = (0..a.until(b).map(|s| s.get_days()).unwrap_or(1))
                    .filter_map(|i| a.checked_add((i as i64).days()).ok()).collect();
                let items = self.occurrences(a, b);
                self.grid(days, &items, cx).into_any_element()
            }
        };
        let width = self.width.clone();
        let view = cx.weak_entity();
        // Measure the panel: the toolbar, day headers and blocks pick shorter forms
        // when it is narrow. A changed width redraws once; an unchanged one does nothing.
        let measure = canvas(move |bounds, window, cx| {
            let w = f32::from(bounds.size.width);
            if (width.get() - w).abs() > 0.5 {
                width.set(w);
                window.defer(cx, move |_, cx| { let _ = view.update(cx, |_, cx| cx.notify()); });
            }
        }, |_, _, _, _| {}).absolute().size_full();
        div().id("calendar-panel").track_focus(&self.root_focus).relative().size_full().bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(measure)
            .child(v_flex().id("calendar").key_context("CalendarGrid").track_focus(&self.focus).size_full()
                .on_action(cx.listener(|this, _: &NewEvent, w, cx| this.open_new(w, cx)))
                .on_action(cx.listener(|this, _: &OpenLink, w, cx| this.open_link(w, cx)))
                .on_action(cx.listener(|this, _: &Today, _, cx| this.go_today(cx)))
                .on_action(cx.listener(|this, _: &Previous, _, cx| this.go(-1, cx)))
                .on_action(cx.listener(|this, _: &Next, _, cx| this.go(1, cx)))
                .on_action(cx.listener(|this, _: &SelectPrevious, w, cx| this.step_selection(-1, w, cx)))
                .on_action(cx.listener(|this, _: &SelectNext, w, cx| this.step_selection(1, w, cx)))
                .on_action(cx.listener(|this, _: &EditEvent, w, cx| this.open_edit(w, cx)))
                .on_action(cx.listener(|this, _: &DeleteEvent, w, cx| this.delete_selected(w, cx)))
                .on_action(cx.listener(|this, _: &UndoDelete, w, cx| this.undo_delete(w, cx)))
                .on_action(cx.listener(|this, _: &DayView, w, cx| this.set_view(ViewKind::Day, w, cx)))
                .on_action(cx.listener(|this, _: &WeekView, w, cx| this.set_view(ViewKind::Week, w, cx)))
                .on_action(cx.listener(|this, _: &AgendaView, w, cx| this.set_view(ViewKind::Agenda, w, cx)))
                .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, w, cx| {
                    if this.form.is_none() { this.focus.focus(w, cx); }
                }))
                .child(self.toolbar(cx))
                .child(self.status(cx))
                .child(div().flex_1().min_h_0().child(body)))
            .children(self.form_card(cx))
    }
}
