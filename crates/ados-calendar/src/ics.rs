//! The `.ics` subset aDOs reads: VEVENTs with DTSTART/DTEND/DURATION, SUMMARY, LOCATION,
//! DESCRIPTION, daily/weekly RRULE, EXDATE and RECURRENCE-ID overrides. Anything else is skipped
//! and counted, never guessed.
use crate::model::{day_start, zone_name, Event, Freq, Repeat, Series};
use jiff::{
    civil::{Date, DateTime, Time, Weekday},
    tz::TimeZone,
    Timestamp, ToSpan,
};

#[derive(Debug, Default)]
pub struct Parsed {
    pub series: Vec<Series>,
    /// Events left out because something in them is outside the subset.
    pub skipped: usize,
    /// False when the text has no `BEGIN:VCALENDAR` at all.
    pub is_calendar: bool,
}

/// A property line: name, parameters, value.
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Prop {
    fn param(&self, key: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// Undo RFC 5545 line folding (a line starting with a space or tab continues the previous one).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = vec![];
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(line.to_string());
    }
    lines
}

fn split_prop(line: &str) -> Option<Prop> {
    // The value starts at the first ':' that is not inside a quoted parameter.
    let mut quoted = false;
    let colon = line.char_indices().find(|&(_, c)| {
        if c == '"' { quoted = !quoted; }
        c == ':' && !quoted
    })?.0;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts.filter_map(|p| {
        let (k, v) = p.split_once('=')?;
        Some((k.trim().to_ascii_uppercase(), v.trim_matches('"').to_string()))
    }).collect();
    Some(Prop { name, params, value: value.to_string() })
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// A DTSTART-like value: an instant, or a whole day.
#[derive(Debug, Clone, Copy, PartialEq)]
enum When {
    At(Timestamp),
    Day(Date),
}

fn parse_date(s: &str) -> Option<Date> {
    if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) { return None; }
    Date::new(s[0..4].parse().ok()?, s[4..6].parse().ok()?, s[6..8].parse().ok()?).ok()
}

fn parse_datetime(s: &str) -> Option<(DateTime, bool)> {
    let (date, time) = s.split_once('T')?;
    let (time, utc) = match time.strip_suffix('Z') { Some(t) => (t, true), None => (time, false) };
    if time.len() != 6 || !time.bytes().all(|b| b.is_ascii_digit()) { return None; }
    let t = Time::new(time[0..2].parse().ok()?, time[2..4].parse().ok()?, time[4..6].parse::<i8>().ok()?.min(59), 0).ok()?;
    Some((parse_date(date)?.to_datetime(t), utc))
}

/// Resolve one value with its TZID. `None` means it cannot be placed in time.
/// The second item names the zone used, for storage.
fn when(value: &str, tzid: Option<&str>, all_day_param: bool, local: &TimeZone) -> Option<(When, String)> {
    let value = value.trim();
    if all_day_param || !value.contains('T') {
        return Some((When::Day(parse_date(value)?), zone_name(local)));
    }
    let (dt, utc) = parse_datetime(value)?;
    if utc {
        return Some((When::At(TimeZone::UTC.to_timestamp(dt).ok()?), "UTC".into()));
    }
    let tz = match tzid {
        Some(id) => TimeZone::get(id).ok()?,
        None => local.clone(),
    };
    let zone = zone_name(&tz);
    Some((When::At(dt.to_zoned(tz).ok()?.timestamp()), zone))
}

fn prop_when(p: &Prop, local: &TimeZone) -> Option<(When, String)> {
    when(&p.value, p.param("TZID"), p.param("VALUE") == Some("DATE"), local)
}

/// `P1D`, `PT1H30M`, `P1W`, `-PT15M`: seconds. Months and years are not durations here.
fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, s) = match s.strip_prefix('-') { Some(r) => (-1, r), None => (1, s.strip_prefix('+').unwrap_or(s)) };
    let s = s.strip_prefix('P')?;
    let mut total = 0i64;
    let mut num = String::new();
    let mut in_time = false;
    for c in s.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            _ => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += n * match (c, in_time) {
                    ('W', false) => 7 * 86_400,
                    ('D', false) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    num.is_empty().then_some(sign * total)
}

fn weekday(code: &str) -> Option<Weekday> {
    Some(match code {
        "MO" => Weekday::Monday, "TU" => Weekday::Tuesday, "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday, "FR" => Weekday::Friday, "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday, _ => return None,
    })
}

