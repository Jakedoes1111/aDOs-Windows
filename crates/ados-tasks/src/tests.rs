use crate::model::{self, Line, View};
use crate::{parse, store};
use chrono::NaiveDate;
use std::path::PathBuf;

// Saturday 14 November 2026.
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()
}
fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}
fn temp_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ados-tasks-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("tasks").join("tasks.json")
}

#[test]
fn parses_dates_and_project() {
    let p = parse::quick_add("Buy milk tomorrow #home", today());
    assert_eq!((p.title.as_str(), p.due, p.project.as_deref()), ("Buy milk", Some(day(15)), Some("home")));
    let p = parse::quick_add("Call mum #family today", today());
    assert_eq!((p.title.as_str(), p.due, p.project.as_deref()), ("Call mum", Some(day(14)), Some("family")));
    let p = parse::quick_add("Pay rent 2026-12-01", today());
    assert_eq!(p.due, NaiveDate::from_ymd_opt(2026, 12, 1));
    assert_eq!(p.title, "Pay rent");
}

#[test]
fn weekdays_are_the_next_one() {
    assert_eq!(parse::date("mon", today()), Some(day(16)));
    assert_eq!(parse::date("Friday", today()), Some(day(20)));
    // Today is Saturday: "sat" means next week, "today" means today.
    assert_eq!(parse::date("saturday", today()), Some(day(21)));
}

#[test]
fn date_words_only_count_at_the_end() {
    let p = parse::quick_add("Plan friday party", today());
    assert_eq!((p.title.as_str(), p.due), ("Plan friday party", None));
    let p = parse::quick_add("tomorrow", today());
    assert_eq!((p.title.as_str(), p.due), ("tomorrow", None));
    let p = parse::quick_add("Fix 2026-13-40", today());
    assert_eq!((p.title.as_str(), p.due), ("Fix 2026-13-40", None));
    let p = parse::quick_add("Issue # 4", today());
    assert_eq!((p.title.as_str(), p.project), ("Issue # 4", None));
    assert_eq!(parse::describe(&parse::quick_add("Gym fri #health", today()), today()).as_deref(),
        Some("Adds “Gym” · due Fri 20 Nov · #health"));
    assert_eq!(parse::describe(&parse::quick_add("  ", today()), today()), None);
}

#[test]
fn store_round_trip() {
    let path = temp_file("round");
    let a = crate::add_at(&path, "Write report fri #work", today()).unwrap();
    let b = crate::add_at(&path, "Water plants", today()).unwrap();
    crate::set_done_at(&path, &b.id, true).unwrap();
    let tasks = store::load(&path).unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0], a);
    assert!(tasks[1].is_done());
    assert!(tasks[0].order < tasks[1].order);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"due\": \"2026-11-20\""), "human-readable dates: {text}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
}

#[test]
fn bad_file_is_kept() {
    let path = temp_file("bad");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ not json").unwrap();
    let err = store::load(&path).unwrap_err();
    assert!(format!("{err:#}").contains("kept as it is"));
    assert!(crate::add_at(&path, "Anything", today()).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
}

fn task(title: &str, order: i64, due: Option<NaiveDate>, project: Option<&str>) -> model::Task {
    let mut t = store::new_task(&[], title);
    t.id = title.into();
    t.order = order;
    t.due = due;
    t.project = project.map(Into::into);
    t
}

#[test]
fn my_day_and_all_views() {
    let mut marked = task("marked", 4, None, None);
    marked.my_day = Some(today());
    let mut stale = task("stale mark", 5, None, None);
    stale.my_day = Some(day(13));
    let mut done = task("done overdue", 6, Some(day(1)), None);
    done.status = model::Status::Done;
    done.updated = done.updated - chrono::Duration::days(400);
    let tasks = vec![
        task("later", 1, Some(day(20)), Some("work")),
        task("overdue", 2, Some(day(10)), Some("home")),
        task("today", 3, Some(day(14)), None),
        marked, stale, done,
    ];
    let titles = |lines: &[Line]| -> Vec<String> {
        lines.iter().map(|l| match l {
            Line::Group(g) => format!("={g}"),
            Line::Task(t) => t.title.clone(),
            Line::DoneToggle { count, .. } => format!("done {count}"),
        }).collect()
    };
    let my_day = model::lines(&tasks, View::MyDay, today(), false);
    // The done task was finished long before the test day, so My Day hides it.
    assert_eq!(titles(&my_day), ["overdue", "today", "marked"]);
    let all = model::lines(&tasks, View::All, today(), false);
    assert_eq!(titles(&all), ["=No project", "today", "marked", "stale mark", "=home", "overdue", "=work", "later", "done 1"]);
    let open = model::lines(&tasks, View::All, today(), true);
    assert_eq!(titles(&open).last().map(String::as_str), Some("done overdue"));
    assert_eq!(model::swap_partner(&all, "marked", true).as_deref(), Some("today"));
    assert_eq!(model::swap_partner(&all, "today", true), None);
    assert_eq!(model::swap_partner(&all, "stale mark", false), None);
    assert!(model::as_text(&my_day, today()).contains("[ ] overdue · Overdue · Tue 10 Nov #home"));
}
