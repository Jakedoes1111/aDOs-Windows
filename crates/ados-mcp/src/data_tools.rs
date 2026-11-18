//! Tools over notes, captures, tasks and the calendar. Each one uses the module's own
//! public file API, so the panels show the change the next time they look.
use std::path::{Path, PathBuf};
use std::rc::Rc;

use ados_notes::file;
use gpui_kit::*;
use serde_json::{json, Value};

use crate::host::{notice, toast, Undo};
use crate::protocol::ToolResult;

const CHANGED_TASK: &str = "This task changed since; undo would remove your edits.";

fn undo(f: impl Fn(&mut Window, &mut App) + 'static) -> Option<Undo> {
    Some(Rc::new(f))
}

/// A path the agent gave, checked to be a note inside the notes folder.
fn note_path(path: &str) -> Result<PathBuf, String> {
    let dir = ados_core::paths::notes_dir();
    let dir = dir.canonicalize().unwrap_or(dir);
    let given = Path::new(path);
    let given = if given.is_absolute() { given.to_path_buf() } else { dir.join(given) };
    let real = given.canonicalize().map_err(|_| format!("There is no note at {path}."))?;
    if real.parent() != Some(dir.as_path()) || real.extension().and_then(|e| e.to_str()) != Some("md") {
        return Err(format!("{path} is not a note in {}.", dir.display()));
    }
    Ok(real)
}

pub fn notes_list() -> ToolResult {
    let notes = file::list_notes().map_err(|e| format!("Could not list notes: {e:#}"))?;
    let list: Vec<Value> = notes
        .iter()
        .take(500)
        .map(|n| json!({ "title": n.title, "path": n.path.to_string_lossy() }))
        .collect();
    Ok(json!(list))
}

pub fn notes_read(path: &str) -> ToolResult {
    let path = note_path(path)?;
    std::fs::read_to_string(&path).map(Value::String).map_err(|e| format!("Could not read {}: {e}", path.display()))
}

pub fn notes_create(title: &str, body: &str, window: &mut Window, cx: &mut App) -> ToolResult {
    let title = title.trim();
    if title.is_empty() {
        return Err("A note needs a title.".into());
    }
    let text = format!("# {title}\n\n{}\n", body.trim_end());
    let path = file::create_note(&text).map_err(|e| format!("Could not create the note: {e:#}"))?;
    let (p, t) = (path.clone(), title.to_string());
    toast(
        format!("created note “{title}”"),
        undo(move |window, cx| {
            // Only a note still exactly as the assistant made it goes to trash.
            let result = ados_notes::flush_open(cx).and_then(|()| {
                if std::fs::read_to_string(&p).ok().as_deref() != Some(text.as_str()) {
                    return Err(file::CHANGED_SINCE.to_string());
                }
                file::trash_note(&p).map(|_| ()).map_err(|e| format!("Could not undo: {e:#}"))
            });
            match result {
                Ok(()) => notice(format!("Moved “{t}” to trash."), window, cx),
                Err(e) => notice(e, window, cx),
            }
        }),
        window,
        cx,
    );
    Ok(json!({ "path": path.to_string_lossy(), "title": title }))
}

