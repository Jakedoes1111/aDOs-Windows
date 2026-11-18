//! New/edit event form, delete and undo. Only native events change.
use crate::{fields, model::{self, zone_name, Event}, panel::{key, CalendarPanel}};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{InputEvent, InputState},
    notification::Notification,
    WindowExt,
};
use gpui_kit::*;
use jiff::Timestamp;

pub(crate) struct Form {
    /// The native event being edited; `None` for a new one.
    pub editing: Option<String>,
    pub title: Entity<InputState>,
    pub start: Entity<InputState>,
    pub duration: Entity<InputState>,
    pub location: Entity<InputState>,
    pub notes: Entity<InputState>,
    pub error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Form {
    pub fn inputs(&self) -> [&Entity<InputState>; 5] {
        [&self.title, &self.start, &self.duration, &self.location, &self.notes]
    }
}

impl CalendarPanel {
    fn open_form(&mut self, editing: Option<String>, values: [String; 5], window: &mut Window, cx: &mut Context<Self>) {
        let placeholders = ["Title", "2026-11-13 15:00", "1h", "Location (optional)", "Notes (optional)"];
        let inputs: Vec<Entity<InputState>> = values.into_iter().zip(placeholders)
            .map(|(value, hint)| cx.new(|cx| InputState::new(window, cx).placeholder(hint).default_value(value)))
            .collect();
        let subscriptions = inputs.iter().map(|input| {
            cx.subscribe_in(input, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event { this.save_form(window, cx); }
            })
        }).collect();
        let [title, start, duration, location, notes]: [Entity<InputState>; 5] = match inputs.try_into() {
            Ok(a) => a,
            Err(_) => return,
        };
        title.read(cx).focus_handle(cx).focus(window, cx);
        self.form = Some(Form { editing, title, start, duration, location, notes, error: None, _subscriptions: subscriptions });
        cx.notify();
    }

    pub(crate) fn open_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Timestamp::now().to_zoned(self.tz.clone());
        // Today: the next whole hour. Another day: 09:00.
        let start = if self.anchor == now.date() && now.hour() < 23 {
            format!("{} {:02}:00", self.anchor, now.hour() + 1)
        } else {
            format!("{} 09:00", self.anchor)
        };
        self.open_form(None, [String::new(), start, "1h".into(), String::new(), String::new()], window, cx);
    }

    pub(crate) fn open_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(occ) = self.selected_occurrence() else { return };
        if !occ.editable() {
            self.error = Some(format!("“{}” comes from {}, which is read-only here.", occ.title, occ.calendar.unwrap_or_default()));
            cx.notify();
            return;
        }
        let Some(ev) = self.native.iter().find(|e| e.id == occ.id).cloned() else { return };
        self.error = None;
        self.open_form(Some(ev.id.clone()), [
            ev.title.clone(),
            fields::format_start(ev.start, ev.all_day, &self.tz),
            fields::format_duration(ev.start, ev.end, ev.all_day, &self.tz),
            ev.location.clone().unwrap_or_default(),
            ev.notes.clone().unwrap_or_default(),
        ], window, cx);
    }

    pub(crate) fn cancel_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn save_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &self.form else { return };
        let [title, start, duration, location, notes] = form.inputs().map(|i| i.read(cx).value().trim().to_string());
        let editing = form.editing.clone();
        let result = if title.is_empty() {
            Err("Give the event a title.".to_string())
        } else {
            fields::parse_times(&start, &duration, &self.tz)
        };
        let (start, end, all_day) = match result {
            Ok(times) => times,
            Err(why) => {
                if let Some(form) = self.form.as_mut() { form.error = Some(why); }
                cx.notify();
                return;
            }
        };
        let optional = |s: String| (!s.is_empty()).then_some(s);
        let event = Event {
            id: editing.clone().unwrap_or_else(model::new_id),
            title, start, end, all_day, zone: zone_name(&self.tz),
            location: optional(location), notes: optional(notes),
        };
        let previous = self.native.clone();
        match self.native.iter_mut().find(|e| Some(&e.id) == editing.as_ref()) {
            Some(slot) => *slot = event.clone(),
            None => self.native.push(event.clone()),
        }
        if !self.save_native() {
            self.native = previous;
            if let Some(form) = self.form.as_mut() { form.error = self.error.take(); }
            cx.notify();
            return;
        }
        self.error = None;
        self.form = None;
        // Show the day the event is on, and select it.
        self.anchor = start.to_zoned(self.tz.clone()).date();
        match self.visible().into_iter().find(|o| o.id == event.id) {
            Some(o) => self.select(&o, window, cx),
            None => { self.persist(); self.focus.focus(window, cx); }
        }
        self.publish(cx);
        cx.notify();
    }

    pub(crate) fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(occ) = self.selected_occurrence() else { return };
        if !occ.editable() {
            self.error = Some(format!("“{}” comes from {}, which is read-only here.", occ.title, occ.calendar.unwrap_or_default()));
            cx.notify();
            return;
        }
        let Some(index) = self.native.iter().position(|e| e.id == occ.id) else { return };
        let removed = self.native.remove(index);
        if !self.save_native() {
            self.native.insert(index, removed);
            cx.notify();
            return;
        }
        self.error = None;
        // Keep the keyboard on a neighbour so Delete can be pressed again.
        let all = self.visible();
        let pos = all.iter().position(|o| o.start >= occ.start).unwrap_or(all.len()).min(all.len().saturating_sub(1));
        self.selected = all.get(pos).map(key);
        self.persist();
        self.deleted.push((index, removed.clone()));
        let weak = cx.entity().downgrade();
        let undo_weak = weak.clone();
        ados_core::notifications::remember_undo(move |window, cx| {
            let _ = undo_weak.update(cx, |p, cx| p.undo_delete(window, cx));
        }, cx);
        window.push_notification(
            Notification::new().message(format!("Deleted “{}”.", removed.title)).content(|_, _, _| ados_core::hints::cover_padded(ados_core::hints::Layer::Float, px(16.)).into_any_element())
                .id::<CalendarPanel>()
                .autohide(false)
                .action(move |_, _, _| {
                    let weak = weak.clone();
                    Button::new("undo-deleted-event").label("Undo").ghost().child(ados_core::hints::mark_in(ados_core::hints::Layer::Float))
                        .on_click(move |_, window, cx| { let _ = weak.update(cx, |p, cx| p.undo_delete(window, cx)); })
                }),
            cx,
        );
        self.publish(cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn undo_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((index, event)) = self.deleted.pop() else { return };
        let at = index.min(self.native.len());
        self.native.insert(at, event.clone());
        if !self.save_native() {
            self.native.remove(at);
            self.deleted.push((index, event));
            cx.notify();
            return;
        }
        if self.deleted.is_empty() { window.remove_notification::<CalendarPanel>(cx); }
        self.anchor = event.start.to_zoned(self.tz.clone()).date();
        self.persist();
        let occ = self.visible().into_iter().find(|o| o.id == event.id);
        match occ {
            Some(o) => self.select(&o, window, cx),
            None => self.focus.focus(window, cx),
        }
        self.publish(cx);
        cx.notify();
    }
}
