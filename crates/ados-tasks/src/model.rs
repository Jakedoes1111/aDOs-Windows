//! A task and the two views over the list. Pure: no files, no UI.
use chrono::{DateTime, Datelike, Local, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Open,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub status: Status,
    #[serde(default)]
    pub due: Option<NaiveDate>,
    #[serde(default)]
    pub project: Option<String>,
    /// The day this task was put on My Day. Only counts on that day.
    #[serde(default)]
    pub my_day: Option<NaiveDate>,
    pub created: DateTime<Local>,
    pub updated: DateTime<Local>,
    /// Position in the list; lower comes first. Unique, but not contiguous.
    pub order: i64,
}

impl Task {
    pub fn is_done(&self) -> bool {
        self.status == Status::Done
    }
    /// Due today or earlier, or marked for today.
    pub fn in_my_day(&self, today: NaiveDate) -> bool {
        self.due.is_some_and(|d| d <= today) || self.my_day == Some(today)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum View {
    #[default]
    MyDay,
    All,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// A project heading in the All view. Not selectable.
    Group(String),
    Task(Task),
    /// The collapsible "Done" heading. Selectable; Space or Enter opens it.
    DoneToggle { count: usize, open: bool },
}

pub const NO_PROJECT: &str = "No project";

/// What the panel shows, top to bottom.
pub fn lines(tasks: &[Task], view: View, today: NaiveDate, done_open: bool) -> Vec<Line> {
    let mut sorted: Vec<&Task> = tasks.iter().collect();
    sorted.sort_by_key(|t| t.order);
    let shown = |t: &&Task| match view {
        View::All => true,
        // Done tasks stay in My Day only for the day they were finished.
        View::MyDay if t.is_done() => t.updated.date_naive() == today && t.in_my_day(today),
        View::MyDay => t.in_my_day(today),
    };
    let (done, open): (Vec<&Task>, Vec<&Task>) = sorted.into_iter().filter(shown).partition(|t| t.is_done());
    let mut out = Vec::new();
    match view {
        View::MyDay => out.extend(open.into_iter().cloned().map(Line::Task)),
        View::All => {
            let mut projects: Vec<Option<&str>> = open.iter().map(|t| t.project.as_deref()).collect();
            projects.sort_by_key(|p| p.map(str::to_lowercase));
            projects.dedup();
            for project in projects {
                out.push(Line::Group(project.unwrap_or(NO_PROJECT).to_string()));
                out.extend(open.iter().filter(|t| t.project.as_deref() == project).map(|t| Line::Task((*t).clone())));
            }
        }
    }
    if !done.is_empty() {
        out.push(Line::DoneToggle { count: done.len(), open: done_open });
        if done_open {
            out.extend(done.into_iter().cloned().map(Line::Task));
        }
    }
    out
}

/// The neighbour to swap with for Alt+Up/Down: the next task in the same group, if any.
pub fn swap_partner(lines: &[Line], id: &str, up: bool) -> Option<String> {
    let at = lines.iter().position(|l| matches!(l, Line::Task(t) if t.id == id))?;
    let next = if up { at.checked_sub(1)? } else { at + 1 };
    match lines.get(next)? {
        Line::Task(t) => Some(t.id.clone()),
        _ => None,
    }
}

/// Plain-text form of the visible list, for the assistant's context.
pub fn as_text(lines: &[Line], today: NaiveDate) -> String {
    let mut out = String::new();
    for line in lines {
        match line {
            Line::Group(name) => out.push_str(&format!("\n{name}\n")),
            Line::DoneToggle { count, open } => {
                out.push_str(&format!("\nDone ({count}){}\n", if *open { "" } else { ", collapsed" }))
            }
            Line::Task(t) => {
                out.push_str(if t.is_done() { "[x] " } else { "[ ] " });
                out.push_str(&t.title);
                if let Some(due) = t.due {
                    out.push_str(&format!(" · {}", due_label(due, today)));
                }
                if let Some(p) = &t.project {
                    out.push_str(&format!(" #{p}"));
                }
                out.push('\n');
            }
        }
    }
    out.trim_start().to_string()
}

/// "Today", "Tomorrow", "Overdue · Thu 12 Nov", "Fri 20 Nov".
pub fn due_label(due: NaiveDate, today: NaiveDate) -> String {
    let days = (due - today).num_days();
    let date = if due.year() == today.year() {
        due.format("%a %-d %b").to_string()
    } else {
        due.format("%a %-d %b %Y").to_string()
    };
    match days {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        d if d < 0 => format!("Overdue · {date}"),
        _ => date,
    }
}
