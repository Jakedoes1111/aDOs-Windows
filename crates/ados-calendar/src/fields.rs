//! Text fields of the event form: parse and format. Pure, so it is unit-tested.
use crate::model::day_start;
use jiff::{civil::{Date, Time}, tz::TimeZone, Timestamp, ToSpan};

/// `2026-11-13 15:00` (timed) or `2026-11-13` (all day).
pub fn parse_start(text: &str) -> Result<(Date, Option<Time>), String> {
    let text = text.trim();
    let bad = || "Start must look like 2026-11-13 15:00, or just 2026-11-13 for all day.".to_string();
    let (date, time) = match text.split_once([' ', 'T']) {
        Some((d, t)) => (d.trim(), Some(t.trim())),
        None => (text, None),
    };
    let date: Date = date.parse().map_err(|_| bad())?;
    let time = match time {
        None | Some("") => None,
        Some(t) => {
            let (h, m) = t.split_once(':').ok_or_else(bad)?;
            Some(Time::new(h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?, 0, 0).map_err(|_| bad())?)
        }
    };
    Ok((date, time))
}

/// Minutes from `90`, `45m`, `1h`, `1h30m`, `1:30`. Days (all day) from `1d`, `3` or `3d`.
pub fn parse_duration(text: &str, all_day: bool) -> Result<i64, String> {
    let t = text.trim().to_ascii_lowercase().replace(' ', "");
    if all_day {
        let days: i64 = t.trim_end_matches("days").trim_end_matches("day").trim_end_matches('d').parse()
            .map_err(|_| "Duration for an all-day event is a number of days, like 1d.".to_string())?;
        return if (1..=366).contains(&days) { Ok(days) } else { Err("An all-day event lasts 1 to 366 days.".into()) };
    }
    let bad = || "Duration must look like 30m, 1h, 1h30m or 1:30.".to_string();
    let minutes = if let Some((h, m)) = t.split_once(':') {
        h.parse::<i64>().map_err(|_| bad())? * 60 + m.parse::<i64>().map_err(|_| bad())?
    } else if t.chars().all(|c| c.is_ascii_digit()) && !t.is_empty() {
        t.parse::<i64>().map_err(|_| bad())?
    } else {
        let mut total = 0i64;
        let mut num = String::new();
        let mut any = false;
        for c in t.chars() {
            match c {
                '0'..='9' => num.push(c),
                'h' | 'm' => {
                    let n: i64 = num.parse().map_err(|_| bad())?;
                    num.clear();
                    total += if c == 'h' { n * 60 } else { n };
                    any = true;
                }
                _ => return Err(bad()),
            }
        }
        if !num.is_empty() || !any { return Err(bad()); }
        total
    };
    if (1..=7 * 24 * 60).contains(&minutes) { Ok(minutes) } else { Err("Duration must be between 1 minute and 7 days.".into()) }
}

/// Start and end instants plus the all-day flag, or a sentence about what to fix.
pub fn parse_times(start: &str, duration: &str, tz: &TimeZone) -> Result<(Timestamp, Timestamp, bool), String> {
    let (date, time) = parse_start(start)?;
    match time {
        None => {
            let days = parse_duration(duration, true)?;
            let end = date.checked_add(days.days()).map_err(|_| "That date is out of range.".to_string())?;
            Ok((day_start(date, tz), day_start(end, tz), true))
        }
        Some(time) => {
            let minutes = parse_duration(duration, false)?;
            let start = date.to_datetime(time).to_zoned(tz.clone())
                .map_err(|_| "That start time does not exist in this time zone.".to_string())?.timestamp();
            let end = Timestamp::from_second(start.as_second() + minutes * 60).map_err(|_| "That date is out of range.".to_string())?;
            Ok((start, end, false))
        }
    }
}

pub fn format_start(start: Timestamp, all_day: bool, tz: &TimeZone) -> String {
    let z = start.to_zoned(tz.clone());
    if all_day { z.strftime("%Y-%m-%d").to_string() } else { z.strftime("%Y-%m-%d %H:%M").to_string() }
}

pub fn format_duration(start: Timestamp, end: Timestamp, all_day: bool, tz: &TimeZone) -> String {
    if all_day {
        let a = start.to_zoned(tz.clone()).date();
        let b = end.to_zoned(tz.clone()).date();
        let days = a.until(b).map(|s| s.get_days()).unwrap_or(1).max(1);
        return format!("{days}d");
    }
    let minutes = ((end.as_second() - start.as_second()) / 60).max(0);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h{m}m"),
    }
}
