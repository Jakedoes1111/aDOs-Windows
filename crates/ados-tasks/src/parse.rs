//! Quick-add syntax: `Buy milk tomorrow #home`.
//! `#word` anywhere sets the project. The last remaining word sets the due date when it is
//! `today`, `tomorrow`, a weekday (`fri`, `friday`: the next one, never today) or `YYYY-MM-DD`.
use chrono::{Datelike, Duration, NaiveDate, Weekday};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    pub title: String,
    pub due: Option<NaiveDate>,
    pub project: Option<String>,
}

pub fn quick_add(text: &str, today: NaiveDate) -> Parsed {
    let mut project = None;
    let mut words: Vec<&str> = Vec::new();
    for word in text.split_whitespace() {
        match word.strip_prefix('#') {
            Some(name) if !name.is_empty() => project = Some(name.to_string()),
            _ => words.push(word),
        }
    }
    let mut due = None;
    // A date alone is a title, not an empty task with a date.
    if words.len() > 1 {
        if let Some(date) = words.last().and_then(|w| date(w, today)) {
            due = Some(date);
            words.pop();
        }
    }
    Parsed { title: words.join(" "), due, project }
}

/// One date word, or None if it is not one.
pub fn date(word: &str, today: NaiveDate) -> Option<NaiveDate> {
    let word = word.trim().to_lowercase();
    match word.as_str() {
        "today" | "tod" => return Some(today),
        "tomorrow" | "tmr" => return Some(today + Duration::days(1)),
        _ => {}
    }
    if let Some(day) = weekday(&word) {
        let ahead = (day.num_days_from_monday() as i64 - today.weekday().num_days_from_monday() as i64 + 7) % 7;
        return Some(today + Duration::days(if ahead == 0 { 7 } else { ahead }));
    }
    if word.len() == 10 {
        return NaiveDate::parse_from_str(&word, "%Y-%m-%d").ok();
    }
    None
}

fn weekday(word: &str) -> Option<Weekday> {
    const DAYS: [(&str, &str, Weekday); 7] = [
        ("mon", "monday", Weekday::Mon),
        ("tue", "tuesday", Weekday::Tue),
        ("wed", "wednesday", Weekday::Wed),
        ("thu", "thursday", Weekday::Thu),
        ("fri", "friday", Weekday::Fri),
        ("sat", "saturday", Weekday::Sat),
        ("sun", "sunday", Weekday::Sun),
    ];
    DAYS.iter().find(|(short, long, _)| word == *short || word == *long).map(|d| d.2)
}

/// What the quick-add field will do on Enter, in words. None when there is nothing to add.
pub fn describe(parsed: &Parsed, today: NaiveDate) -> Option<String> {
    if parsed.title.is_empty() {
        return None;
    }
    let mut out = format!("Adds “{}”", parsed.title);
    if let Some(due) = parsed.due {
        out.push_str(&format!(" · due {}", crate::model::due_label(due, today)));
    }
    if let Some(p) = &parsed.project {
        out.push_str(&format!(" · #{p}"));
    }
    Some(out)
}