/// Daily/weekly only; any other part makes the rule (and its event) unsupported.
fn parse_rrule(value: &str, local: &TimeZone) -> Option<Repeat> {
    let mut rule = Repeat { freq: Freq::Daily, interval: 1, count: None, until: None, by_day: vec![] };
    let mut freq = None;
    for part in value.split(';').filter(|p| !p.is_empty()) {
        let (key, val) = part.split_once('=')?;
        match key.to_ascii_uppercase().as_str() {
            "FREQ" => freq = Some(match val { "DAILY" => Freq::Daily, "WEEKLY" => Freq::Weekly, _ => return None }),
            "INTERVAL" => rule.interval = val.parse().ok().filter(|n| *n > 0)?,
            "COUNT" => rule.count = Some(val.parse().ok()?),
            "UNTIL" => rule.until = Some(match when(val, None, false, local)?.0 {
                When::At(ts) => ts,
                // A date UNTIL includes that whole day.
                When::Day(d) => day_start(d.checked_add(1.day()).ok()?, local),
            }),
            "BYDAY" => rule.by_day = val.split(',').map(weekday).collect::<Option<_>>()?,
            "WKST" => {}
            _ => return None,
        }
    }
    rule.freq = freq?;
    if rule.freq == Freq::Daily && !rule.by_day.is_empty() { return None; }
    Some(rule)
}

/// Parse one VEVENT's properties into a series, or `None` if it is outside the subset.
fn event(props: &[Prop], local: &TimeZone) -> Option<(Series, Option<Timestamp>)> {
    let get = |name: &str| props.iter().find(|p| p.name == name);
    if get("STATUS").is_some_and(|p| p.value.eq_ignore_ascii_case("CANCELLED")) { return None; }
    if props.iter().any(|p| p.name == "RDATE") { return None; }
    let (start, zone) = prop_when(get("DTSTART")?, local)?;
    let (start_ts, all_day) = match start {
        When::At(ts) => (ts, false),
        When::Day(d) => (day_start(d, local), true),
    };
    let end_ts = if let Some(p) = get("DTEND") {
        match prop_when(p, local)?.0 {
            When::At(ts) => ts,
            When::Day(d) => day_start(d, local),
        }
    } else if let Some(p) = get("DURATION") {
        let secs = parse_duration(&p.value)?;
        match start {
            // Whole-day durations stay whole days across a clock change.
            When::Day(d) if secs % 86_400 == 0 => day_start(d.checked_add((secs / 86_400).days()).ok()?, local),
            _ => Timestamp::from_second(start_ts.as_second() + secs).ok()?,
        }
    } else if all_day {
        match start { When::Day(d) => day_start(d.checked_add(1.day()).ok()?, local), _ => start_ts }
    } else {
        start_ts
    };
    if end_ts < start_ts { return None; }
    let repeat = match get("RRULE") {
        Some(p) => Some(parse_rrule(&p.value, local)?),
        None => None,
    };
    let mut exdates = vec![];
    for p in props.iter().filter(|p| p.name == "EXDATE") {
        for v in p.value.split(',') {
            exdates.push(match when(v, p.param("TZID"), p.param("VALUE") == Some("DATE"), local)?.0 {
                When::At(ts) => ts,
                When::Day(d) => day_start(d, local),
            });
        }
    }
    let recurrence_id = match get("RECURRENCE-ID") {
        Some(p) => Some(match prop_when(p, local)?.0 { When::At(ts) => ts, When::Day(d) => day_start(d, local) }),
        None => None,
    };
    let text = |name: &str| get(name).map(|p| unescape(&p.value)).filter(|s| !s.trim().is_empty());
    let title = text("SUMMARY").unwrap_or_else(|| "(No title)".into());
    let id = text("UID").unwrap_or_else(|| format!("{}-{}", title, start_ts.as_second()));
    let event = Event {
        id, title, start: start_ts, end: end_ts, all_day, zone,
        location: text("LOCATION"), notes: text("DESCRIPTION"),
    };
    Some((Series { event, repeat, exdates }, recurrence_id))
}

pub fn parse(text: &str, local: &TimeZone) -> Parsed {
    let mut parsed = Parsed::default();
    let mut overrides: Vec<(String, Timestamp, Series)> = vec![];
    let mut current: Option<Vec<Prop>> = None;
    let mut nested = 0usize;
    for line in unfold(text) {
        let Some(prop) = split_prop(&line) else { continue };
        match (prop.name.as_str(), prop.value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VCALENDAR") => parsed.is_calendar = true,
            ("BEGIN", "VEVENT") => { current = Some(vec![]); nested = 0; }
            ("END", "VEVENT") => {
                if let Some(props) = current.take() {
                    match event(&props, local) {
                        Some((series, Some(rid))) => overrides.push((series.event.id.clone(), rid, series)),
                        Some((series, None)) => parsed.series.push(series),
                        None => parsed.skipped += 1,
                    }
                }
            }
            // VALARM and friends live inside a VEVENT; their properties are not the event's.
            ("BEGIN", _) if current.is_some() => nested += 1,
            ("END", _) if current.is_some() => nested = nested.saturating_sub(1),
            _ => {
                if let (Some(props), 0) = (current.as_mut(), nested) { props.push(prop); }
            }
        }
    }
    // A RECURRENCE-ID event replaces one instance of its series.
    for (uid, rid, series) in overrides {
        if let Some(master) = parsed.series.iter_mut().find(|s| s.event.id == uid && s.repeat.is_some()) {
            master.exdates.push(rid);
        }
        parsed.series.push(series);
    }
    parsed
}
