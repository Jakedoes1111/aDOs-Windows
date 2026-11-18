//! Events, repeat rules and their expansion into occurrences. Pure: no I/O, no UI.
use jiff::{
    civil::{Date, DateTime, Weekday},
    tz::TimeZone,
    Timestamp, ToSpan,
};
use serde::{Deserialize, Serialize};

/// One event as stored: UTC instants plus the zone it was made in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub start: Timestamp,
    /// Exclusive. For all-day events: midnight after the last day.
    pub end: Timestamp,
    #[serde(default)]
    pub all_day: bool,
    /// IANA zone name, e.g. "Europe/London".
    pub zone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Freq {
    Daily,
    Weekly,
}

/// The `RRULE` subset aDOs understands: daily or weekly, with interval, count, until, weekdays.
#[derive(Debug, Clone, PartialEq)]
pub struct Repeat {
    pub freq: Freq,
    pub interval: u32,
    pub count: Option<u32>,
    pub until: Option<Timestamp>,
    pub by_day: Vec<Weekday>,
}

/// An event with its optional repeat, as read from a subscription.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub event: Event,
    pub repeat: Option<Repeat>,
    pub exdates: Vec<Timestamp>,
}

/// One concrete appearance of an event in time.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    /// The event id (native) or UID (subscription).
    pub id: String,
    pub title: String,
    pub start: Timestamp,
    pub end: Timestamp,
    pub all_day: bool,
    pub location: Option<String>,
    pub notes: Option<String>,
    /// Subscription name, or `None` for native events (the only editable ones).
    pub calendar: Option<String>,
}

impl Occurrence {
    pub fn editable(&self) -> bool {
        self.calendar.is_none()
    }
}

/// The machine's zone. Falls back to UTC when it cannot be found.
pub fn local_zone() -> TimeZone {
    TimeZone::try_system().unwrap_or(TimeZone::UTC)
}

pub fn zone_name(tz: &TimeZone) -> String {
    tz.iana_name().unwrap_or("UTC").to_string()
}

pub fn zone_named(name: &str, fallback: &TimeZone) -> TimeZone {
    TimeZone::get(name).unwrap_or_else(|_| fallback.clone())
}

/// Start of `date` in `tz` as an instant (the first valid moment of that day).
pub fn day_start(date: Date, tz: &TimeZone) -> Timestamp {
    date.to_zoned(tz.clone()).map(|z| z.timestamp()).unwrap_or_else(|_| {
        tz.to_timestamp(date.to_datetime(jiff::civil::Time::midnight())).unwrap_or(Timestamp::UNIX_EPOCH)
    })
}

fn at(dt: DateTime, tz: &TimeZone) -> Option<Timestamp> {
    dt.to_zoned(tz.clone()).ok().map(|z| z.timestamp())
}

/// Hard stop on expansion so a malformed rule can never hang the UI.
const MAX_STEPS: usize = 20_000;

/// Every occurrence of `series` that overlaps `[from, to)`.
pub fn expand(series: &Series, from: Timestamp, to: Timestamp, calendar: Option<&str>, local: &TimeZone) -> Vec<Occurrence> {
    let ev = &series.event;
    let make = |start: Timestamp, end: Timestamp| Occurrence {
        id: ev.id.clone(),
        title: ev.title.clone(),
        start,
        end,
        all_day: ev.all_day,
        location: ev.location.clone(),
        notes: ev.notes.clone(),
        calendar: calendar.map(str::to_string),
    };
    let overlaps = |s: Timestamp, e: Timestamp| s < to && (e > from || (e <= s && s >= from));
    let Some(rule) = &series.repeat else {
        return if overlaps(ev.start, ev.end) { vec![make(ev.start, ev.end)] } else { vec![] };
    };
    let tz = zone_named(&ev.zone, local);
    let first = tz.to_datetime(ev.start);
    let end_civil = tz.to_datetime(ev.end);
    let all_day_len = first.date().until(end_civil.date()).ok().map(|s| s.get_days()).unwrap_or(1).max(1);
    let seconds = ev.end.as_second() - ev.start.as_second();
    let interval = rule.interval.max(1) as i64;
    let mut out = vec![];
    let mut emitted = 0u32;
    let mut emit = |date: Date| -> bool {
        // Returns false when the series is over.
        let Some(start) = at(date.to_datetime(first.time()), &tz) else { return true };
        if rule.until.is_some_and(|u| start > u) || rule.count.is_some_and(|c| emitted >= c) || start >= to {
            return false;
        }
        emitted += 1;
        if series.exdates.contains(&start) {
            return true;
        }
        let end = if ev.all_day {
            date.checked_add(all_day_len.days()).ok().map(|d| day_start(d, &tz)).unwrap_or(start)
        } else {
            Timestamp::from_second(start.as_second() + seconds).unwrap_or(start)
        };
        if overlaps(start, end) {
            out.push(make(start, end));
        }
        true
    };
    let start_date = first.date();
    match rule.freq {
        Freq::Daily => {
            for step in 0..MAX_STEPS as i64 {
                let Ok(date) = start_date.checked_add((step * interval).days()) else { break };
                if !emit(date) { break; }
            }
        }
        Freq::Weekly => {
            let mut days: Vec<i8> = if rule.by_day.is_empty() {
                vec![start_date.weekday().to_monday_zero_offset()]
            } else {
                rule.by_day.iter().map(|d| d.to_monday_zero_offset()).collect()
            };
            days.sort();
            days.dedup();
            let monday = start_date.checked_sub((start_date.weekday().to_monday_zero_offset() as i64).days()).unwrap_or(start_date);
            'weeks: for week in 0..(MAX_STEPS / 7) as i64 {
                let Ok(week_start) = monday.checked_add((week * interval * 7).days()) else { break };
                for offset in &days {
                    let Ok(date) = week_start.checked_add((*offset as i64).days()) else { break 'weeks };
                    if date < start_date { continue; }
                    if !emit(date) { break 'weeks; }
                }
            }
        }
    }
    out
}

/// Short id for new native events: time-based, unique enough for one person's calendar.
pub fn new_id() -> String {
    let now = Timestamp::now();
    format!("ev-{}-{:09}", now.as_second(), now.subsec_nanosecond())
}
