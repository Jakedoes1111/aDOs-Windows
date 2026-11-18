//! The page feeds the other modules: page to note, append to the current note, page to task,
//! copy as a markdown link. Palette actions and `BrowserPanel` shortcuts (they work while
//! the page has focus, because the page hands registry chords back to aDOs).
use ados_core::{hub::ContextHub, ids};
use gpui_kit::component::WindowExt;
use gpui_kit::*;

actions!(ados_page, [PageToNote, PageAppendNote, PageToTask, PageCopyLink]);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind { Note, Append, Task, CopyLink }

pub(crate) fn run_page(kind: Kind, window: &mut Window, cx: &mut App) { run(kind, window, cx) }

pub fn init(cx: &mut App) {
    use ados_core::shortcuts::bind;
    bind(cx, "page.to_note", PageToNote);
    bind(cx, "page.append_note", PageAppendNote);
    bind(cx, "page.to_task", PageToTask);
    bind(cx, "page.copy_link", PageCopyLink);
    for (id, title, kind) in [
        ("page.to_note", "Page to note", Kind::Note),
        ("page.append_note", "Append page to current note", Kind::Append),
        ("page.to_task", "Page to task", Kind::Task),
        ("page.copy_link", "Copy page link as markdown", Kind::CopyLink),
    ] {
        ados_core::actions::register(id, title, "browser page web link save note task", None, cx, move |window, cx| run(kind, window, cx));
    }
}

fn run(kind: Kind, window: &mut Window, cx: &mut App) {
    let page = ContextHub::get(ids::BROWSER, cx).filter(|s| s.location.as_ref().is_some_and(|l| !l.is_empty()));
    let Some(page) = page else {
        window.push_notification("The Browser has no page open yet", cx);
        return;
    };
    let url = page.location.map(|l| l.to_string()).unwrap_or_default();
    let title = page.title.to_string();
    #[cfg(feature = "browser")]
    if let Some(browser) = crate::modules::browser_panel(cx) {
        if kind == Kind::CopyLink {
            return finish(kind, &title, &url, "", window, cx);
        }
        browser.update(cx, |b, cx| {
            b.with_selection(window, cx, move |selection, window, cx| finish(kind, &title, &url, &selection, window, cx))
        });
        return;
    }
    let _ = (kind, title, url);
    window.push_notification("The Browser is not available in this build", cx);
}

use ados_core::links::{append_text, md_link, note_text};

fn finish(kind: Kind, title: &str, url: &str, selection: &str, window: &mut Window, cx: &mut App) {
    let shown = if title.trim().is_empty() { url } else { title.trim() };
    match kind {
        Kind::CopyLink => {
            cx.write_to_clipboard(ClipboardItem::new_string(md_link(title, url)));
            window.push_notification("Copied the page link", cx);
        }
        Kind::Note => match new_note(&note_text(title, url, selection)) {
            Ok(()) => {
                refresh_notes(window, cx);
                window.push_notification(format!("Saved note “{shown}”"), cx);
            }
            Err(e) => window.push_notification(e, cx),
        },
        Kind::Append => match append_note(&append_text(title, url, selection), cx) {
            Ok(()) => {
                refresh_notes(window, cx);
                window.push_notification("Added the page to the current note", cx);
            }
            Err(e) => window.push_notification(e, cx),
        },
        Kind::Task => match new_task(shown, url) {
            Ok(()) => {
                #[cfg(feature = "tasks")]
                ados_tasks::refresh(cx);
                window.push_notification(format!("Added task “{shown}”"), cx);
            }
            Err(e) => window.push_notification(e, cx),
        },
    }
}

fn refresh_notes(window: &mut Window, cx: &mut App) {
    #[cfg(feature = "notes")]
    ados_notes::panel::refresh_list(window, cx);
    let _ = (window, cx);
}

#[cfg(feature = "notes")]
fn new_note(text: &str) -> Result<(), String> {
    ados_notes::file::create_note(text).map(|_| ()).map_err(|e| format!("Could not create the note: {e:#}"))
}
#[cfg(not(feature = "notes"))]
fn new_note(_: &str) -> Result<(), String> { Err("Notes is not available in this build".into()) }

#[cfg(feature = "notes")]
fn append_note(text: &str, cx: &mut App) -> Result<(), String> {
    use ados_notes::file;
    let path = ados_notes::state::load().ok().and_then(|s| s.selected)
        .ok_or("Open a note in Notes first, or use Page to note")?;
    ados_notes::flush_open(cx)?;
    let old = std::fs::read_to_string(&path).map_err(|e| format!("Could not read the note: {e}"))?;
    file::write_note(&path, &file::appended(&old, text)).map(|_| ()).map_err(|e| format!("Could not save the note: {e:#}"))
}
#[cfg(not(feature = "notes"))]
fn append_note(_: &str, _: &mut App) -> Result<(), String> { Err("Notes is not available in this build".into()) }

#[cfg(feature = "tasks")]
fn new_task(title: &str, url: &str) -> Result<(), String> {
    ados_tasks::store::update(&ados_tasks::store::default_path(), |tasks| {
        let mut task = ados_tasks::store::new_task(tasks, title);
        task.notes = url.to_string();
        tasks.push(task);
        Ok(())
    }).map_err(|e| format!("Could not add the task: {e:#}"))
}
#[cfg(not(feature = "tasks"))]
fn new_task(_: &str, _: &str) -> Result<(), String> { Err("Tasks is not available in this build".into()) }
