//! The Calendar dock panel: state, keys, palette actions, persistence.
use crate::{
    model::{self, local_zone, Event, Occurrence},
    store::{self, ViewKind, ViewState},
    subs,
};
use ados_core::{
    actions::ActionRegistry,
    hub::{ContextHub, Snapshot},
    ids,
};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::*;
use jiff::{civil::Date, tz::TimeZone, Timestamp, ToSpan};

actions!(ados_calendar, [
    NewEvent, Today, Previous, Next, SelectPrevious, SelectNext, EditEvent, DeleteEvent,
    UndoDelete, DayView, WeekView, AgendaView, Refresh, CancelForm, OpenLink
]);

/// Height of one hour in the Day and Week grids.
pub(crate) const HOUR: f32 = 44.;

#[derive(Default)]
struct Target {
    panel: Option<WeakEntity<CalendarPanel>>,
    /// "New event" ran before the panel existed: open the form once it is built.
    pending_new: bool,
}
impl Global for Target {}

fn with_panel(window: &mut Window, cx: &mut App, f: impl FnOnce(&mut CalendarPanel, &mut Window, &mut Context<CalendarPanel>)) -> bool {
    match cx.global::<Target>().panel.clone().and_then(|w| w.upgrade()) {
        Some(panel) => { panel.update(cx, |p, cx| f(p, window, cx)); true }
        None => false,
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(Target::default());
    use ados_core::shortcuts::bind;
    bind(cx, "calendar.new", NewEvent);
    bind(cx, "calendar.today", Today);
    bind(cx, "calendar.previous", Previous);
    bind(cx, "calendar.next", Next);
    bind(cx, "calendar.select_previous", SelectPrevious);
    bind(cx, "calendar.select_next", SelectNext);
    bind(cx, "calendar.edit", EditEvent);
    bind(cx, "calendar.delete", DeleteEvent);
    bind(cx, "calendar.undo_delete", UndoDelete);
    bind(cx, "calendar.day", DayView);
    bind(cx, "calendar.week", WeekView);
    bind(cx, "calendar.agenda", AgendaView);
    bind(cx, "calendar.refresh", Refresh);
    bind(cx, "calendar.cancel", CancelForm);
    bind(cx, "calendar.open_link", OpenLink);
    ados_core::actions::register("calendar.new", "New event", "calendar event meeting appointment schedule add", None, cx, |window, cx| {
        if !with_panel(window, cx, |p, w, cx| p.open_new(w, cx)) {
            // Let the shell add the module; the panel opens the form when it is built.
            cx.global_mut::<Target>().pending_new = true;
            if let Some(open) = ActionRegistry::all(cx).into_iter().find(|a| a.id.as_ref() == "shell.focus.calendar") {
                (open.run)(window, cx);
            }
        }
    });
    ados_core::actions::register("calendar.refresh", "Refresh calendar subscriptions", "calendar ics subscription sync update", None, cx, |window, cx| {
        with_panel(window, cx, |p, _, cx| p.refresh(cx));
    });
}

pub struct CalendarPanel {
    pub(crate) focus: FocusHandle,
    /// Tracked on the panel root (grid and event form), for the shell's active module.
    pub(crate) root_focus: FocusHandle,
    pub(crate) scroll: ScrollHandle,
    pub(crate) agenda_scroll: ScrollHandle,
    pub(crate) tz: TimeZone,
    pub(crate) view: ViewKind,
    pub(crate) anchor: Date,
    /// `calendar/id@start-seconds`: occurrences of one repeating event share an id.
    pub(crate) selected: Option<String>,
    pub(crate) native: Vec<Event>,
    pub(crate) subs: Vec<subs::Loaded>,
    pub(crate) subs_error: Option<String>,
    pub(crate) refreshing: bool,
    pub(crate) error: Option<String>,
    pub(crate) form: Option<crate::panel_form::Form>,
    pub(crate) deleted: Vec<(usize, Event)>,
    /// The panel's width at the last paint (see `Render`), for narrow layouts.
    pub(crate) width: std::rc::Rc<std::cell::Cell<f32>>,
    /// The skipped-events note the user hid; it shows again when the note changes.
    pub(crate) skipped_dismissed: Option<String>,
    _refresh: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// Calendar name too: two subscriptions may share a UID.
pub(crate) fn key(o: &Occurrence) -> String { format!("{}/{}@{}", o.calendar.as_deref().unwrap_or(""), o.id, o.start.as_second()) }

impl CalendarPanel {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tz = local_zone();
        let mut error = None;
        let state = store::load_view().unwrap_or_else(|e| {
            error = Some(format!("Could not restore the calendar view, showing this week: {e:#}"));
            ViewState::default()
        });
        let native = store::load_events().unwrap_or_else(|e| {
            error = Some(format!("Could not read your events ({e:#}). The file is left untouched; fix or move {} to start fresh.", store::events_file().display()));
            vec![]
        });
        let today = Timestamp::now().to_zoned(tz.clone()).date();
        let anchor = state.anchor.as_deref().and_then(|s| s.parse().ok()).unwrap_or(today);
        let focus = cx.focus_handle();
        let scroll = ScrollHandle::new();
        scroll.set_offset(point(px(0.), px(-7. * HOUR)));
        let subscriptions = vec![
            cx.on_focus_in(&focus, window, |this, _, cx| {
                ContextHub::set_active(ids::CALENDAR, cx);
                this.publish(cx);
            }),
        ];
        cx.global_mut::<Target>().panel = Some(cx.entity().downgrade());
        let mut panel = Self {
            focus, root_focus: cx.focus_handle(), scroll, agenda_scroll: ScrollHandle::new(), tz, view: state.view, anchor,
            selected: state.selected, native, subs: vec![], subs_error: None, refreshing: false,
            error, form: None, deleted: vec![], width: std::rc::Rc::new(std::cell::Cell::new(1000.)),
            skipped_dismissed: state.skipped_dismissed.clone(), _refresh: None, _subscriptions: subscriptions,
        };
        // Cached copies first so the view is never empty while fetching.
        match subs::list() {
            Ok(list) => panel.subs = list.iter().map(subs::cached).collect(),
            Err(e) => panel.subs_error = Some(e),
        }
        panel.refresh(cx);
        if std::mem::take(&mut cx.global_mut::<Target>().pending_new) {
            panel.open_new(window, cx);
        }
        panel.publish(cx);
        panel
    }

    /// Stable handle on the panel root: focus anywhere in Calendar is inside it.
    pub fn root_focus(&self) -> FocusHandle {
        self.root_focus.clone()
    }
    pub fn focus_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.form {
            Some(form) => form.title.read(cx).focus_handle(cx).focus(window, cx),
            None => self.focus.focus(window, cx),
        }
        ContextHub::set_active(ids::CALENDAR, cx);
        self.publish(cx);
        cx.notify();
    }

    pub(crate) fn today(&self) -> Date { Timestamp::now().to_zoned(self.tz.clone()).date() }

    /// First day and the day after the last one shown.
    pub(crate) fn range(&self) -> (Date, Date) {
        let start = match self.view {
            ViewKind::Week => self.anchor.checked_sub((self.anchor.weekday().to_monday_zero_offset() as i64).days()).unwrap_or(self.anchor),
            ViewKind::Day | ViewKind::Agenda => self.anchor,
        };
        let days = if self.view == ViewKind::Day { 1 } else { 7 };
        (start, start.checked_add(days.days()).unwrap_or(start))
    }

    pub(crate) fn occurrences(&self, from: Date, to: Date) -> Vec<Occurrence> {
        crate::collect(&self.native, &self.subs, model::day_start(from, &self.tz), model::day_start(to, &self.tz))
    }

    pub(crate) fn visible(&self) -> Vec<Occurrence> {
        let (a, b) = self.range();
        self.occurrences(a, b)
    }

    pub(crate) fn selected_occurrence(&self) -> Option<Occurrence> {
        let sel = self.selected.as_ref()?;
        self.visible().into_iter().find(|o| &key(o) == sel)
    }

    pub(crate) fn persist(&mut self) {
        let state = ViewState {
            view: self.view, anchor: Some(self.anchor.to_string()), selected: self.selected.clone(),
            skipped_dismissed: self.skipped_dismissed.clone(),
        };
        if let Err(e) = store::save_view(&state) {
            self.error = Some(format!("Could not remember the calendar view: {e:#}"));
        }
    }

    pub(crate) fn save_native(&mut self) -> bool {
        match store::save_events(&self.native) {
            Ok(()) => true,
            Err(e) => { self.error = Some(format!("Could not save your events: {e:#}. Your change is kept on screen; try again.")); false }
        }
    }

    /// Fetch every subscription in the background. Only on open and on Refresh.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let list = match subs::list() {
            Ok(list) => { self.subs_error = None; list }
            Err(e) => { self.subs_error = Some(e); self.subs.clear(); cx.notify(); return; }
        };
        if list.is_empty() { self.subs.clear(); cx.notify(); return; }
        self.refreshing = true;
        let task = cx.background_executor().spawn(async move { list.iter().map(subs::refresh).collect::<Vec<_>>() });
        self._refresh = Some(cx.spawn(async move |weak, cx| {
            let loaded = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.subs = loaded;
                this.refreshing = false;
                this.publish(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn set_view(&mut self, view: ViewKind, window: &mut Window, cx: &mut Context<Self>) {
        self.view = view;
        self.persist();
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn dismiss_skipped(&mut self, key: String, cx: &mut Context<Self>) {
        self.skipped_dismissed = Some(key);
        self.persist();
        cx.notify();
    }

    pub(crate) fn go(&mut self, steps: i64, cx: &mut Context<Self>) {
        let days = if self.view == ViewKind::Day { steps } else { steps * 7 };
        if let Ok(d) = self.anchor.checked_add(days.days()) { self.anchor = d; }
        self.persist();
        cx.notify();
    }

    pub(crate) fn go_today(&mut self, cx: &mut Context<Self>) {
        self.anchor = self.today();
        self.persist();
        cx.notify();
    }

    pub(crate) fn select(&mut self, occ: &Occurrence, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(key(occ));
        self.persist();
        self.reveal(occ);
        self.publish(cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn step_selection(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let all = self.visible();
        if all.is_empty() { return; }
        let current = self.selected.as_ref().and_then(|s| all.iter().position(|o| &key(o) == s));
        let next = match current {
            None if delta >= 0 => 0,
            None => all.len() - 1,
            Some(i) => (i as i32 + delta).clamp(0, all.len() as i32 - 1) as usize,
        };
        let occ = all[next].clone();
        self.select(&occ, window, cx);
    }

    /// Scroll the hour grid so a keyboard-selected event is on screen.
    fn reveal(&self, occ: &Occurrence) {
        if occ.all_day || self.view == ViewKind::Agenda { return; }
        let t = occ.start.to_zoned(self.tz.clone());
        let y = (t.hour() as f32 + t.minute() as f32 / 60.) * HOUR;
        let view_h = f32::from(self.scroll.bounds().size.height);
        let top = -f32::from(self.scroll.offset().y);
        if view_h > 0. && (y < top || y + HOUR > top + view_h) {
            self.scroll.set_offset(point(px(0.), px(-(y - HOUR).max(0.))));
        }
    }

    /// Today's agenda for the assistant, plus the selected event.
    pub(crate) fn publish(&self, cx: &mut App) {
        let today = self.today();
        let items = self.occurrences(today, today.tomorrow().unwrap_or(today));
        let mut text = format!("Today, {}\n", today.strftime("%A %-d %B %Y"));
        if items.is_empty() { text.push_str("Nothing scheduled today.\n"); }
        for o in &items { text.push_str(&crate::panel_view::describe(o, &self.tz)); text.push('\n'); }
        let selection = self.selected_occurrence().map(|o| crate::panel_view::describe(&o, &self.tz).into());
        ContextHub::publish(Snapshot {
            module: ids::CALENDAR.into(), title: "Calendar".into(), location: None, text: text.into(), selection,
        }, cx);
    }
}

impl EventEmitter<PanelEvent> for CalendarPanel {}
impl Focusable for CalendarPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.form {
            Some(form) => form.title.read(cx).focus_handle(cx),
            None => self.focus.clone(),
        }
    }
}
impl Panel for CalendarPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { "Calendar" }
    fn inner_padding(&self, _: &App) -> bool { false }
}
impl BasePanel for CalendarPanel {
    fn panel_name(&self) -> &'static str { ids::CALENDAR }
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if active { self.publish(cx); }
    }
}
