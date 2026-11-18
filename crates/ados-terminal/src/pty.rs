//! Alacritty owns the PTY, parser and blocking poll loop. No periodic polling.
use alacritty_terminal::{
    Term,
    event::{Event, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg},
    grid::{Dimensions, Grid},
    index::{Column, Line},
    sync::FairMutex,
    term::{
        Config,
        cell::{Cell, Flags},
    },
    tty,
};
use anyhow::{Context, Result};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub columns: usize,
    pub rows: usize,
    pub cell_width: u16,
    pub cell_height: u16,
}
impl Default for Size {
    fn default() -> Self {
        Self {
            columns: 80,
            rows: 24,
            cell_width: 9,
            cell_height: 20,
        }
    }
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.columns
    }
}
impl From<Size> for WindowSize {
    fn from(s: Size) -> Self {
        Self {
            num_cols: s.columns as u16,
            num_lines: s.rows as u16,
            cell_width: s.cell_width,
            cell_height: s.cell_height,
        }
    }
}

#[derive(Clone)]
pub struct Proxy {
    wake: async_channel::Sender<()>,
    sender: Arc<Mutex<Option<EventLoopSender>>>,
    size: Arc<Mutex<Size>>,
    pub exited: Arc<AtomicBool>,
}
impl Proxy {
    fn reply(&self, text: String) {
        if let Some(sender) = self.sender.lock().as_ref() {
            let _ = sender.send(Msg::Input(text.into_bytes().into()));
        }
    }
}
impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => self.reply(text),
            Event::TextAreaSizeRequest(format) => self.reply(format((*self.size.lock()).into())),
            Event::ColorRequest(index, format) => self.reply(format(crate::colors::indexed(index))),
            // OSC 52 clipboard access is deliberately not granted to shell applications.
            Event::ClipboardLoad(_, format) => self.reply(format("")),
            Event::Exit | Event::ChildExit(_) => {
                self.exited.store(true, Ordering::Release);
                let _ = self.wake.try_send(());
            }
            Event::Wakeup => {
                let _ = self.wake.try_send(());
            }
            _ => {}
        }
    }
}

