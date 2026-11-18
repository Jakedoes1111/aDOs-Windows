use crate::{fields, ics, model::*, store};
use jiff::{civil::date, tz::TimeZone, Timestamp};

fn london() -> TimeZone { TimeZone::get("Europe/London").expect("tzdata") }
fn ts(s: &str) -> Timestamp { s.parse().expect("timestamp") }
const FIXTURE: &str = include_str!("../fixtures/sample.ics");

#[test]
fn parses_fixture_subset() {
    let parsed = ics::parse(FIXTURE, &london());
    assert!(parsed.is_calendar);
    assert_eq!(parsed.skipped, 1, "the monthly rule is skipped and counted");
    let titles: Vec<_> = parsed.series.iter().map(|s| s.event.title.as_str()).collect();
    assert!(titles.contains(&"Design review, round 2"), "{titles:?}");
    let review = parsed.series.iter().find(|s| s.event.id == "review@fixture").unwrap();
    assert_eq!(review.event.start, ts("2026-11-13T14:00:00Z"));
    assert_eq!(review.event.end, ts("2026-11-13T15:30:00Z"));
    assert_eq!(review.event.notes.as_deref(), Some("Bring the midnight theme\nscreenshots."));
    assert_eq!(review.event.location.as_deref(), Some("Studio"), "VALARM's DESCRIPTION must not leak");
    let walk = parsed.series.iter().find(|s| s.event.id == "walk@fixture").unwrap();
    assert_eq!(walk.event.end, ts("2026-11-01T07:30:00Z"));
    assert_eq!(walk.repeat.as_ref().unwrap().interval, 2);
    let holiday = parsed.series.iter().find(|s| s.event.id == "holiday@fixture").unwrap();
    assert!(holiday.event.all_day);
    assert_eq!(holiday.event.start, ts("2026-11-16T00:00:00Z"));
}

#[test]
fn unfolds_and_rejects_unknown_rules() {
    let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Long\r\n  title\r\nDTSTART:20260101T100000Z\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:b\r\nDTSTART:20260101T100000Z\r\nRRULE:FREQ=WEEKLY;BYDAY=1MO\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:c\r\nDTSTART;TZID=Nowhere/Land:20260101T100000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let parsed = ics::parse(text, &TimeZone::UTC);
    assert_eq!(parsed.series.len(), 1);
    assert_eq!(parsed.series[0].event.title, "Long title");
    assert_eq!(parsed.skipped, 2);
    assert!(!ics::parse("hello", &TimeZone::UTC).is_calendar);
}

#[test]
fn weekly_byday_keeps_wall_clock_across_dst() {
    // London leaves summer time on 25 Oct 2026; 09:30 local stays 09:30.
    let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nSUMMARY:S\nDTSTART;TZID=Europe/London:20261019T093000\n\
DTEND;TZID=Europe/London:20261019T094500\nRRULE:FREQ=WEEKLY;BYDAY=MO,TH;UNTIL=20261102T235959Z\nEND:VEVENT\nEND:VCALENDAR\n";
    let parsed = ics::parse(text, &london());
    let occ = expand(&parsed.series[0], ts("2026-10-01T00:00:00Z"), ts("2026-12-01T00:00:00Z"), None, &london());
    let starts: Vec<String> = occ.iter().map(|o| o.start.to_string()).collect();
    assert_eq!(starts, vec![
        "2026-10-19T08:30:00Z", "2026-10-22T08:30:00Z", "2026-10-26T09:30:00Z",
        "2026-10-29T09:30:00Z", "2026-11-02T09:30:00Z",
    ]);
    assert!(occ.iter().all(|o| o.end.as_second() - o.start.as_second() == 15 * 60));
}

#[test]
fn daily_interval_count_and_exdate() {
    let parsed = ics::parse(FIXTURE, &london());
    let walk = parsed.series.iter().find(|s| s.event.id == "walk@fixture").unwrap();
    let all = expand(walk, ts("2026-01-01T00:00:00Z"), ts("2027-06-01T00:00:00Z"), Some("F"), &london());
    // 30 by COUNT, one of them removed by EXDATE.
    assert_eq!(all.len(), 29);
    assert_eq!(all[0].start, ts("2026-11-01T07:00:00Z"));
    assert!(all.iter().all(|o| o.start != ts("2026-11-11T07:00:00Z")));
    assert_eq!(all[1].start, ts("2026-11-03T07:00:00Z"));
    // Window clipping.
    let week = expand(walk, ts("2026-11-09T00:00:00Z"), ts("2026-11-16T00:00:00Z"), None, &london());
    let days: Vec<i8> = week.iter().map(|o| o.start.to_zoned(TimeZone::UTC).day()).collect();
    assert_eq!(days, vec![9, 13, 15]);
}

#[test]
fn recurrence_id_replaces_one_instance() {
    let parsed = ics::parse(FIXTURE, &london());
    let day = crate::collect(&[], &[crate::subs::Loaded { name: "F".into(), series: parsed.series, skipped: 0, error: None }],
        ts("2026-11-13T00:00:00Z"), ts("2026-11-14T00:00:00Z"));
    let titles: Vec<_> = day.iter().map(|o| o.title.as_str()).collect();
    assert!(titles.contains(&"Team standup (moved)"), "{titles:?}");
    assert!(!titles.contains(&"Team standup"), "{titles:?}");
}

#[test]
fn store_round_trip() {
    let ev = Event {
        id: "ev-1".into(), title: "Dentist".into(), start: ts("2026-11-13T10:00:00Z"), end: ts("2026-11-13T10:30:00Z"),
        all_day: false, zone: "Europe/London".into(), location: Some("High St".into()), notes: None,
    };
    let text = store::render_events(std::slice::from_ref(&ev)).unwrap();
    assert!(text.contains("2026-11-13T10:00:00Z"), "UTC instants on disk: {text}");
    assert_eq!(store::parse_events(&text).unwrap(), vec![ev]);
    assert!(store::parse_events("{not json").is_err());
    assert_eq!(store::parse_events("{}").unwrap(), vec![]);
}

#[test]
fn form_fields() {
    let tz = london();
    let (s, e, all) = fields::parse_times("2026-11-13 15:00", "1h30m", &tz).unwrap();
    assert_eq!((s, e, all), (ts("2026-11-13T15:00:00Z"), ts("2026-11-13T16:30:00Z"), false));
    let (s, e, all) = fields::parse_times("2026-07-01", "2d", &tz).unwrap();
    assert_eq!((s, e, all), (ts("2026-06-30T23:00:00Z"), ts("2026-07-02T23:00:00Z"), true));
    assert_eq!(fields::parse_duration("90", false), Ok(90));
    assert_eq!(fields::parse_duration("1:15", false), Ok(75));
    assert!(fields::parse_duration("soon", false).is_err());
    assert!(fields::parse_times("13/11/2026", "1h", &tz).is_err());
    assert_eq!(fields::format_start(s, true, &tz), "2026-07-01");
    assert_eq!(fields::format_duration(s, e, true, &tz), "2d");
    assert_eq!(fields::format_duration(ts("2026-01-01T10:00:00Z"), ts("2026-01-01T11:45:00Z"), false, &tz), "1h45m");
    let _ = date(2026, 1, 1);
}