pub fn notes_append(path: &str, text: &str, window: &mut Window, cx: &mut App) -> ToolResult {
    let path = note_path(path)?;
    // The Notes editor does not reload a file changed underneath it, so its next save would
    // drop the appended text. Refuse rather than lose work.
    let open = ados_notes::state::load().ok().and_then(|s| s.selected).and_then(|p| p.canonicalize().ok());
    if open.as_deref() == Some(path.as_path()) {
        return Err("That note is open in Notes right now, so appending could be overwritten by the editor. Create a new note, or ask the owner to open another note first.".into());
    }
    let old = std::fs::read_to_string(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let new = file::appended(&old, text);
    let saved = file::write_note(&path, &new).map_err(|e| format!("Could not save the note: {e:#}"))?;
    let title = file::title_from_text(&new);
    let (p, before, added) = (saved.clone(), old, text.to_string());
    toast(
        format!("added to note “{title}”"),
        undo(move |window, cx| {
            // Compare with the note as it is now (the open editor saved first): remove only
            // the appended block, never what the owner wrote since.
            let result = ados_notes::flush_open(cx).and_then(|()| {
                let current = std::fs::read_to_string(&p).map_err(|_| file::CHANGED_SINCE.to_string())?;
                let undone = file::without_appended(&current, &before, &added).ok_or_else(|| file::CHANGED_SINCE.to_string())?;
                file::atomic_write(&p, &undone).map_err(|e| format!("Could not undo: {e:#}"))
            });
            match result {
                Ok(()) => notice("Removed the added text.".into(), window, cx),
                Err(e) => notice(e, window, cx),
            }
        }),
        window,
        cx,
    );
    Ok(json!({ "path": saved.to_string_lossy(), "title": title }))
}

pub fn capture_save(text: &str, window: &mut Window, cx: &mut App) -> ToolResult {
    let capture = ados_core::capture::save(text).map_err(|e| format!("Could not capture: {e:#}"))?;
    let kind = capture.kind.label();
    let kept = capture.clone();
    toast(
        format!("captured a {kind}"),
        undo(move |window, cx| match ados_core::capture::delete(&kept) {
            Ok(()) => notice("Removed the capture.".into(), window, cx),
            Err(e) => notice(format!("Could not undo: {e:#}"), window, cx),
        }),
        window,
        cx,
    );
    Ok(json!({ "kind": kind, "path": capture.path.to_string_lossy() }))
}

pub fn tasks_list() -> ToolResult {
    let tasks = ados_tasks::list().map_err(|e| format!("Could not read tasks: {e:#}"))?;
    serde_json::to_value(tasks).map_err(|e| e.to_string())
}

pub fn tasks_add(text: &str, window: &mut Window, cx: &mut App) -> ToolResult {
    let task = ados_tasks::add(text).map_err(|e| format!("Could not add the task: {e:#}"))?;
    let (added, title) = (task.clone(), task.title.clone());
    toast(
        format!("added task “{title}”"),
        undo(move |window, cx| {
            // Only a task still exactly as the assistant added it is removed.
            let removed = ados_tasks::store::update(&ados_tasks::store::default_path(), |tasks| {
                match tasks.iter().find(|t| t.id == added.id) {
                    Some(t) if *t == added => {
                        tasks.retain(|t| t.id != added.id);
                        Ok(true)
                    }
                    Some(_) => Ok(false),
                    None => Ok(true),
                }
            });
            match removed {
                Ok(true) => notice(format!("Removed task “{title}”."), window, cx),
                Ok(false) => notice(CHANGED_TASK.into(), window, cx),
                Err(e) => notice(format!("Could not undo: {e:#}"), window, cx),
            }
        }),
        window,
        cx,
    );
    serde_json::to_value(task).map_err(|e| e.to_string())
}

pub fn tasks_set_done(id: &str, done: bool, window: &mut Window, cx: &mut App) -> ToolResult {
    let before = ados_tasks::list()
        .map_err(|e| format!("Could not read tasks: {e:#}"))?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("There is no task with id {id}. Use tasks_list."))?;
    let task = ados_tasks::set_done(id, done).map_err(|e| format!("Could not change the task: {e:#}"))?;
    let (id, was_done, title, set) = (task.id.clone(), before.is_done(), task.title.clone(), task.clone());
    let verb = if done { "marked done" } else { "reopened" };
    toast(
        format!("{verb} “{title}”"),
        undo(move |window, cx| {
            // The owner changed the task since: leave it as they left it.
            let now = ados_tasks::list().ok().and_then(|l| l.into_iter().find(|t| t.id == id));
            if now.as_ref() != Some(&set) {
                return notice(CHANGED_TASK.into(), window, cx);
            }
            match ados_tasks::set_done(&id, was_done) {
                Ok(_) => notice(format!("“{title}” is back as it was."), window, cx),
                Err(e) => notice(format!("Could not undo: {e:#}"), window, cx),
            }
        }),
        window,
        cx,
    );
    serde_json::to_value(task).map_err(|e| e.to_string())
}

/// RFC 3339 instant, or a plain date meaning local midnight.
pub fn parse_time(text: &str) -> Result<jiff::Timestamp, String> {
    let text = text.trim();
    if let Ok(ts) = text.parse::<jiff::Timestamp>() {
        return Ok(ts);
    }
    text.parse::<jiff::civil::Date>()
        .ok()
        .and_then(|d| d.to_zoned(jiff::tz::TimeZone::system()).ok())
        .map(|z| z.timestamp())
        .ok_or_else(|| format!("\"{text}\" is not a time. Use 2026-11-14T09:00:00Z or 2026-11-14."))
}

pub fn events_between(start: &str, end: &str) -> ToolResult {
    let (start, end) = (parse_time(start)?, parse_time(end)?);
    if end <= start {
        return Err("end must be after start.".into());
    }
    let tz = jiff::tz::TimeZone::system();
    let local = |ts: jiff::Timestamp| ts.to_zoned(tz.clone()).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string();
    let list: Vec<Value> = ados_calendar::events_between(start, end)
        .iter()
        .map(|o| {
            json!({
                "id": o.id, "title": o.title, "start": local(o.start), "end": local(o.end),
                "all_day": o.all_day, "location": o.location, "notes": o.notes,
                "calendar": o.calendar.clone().unwrap_or_else(|| "aDOs".into()),
            })
        })
        .collect();
    Ok(json!(list))
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: that glob brings in `gpui_kit::*`, whose `test` attribute macro
    // shadows the built-in `#[test]` and recurses.
    use super::parse_time;

    #[test]
    fn parses_instants_and_dates() {
        assert_eq!(parse_time("2026-11-14T09:00:00Z").map(|t| t.as_second()), Ok(1_794_646_800));
        assert!(parse_time("2026-11-14").is_ok());
        assert!(parse_time("friday").is_err());
    }
}