pub struct Backend {
    pub term: Arc<FairMutex<Term<Proxy>>>,
    pub proxy: Proxy,
    sender: EventLoopSender,
    size: Size,
    shell_pid: u32,
}
impl Backend {
    pub fn new(history: &str) -> Result<(Self, async_channel::Receiver<()>)> {
        let (wake, receiver) = async_channel::bounded(1);
        let size = Size::default();
        let proxy = Proxy {
            wake,
            sender: Arc::default(),
            size: Arc::new(Mutex::new(size)),
            exited: Arc::default(),
        };
        let mut term = Term::new(
            Config {
                scrolling_history: 10_000,
                ..Config::default()
            },
            &size,
            proxy.clone(),
        );
        // Only the first build in this process is the app's start. Later rebuilds are
        // the same session and must not print another restore marker.
        let marker_written = RESTORE_MARKER_WRITTEN.swap(true, Ordering::AcqRel);
        let restored = restore_text(history, marker_written);
        if !restored.is_empty() {
            // Restore plain text as scrollback, never replay executable escape sequences or shell input.
            let text: String = restored
                .chars()
                .filter(|c| !c.is_control() || *c == '\n')
                .collect::<String>()
                .replace('\n', "\r\n");
            let mut parser = alacritty_terminal::vte::ansi::Processor::<
                alacritty_terminal::vte::ansi::StdSyncHandler,
            >::new();
            parser.advance(&mut term, text.as_bytes());
            // Keep the new prompt below the restored transcript, not on its last line.
            if !text.ends_with('\n') { parser.advance(&mut term, b"\r\n"); }
            // Dim the restored rows so the new shell's prompt reads as fresh.
            let grid = term.grid_mut();
            let cursor = grid.cursor.point.line.0;
            let top = -(grid.history_size() as i32);
            for row in top..cursor {
                for column in 0..grid.columns() {
                    grid[Line(row)][Column(column)].flags.insert(Flags::DIM);
                }
            }
        }
        let term = Arc::new(FairMutex::new(term));
        let home: PathBuf = std::env::home_dir().context("Could not find your home folder")?;
        let options = tty::Options {
            shell: Some(shell()),
            working_directory: Some(home),
            env: HashMap::from([
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
            ]),
            ..Default::default()
        };
        let pty = tty::new(&options, size.into(), 0).context("Could not start the shell")?;
        #[cfg(unix)]
        let shell_pid = pty.child().id();
        #[cfg(windows)]
        let shell_pid = pty.child_watcher().pid().map_or(0, |pid| pid.get());
        let event_loop = EventLoop::new(term.clone(), proxy.clone(), pty, true, false)
            .context("Could not start the terminal event loop")?;
        let sender = event_loop.channel();
        *proxy.sender.lock() = Some(sender.clone());
        // Alacritty's thread blocks in poll when idle. Dropping its handle detaches it;
        // Shutdown below makes it exit and drop/reap the PTY child.
        event_loop.spawn();
        Ok((
            Self {
                term,
                proxy,
                sender,
                size,
                shell_pid,
            },
            receiver,
        ))
    }
    /// A job holds the terminal (the foreground process group is not the shell's own).
    #[cfg(unix)]
    pub fn foreground_job(&self) -> bool {
        std::fs::read_to_string(format!("/proc/{}/stat", self.shell_pid))
            .ok()
            .is_some_and(|stat| foreground_job(&stat))
    }
    /// Windows has no foreground group behind ConPTY: a program the shell started (vim, a
    /// build, `ping -t`) is running when the shell has a child process.
    #[cfg(windows)]
    pub fn foreground_job(&self) -> bool {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
        };
        if self.shell_pid == 0 {
            return false;
        }
        // SAFETY: plain Win32 calls on a snapshot handle closed before returning.
        unsafe {
            let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return false };
            let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut found = false;
            let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
            while more && !found {
                found = entry.th32ParentProcessID == self.shell_pid;
                more = Process32NextW(snapshot, &mut entry).is_ok();
            }
            let _ = CloseHandle(snapshot);
            found
        }
    }
    pub fn exited(&self) -> bool {
        self.proxy.exited.load(Ordering::Acquire)
    }
    pub fn write(&self, bytes: Vec<u8>) -> Result<()> {
        if !bytes.is_empty() {
            self.sender.send(Msg::Input(bytes.into()))?;
        }
        Ok(())
    }
    pub fn resize(&mut self, size: Size) -> Result<()> {
        if size != self.size {
            self.term.lock().resize(size);
            *self.proxy.size.lock() = size;
            self.sender.send(Msg::Resize(size.into()))?;
            self.size = size;
        }
        Ok(())
    }
    /// The grid's text as logical lines: a row that wraps into the next row is
    /// joined without a newline, so saved text reflows when the width changes.
    /// `max_lines` keeps the newest logical lines and never starts on a partial wrap.
    pub fn snapshot(&self, max_lines: usize) -> String {
        self.lines(max_lines, false)
    }
    /// What is saved for the next start: the snapshot without the shell's live line
    /// (the prompt the cursor is on). The next shell draws its own prompt, so saving
    /// it stacked one stale empty prompt per relaunch.
    pub fn history(&self, max_lines: usize) -> String {
        self.lines(max_lines, true)
    }
    fn lines(&self, max_lines: usize, without_live_line: bool) -> String {
        let term = self.term.lock();
        let grid = term.grid();
        let top = grid.topmost_line().0;
        let mut bottom = grid.bottommost_line().0;
        if without_live_line {
            // The cursor's logical line starts where the row above stops wrapping.
            bottom = grid.cursor.point.line.0 - 1;
            while bottom >= top && row_wrapped(grid, bottom) {
                bottom -= 1;
            }
        }
        let mut rows: Vec<(String, bool)> = Vec::new();
        let mut lines = 0;
        let mut row = bottom;
        while row >= top {
            let starts = row == top || !row_wrapped(grid, row - 1);
            rows.push((row_text(grid, row), row_wrapped(grid, row)));
            if starts {
                lines += 1;
                if lines >= max_lines {
                    break;
                }
            }
            row -= 1;
        }
        rows.reverse();
        join_rows(rows)
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
        // Break the proxy -> sender link as well, including on failed/finished PTYs.
        self.proxy.sender.lock().take();
    }
}

/// The shell: `$SHELL` (else `/bin/sh`) on Unix; PowerShell 7 (`pwsh.exe`) when it is on
/// `PATH` on Windows, else the Windows PowerShell every Windows has.
fn shell() -> tty::Shell {
    #[cfg(unix)]
    return tty::Shell::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()), vec!["-i".into()]);
    #[cfg(windows)]
    {
        let on_path = |exe: &str| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(exe).is_file()));
        let program = if on_path("pwsh.exe") { "pwsh.exe" } else { "powershell.exe" };
        tty::Shell::new(program.into(), vec!["-NoLogo".into()])
    }
}

/// From `/proc/<shell>/stat`: the terminal's foreground group (`tpgid`) is not the shell's
/// group (`pgrp`), so a job such as `vim` or `sleep` is running in it.
#[cfg_attr(windows, allow(dead_code))]
fn foreground_job(stat: &str) -> bool {
    // The command name may hold spaces and parentheses: fields start after the last ") ".
    let Some((_, rest)) = stat.rsplit_once(") ") else { return false };
    let fields: Vec<&str> = rest.split_whitespace().collect();
    match (fields.get(2).and_then(|f| f.parse::<i64>().ok()), fields.get(5).and_then(|f| f.parse::<i64>().ok())) {
        (Some(pgrp), Some(tpgid)) => tpgid > 0 && tpgid != pgrp,
        _ => false,
    }
}

/// The one line that marks a fresh shell over a restored transcript.
const RESTORE_MARKER: &str = "[Restored terminal history; new shell]";
/// Set on the first panel build in the process; rebuilds share the same session.
static RESTORE_MARKER_WRITTEN: AtomicBool = AtomicBool::new(false);

