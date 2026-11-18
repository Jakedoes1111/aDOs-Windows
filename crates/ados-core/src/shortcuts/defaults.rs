//! Every aDOs shortcut and its default chords. Crates attach actions by id with `shortcuts::bind`.
//! Ids match palette action ids where one exists, so the palette shows the live chord.
use super::model::{Spec, GLOBAL};

pub struct Def {
    pub id: &'static str,
    pub title: &'static str,
    pub scope: &'static str,
    pub keys: &'static [&'static str],
    pub overrides: &'static [&'static str],
}
const fn d(id: &'static str, title: &'static str, scope: &'static str, keys: &'static [&'static str]) -> Def {
    Def { id, title, scope, keys, overrides: &[] }
}
const fn o(id: &'static str, title: &'static str, scope: &'static str, keys: &'static [&'static str], overrides: &'static [&'static str]) -> Def {
    Def { id, title, scope, keys, overrides }
}

/// Active in the shell, but not one of the scopes the browser page hands back to aDOs.
pub const ZOOM_SCOPE: &str = "AdosShell && !BrowserPage";

pub const DEFAULTS: &[Def] = &[
    // Workspace, active everywhere in the window.
    d("shell.palette", "Open command palette", GLOBAL, &["ctrl-k"]),
    d("shell.capture", "Quick capture", GLOBAL, &["ctrl-shift-k"]),
    d("shell.shortcuts", "Edit shortcuts", GLOBAL, &["ctrl-,"]),
    d("assistant.toggle", "Toggle assistant", GLOBAL, &["alt-space", "ctrl-5"]),
    d("assistant.new_chat", "New assistant chat", GLOBAL, &["ctrl-shift-n"]),
    d("shell.focus.home", "Open Home", GLOBAL, &["ctrl-1"]),
    d("shell.focus.notes", "Open Notes", GLOBAL, &["ctrl-2"]),
    d("shell.focus.terminal", "Open Terminal", GLOBAL, &["ctrl-3"]),
    d("shell.focus.browser", "Open Browser", GLOBAL, &["ctrl-4"]),
    d("shell.focus.sessions", "Open Sessions", GLOBAL, &["ctrl-6"]),
    d("shell.close", "Hide module", GLOBAL, &["ctrl-w"]),
    d("shell.undo", "Undo last close/delete", GLOBAL, &["ctrl-shift-z"]),
    d("shell.zoom", "Zoom module", GLOBAL, &["shift-escape"]),
    d("shell.settings", "Open Settings", GLOBAL, &["ctrl-alt-,"]),
    // Interface zoom. Not grabbed from a Browser page: there the engine's own page zoom keeps the keys.
    d("shell.ui_zoom_in", "Zoom interface in", ZOOM_SCOPE, &["ctrl-="]),
    d("shell.ui_zoom_out", "Zoom interface out", ZOOM_SCOPE, &["ctrl--"]),
    d("shell.ui_zoom_reset", "Reset interface zoom", ZOOM_SCOPE, &["ctrl-0"]),
    d("shell.save", "Save layout", "AdosShell", &["ctrl-s"]),
    d("shell.save-as", "Save as new layout", "AdosShell", &[]),
    d("shell.revert", "Revert layout to saved", "AdosShell", &[]),
    d("browser.focus_address", "Focus browser address bar", "AdosShell", &["ctrl-l"]),
    // Browser.
    d("browser.reload", "Reload page", "BrowserPanel", &["ctrl-r"]),
    d("browser.back", "Back", "BrowserPanel", &["alt-left"]),
    d("browser.forward", "Forward", "BrowserPanel", &["alt-right"]),
    // Page tabs. In the Browser Ctrl+W closes the page tab; the last one hides the Browser.
    d("browser.new_tab", "New page tab", "BrowserPanel", &["ctrl-t"]),
    o("browser.close_tab", "Close page tab", "BrowserPanel", &["ctrl-w"], &["shell.close"]),
    d("browser.next_tab", "Next page tab", "BrowserPanel", &["ctrl-tab"]),
    d("browser.previous_tab", "Previous page tab", "BrowserPanel", &["ctrl-shift-tab"]),
    d("browser.reopen_tab", "Reopen closed page tab", "BrowserPanel", &["ctrl-shift-t"]),
    // Tiled tabs and follower tabs. Not Vivaldi's Ctrl+F7–F9: window managers (xfwm4) take Ctrl+F1–F12.
    d("browser.tile_columns", "Tile tabs side by side", "BrowserPanel", &["alt-shift-v"]),
    d("browser.tile_rows", "Tile tabs stacked", "BrowserPanel", &["alt-shift-h"]),
    d("browser.tile_grid", "Tile tabs in a grid", "BrowserPanel", &["alt-shift-g"]),
    d("browser.tile_next", "Tile with the next tab", "BrowserPanel", &["alt-shift-n"]),
    d("browser.untile_tab", "Untile this tab", "BrowserPanel", &["alt-shift-u"]),
    d("browser.untile_group", "Untile all tabs in the group", "BrowserPanel", &["alt-shift-x"]),
    d("browser.next_tile", "Next tile", "BrowserPanel", &["alt-]"]),
    d("browser.previous_tile", "Previous tile", "BrowserPanel", &["alt-["]),
    d("browser.follow", "Follower tab: follow the other tile, or stop", "BrowserPanel", &["alt-shift-f"]),
    d("browser.tile_grow", "Make this tile bigger", "BrowserPanel", &["ctrl-alt-="]),
    d("browser.tile_shrink", "Make this tile smaller", "BrowserPanel", &["ctrl-alt--"]),
    // Page to note / task (work while the page has focus).
    d("page.to_note", "Page to note", "BrowserPanel", &["ctrl-alt-n"]),
    d("page.append_note", "Append page to current note", "BrowserPanel", &["ctrl-alt-a"]),
    d("page.to_task", "Page to task", "BrowserPanel", &["ctrl-alt-t"]),
    d("page.copy_link", "Copy page link as markdown", "BrowserPanel", &["ctrl-alt-c"]),
    // Comment on part of the page for the assistant (`comment.rs`).
    d("browser.comment", "Comment on the page", "BrowserPanel", &["ctrl-shift-m"]),
    // Link hints. The page script owns these keys (never while a text field has focus).
    d("browser.hints", "Link hints: click", "BrowserPage", &["f"]),
    d("browser.hints_new", "Link hints: open link in place", "BrowserPage", &["shift-f"]),
    // Hint mode over the whole window (aDOs controls and the visible web page).
    d("shell.hints", "Hint mode: click anything on screen", GLOBAL, &["ctrl-;"]),
    // Terminal. Keys the shell program needs are kept from the workspace on purpose.
    d("terminal.clear", "Clear terminal", "Terminal", &["ctrl-shift-l"]),
    d("terminal.open_link", "Open the link above the cursor in the Browser", "Terminal", &["ctrl-shift-o"]),
    d("terminal.copy", "Copy terminal selection", "Terminal", &["ctrl-shift-c"]),
    o("terminal.keep_ctrl_l", "Keep for the shell: clear screen", "Terminal", &["ctrl-l"], &["browser.focus_address"]),
    d("terminal.keep_tab", "Keep for the shell: Tab completion", "Terminal", &["tab"]),
    d("terminal.keep_shift_tab", "Keep for the shell: Shift+Tab", "Terminal", &["shift-tab"]),
    // Notes.
    d("notes.new", "New note", "NotesPanel", &["ctrl-n"]),
    d("notes.preview", "Toggle Markdown preview", "NotesPanel", &["ctrl-shift-p"]),
    d("notes.open_link", "Open link in note", "NotesPanel", &["ctrl-o"]),
    d("notes.search", "Filter notes list", "NotesPanel", &["ctrl-f", "ctrl-shift-f"]),
    d("notes.delete", "Delete note", "NotesList", &["delete"]),
    d("notes.undo_delete", "Undo delete", "NotesList", &["ctrl-z"]),
    d("notes.previous", "Previous note in the list", "NotesList", &["up"]),
    d("notes.next", "Next note in the list", "NotesList", &["down"]),
    // Calendar. Single keys act only on the grid, never inside the event form.
    d("shell.focus.calendar", "Open Calendar", GLOBAL, &["ctrl-8"]),
    d("calendar.new", "New event", "CalendarGrid", &["n"]),
    d("calendar.today", "Go to today", "CalendarGrid", &["t"]),
    d("calendar.previous", "Previous day or week", "CalendarGrid", &["left"]),
    d("calendar.next", "Next day or week", "CalendarGrid", &["right"]),
    d("calendar.select_previous", "Previous event", "CalendarGrid", &["up"]),
    d("calendar.select_next", "Next event", "CalendarGrid", &["down"]),
    d("calendar.edit", "Edit event", "CalendarGrid", &["enter"]),
    d("calendar.delete", "Delete event", "CalendarGrid", &["delete"]),
    d("calendar.undo_delete", "Undo delete event", "CalendarGrid", &["ctrl-z"]),
    d("calendar.day", "Calendar day view", "CalendarGrid", &["d"]),
    d("calendar.week", "Calendar week view", "CalendarGrid", &["w"]),
    d("calendar.agenda", "Calendar agenda", "CalendarGrid", &["a"]),
    d("calendar.refresh", "Refresh calendar subscriptions", "CalendarGrid", &["r"]),
    d("calendar.open_link", "Open the event's link in the Browser", "CalendarGrid", &["o"]),
    d("calendar.cancel", "Close event form", "CalendarForm", &["escape"]),
    // Sessions.
    d("sessions.filter", "Filter sessions list", "SessionsPanel", &["ctrl-f"]),
    d("sessions.new", "New session", "SessionsPanel", &["alt-n"]),
    d("sessions.toggle_list", "Collapse or expand the sessions list", "SessionsPanel", &["ctrl-b"]),
    // Tasks. Single keys act on the list only, never while a title or date is being typed.
    d("shell.focus.tasks", "Open Tasks", GLOBAL, &["ctrl-7"]),
    d("tasks.previous", "Previous task", "TasksList", &["up"]),
    d("tasks.next", "Next task", "TasksList", &["down"]),
    d("tasks.toggle", "Mark task done or open", "TasksList", &["space"]),
    d("tasks.edit", "Edit task title", "TasksList", &["enter"]),
    d("tasks.due", "Set due date", "TasksList", &["d"]),
    d("tasks.my_day", "Add to or remove from My Day", "TasksList", &["t"]),
    d("tasks.delete", "Delete task", "TasksList", &["delete"]),
    d("tasks.undo_delete", "Undo delete task", "TasksList", &["ctrl-z"]),
    d("tasks.move_up", "Move task up", "TasksList", &["alt-up"]),
    d("tasks.move_down", "Move task down", "TasksList", &["alt-down"]),
    d("tasks.switch_view", "Switch My Day / All", "TasksList", &["v"]),
    d("tasks.add", "Add task", "TasksList", &["n"]),
    d("tasks.to_list", "Back to the task list", "TasksAdd", &["escape", "down"]),
    d("tasks.open_link", "Open the task's link in the Browser", "TasksList", &["o"]),
    d("tasks.cancel", "Cancel task edit", "TasksEdit", &["escape"]),
    // Command palette. It is modal, so its capture kinds deliberately take Ctrl+1–4.
    d("palette.next", "Next result", "AdosPalette", &["ctrl-n"]),
    d("palette.previous", "Previous result", "AdosPalette", &["ctrl-p"]),
    o("palette.thought", "Capture as thought", "AdosPalette", &["ctrl-1"], &["shell.focus.home"]),
    o("palette.file", "Capture a file", "AdosPalette", &["ctrl-2"], &["shell.focus.notes"]),
    o("palette.image", "Capture an image", "AdosPalette", &["ctrl-3"], &["shell.focus.terminal"]),
    o("palette.link", "Capture a link", "AdosPalette", &["ctrl-4"], &["shell.focus.browser"]),
];

pub fn find(id: &str) -> Option<&'static Def> { DEFAULTS.iter().find(|d| d.id == id) }

impl Def {
    pub fn spec(&self) -> Spec {
        Spec {
            id: self.id.into(), title: self.title.into(), scope: self.scope.into(),
            defaults: self.keys.iter().filter_map(|k| super::chord::normalize(k).ok()).collect(),
            overrides: self.overrides.iter().map(|s| s.to_string()).collect(),
        }
    }
}