/// The saved file already holds logical lines. Append the marker only when the
/// first build in this process restores a transcript whose last line is not
/// already the marker.
fn restore_text(history: &str, marker_written: bool) -> String {
    let history = history.trim_end_matches('\n');
    if history.is_empty() { return String::new(); }
    // Markers are restore metadata, not shell output. Collapse inherited copies
    // so app restarts never turn the transcript into a stack of restore notices.
    let had_marker = history.lines().any(|line| line.trim_end() == RESTORE_MARKER);
    let mut lines: Vec<&str> = history.lines()
        .filter(|line| line.trim_end() != RESTORE_MARKER).collect();
    // Files saved before `Backend::history` end in one stale prompt per relaunch.
    while lines.len() > 1 && lines[lines.len() - 1] == lines[lines.len() - 2] {
        lines.pop();
    }
    if !marker_written || had_marker { lines.push(RESTORE_MARKER); }
    lines.join("\n")
}

/// Text of one physical row, without trailing blanks.
fn row_text(grid: &Grid<Cell>, row: i32) -> String {
    let mut text = String::new();
    for column in 0..grid.columns() {
        let cell = &grid[Line(row)][Column(column)];
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        text.push(if cell.c.is_control() { ' ' } else { cell.c });
        if let Some(extra) = cell.zerowidth() {
            text.extend(extra.iter().filter(|c| !c.is_control()));
        }
    }
    if row_wrapped(grid, row) {
        text
    } else {
        text.trim_end().to_owned()
    }
}

/// A row marked WRAPLINE continues on the row below it.
fn row_wrapped(grid: &Grid<Cell>, row: i32) -> bool {
    grid[Line(row)][grid.last_column()]
        .flags
        .contains(Flags::WRAPLINE)
}

/// Join top-to-bottom rows; a wrapped row's text continues without a newline.
fn join_rows(rows: Vec<(String, bool)>) -> String {
    let mut text = String::new();
    for (index, (line, _)) in rows.iter().enumerate() {
        if index > 0 && !rows[index - 1].1 {
            text.push('\n');
        }
        text.push_str(line);
    }
    while text.ends_with('\n') {
        text.pop();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{RESTORE_MARKER, foreground_job, join_rows, restore_text};

    #[test]
    fn foreground_job_from_proc_stat() {
        // pid (comm) state ppid pgrp session tty_nr tpgid ...
        assert!(!foreground_job("4242 (bash) S 4200 4242 4242 34816 4242 4194560 0"));
        assert!(foreground_job("4242 (bash) S 4200 4242 4242 34816 5000 4194560 0"));
        assert!(foreground_job("4242 (my (odd) sh) S 4200 4242 4242 34816 5000 1 0"));
        assert!(!foreground_job("4242 (bash) S 4200 4242 4242 0 -1 1 0"), "no terminal");
        assert!(!foreground_job("garbage"));
    }

    #[test]
    fn wrapped_rows_join_without_a_newline() {
        let rows = vec![
            ("The quick brown fo".to_owned(), true),
            ("x jumps over".to_owned(), false),
            ("a lazy dog".to_owned(), false),
        ];
        assert_eq!(
            join_rows(rows),
            "The quick brown fox jumps over\na lazy dog"
        );
    }

    #[test]
    fn plain_rows_keep_their_newlines() {
        let rows = vec![("one".to_owned(), false), ("two".to_owned(), false)];
        assert_eq!(join_rows(rows), "one\ntwo");
    }

    #[test]
    fn trailing_empty_rows_are_dropped() {
        let rows = vec![("one".to_owned(), false), (String::new(), false)];
        assert_eq!(join_rows(rows), "one");
    }

    #[test]
    fn marker_is_added_to_a_restored_transcript() {
        assert_eq!(
            restore_text("old output", false),
            format!("old output\n{RESTORE_MARKER}")
        );
    }

    #[test]
    fn marker_is_not_added_twice_in_one_start() {
        assert_eq!(restore_text("old output", true), "old output");
    }

    #[test]
    fn marker_is_not_added_when_already_last_line() {
        let history = format!("old output\n{RESTORE_MARKER}");
        assert_eq!(restore_text(&history, false), history);
    }

    #[test]
    fn inherited_markers_are_collapsed() {
        let history = format!("old\n{RESTORE_MARKER}\nnew prompt\n{RESTORE_MARKER}\nnewer");
        let text = restore_text(&history, false);
        assert_eq!(text.matches(RESTORE_MARKER).count(), 1);
        assert!(text.ends_with(RESTORE_MARKER));
    }

    #[test]
    fn stacked_stale_prompts_collapse_to_one() {
        let history = format!("$ echo hi\nhi\n$\n$\n$\n{RESTORE_MARKER}\n$");
        assert_eq!(restore_text(&history, false), format!("$ echo hi\nhi\n$\n{RESTORE_MARKER}"));
    }

    #[test]
    fn restoring_a_restored_history_is_stable() {
        // What `Backend::history` saves after a relaunch with no new commands.
        let once = restore_text("$ echo hi\nhi", false);
        assert_eq!(restore_text(&once, false), once);
    }

    #[test]
    fn empty_history_has_no_marker() {
        assert_eq!(restore_text("", false), "");
    }
}
