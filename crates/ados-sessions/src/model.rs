//! Pure logic for the Sessions panel: list rows, filtering, ordering, labels and the
//! transcript built from agent events. No GPUI here, so it is unit tested.
use ados_agents::{AgentEvent, ToolStatus};
use std::path::{Path, PathBuf};

/// One session in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The `backends.toml` id.
    pub backend: String,
    pub id: String,
    pub cwd: PathBuf,
    pub title: Option<String>,
    /// RFC 3339, as the agent reports it (or set by aDOs after a turn).
    pub updated_at: Option<String>,
}

impl Entry {
    pub fn key(&self) -> String {
        key(&self.backend, &self.id)
    }
}

/// The stable key of a session across backends: `backend/id`.
pub fn key(backend: &str, id: &str) -> String {
    format!("{backend}/{id}")
}

/// Split a key back into backend id and session id.
pub fn split_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('/')
}

/// Newest first by `updated_at` (RFC 3339 strings sort as text); undated last, stable.
pub fn sort_newest(entries: &mut [Entry]) {
    entries.sort_by(|a, b| match (&a.updated_at, &b.updated_at) {
        (Some(a), Some(b)) => b.cmp(a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
}

/// Whether a row passes the filter box text and the backend filter.
pub fn matches(entry: &Entry, query: &str, backend: Option<&str>, backend_name: &str) -> bool {
    if backend.is_some_and(|b| b != entry.backend) {
        return false;
    }
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {}",
        label(entry),
        entry.cwd.display(),
        backend_name,
        entry.id
    )
    .to_lowercase();
    query.split_whitespace().all(|word| hay.contains(word))
}

/// The row title: the first meaningful line of the title, else "Session <short id>".
pub fn label(entry: &Entry) -> String {
    match entry.title.as_deref().and_then(first_meaningful_line) {
        Some(line) => line.chars().take(80).collect(),
        None => format!("Session {}", entry.id.chars().take(8).collect::<String>()),
    }
}

/// The first line worth showing: skips a closed front-matter block, `---` rules and blanks.
pub fn first_meaningful_line(text: &str) -> Option<&str> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut start = 0;
    while lines.get(start).is_some_and(|l| l.is_empty()) {
        start += 1;
    }
    if lines.get(start) == Some(&"---") {
        if let Some(end) = lines[start + 1..].iter().position(|l| *l == "---") {
            let block = &lines[start + 1..start + 1 + end];
            if block.iter().all(|l| l.is_empty() || l.contains(':')) {
                start += end + 2;
            }
        }
    }
    lines[start..]
        .iter()
        .copied()
        .find(|l| !l.is_empty() && l.chars().any(|c| c != '-'))
}

/// The folder shown on a row: `~` for the home folder, `~/x` under it, else the path.
pub fn folder_label(cwd: &Path, home: &Path) -> String {
    if cwd == home {
        return "~".into();
    }
    match cwd.strip_prefix(home) {
        Ok(rest) if !home.as_os_str().is_empty() && home != Path::new("/") => {
            format!("~/{}", rest.display())
        }
        _ => cwd.display().to_string(),
    }
}

/// "13 Nov 09:30" from an RFC 3339 time (shown as the agent wrote it, usually UTC).
/// Anything unreadable is shown as it is.
pub fn time_label(updated: &str) -> String {
    const MONTHS: [&str; 12] =
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let parsed = (|| {
        let month: usize = updated.get(5..7)?.parse().ok()?;
        let day: u32 = updated.get(8..10)?.parse().ok()?;
        let time = updated.get(11..16)?;
        (updated.as_bytes().get(10) == Some(&b'T') || updated.as_bytes().get(10) == Some(&b' '))
            .then_some(())?;
        Some(format!("{day} {} {time}", MONTHS.get(month.checked_sub(1)?)?))
    })();
    parsed.unwrap_or_else(|| updated.to_string())
}

/// Now as RFC 3339 UTC (`2026-11-13T15:46:00Z`), for sessions aDOs just touched.
pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339(secs)
}

pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// One item of an open session's transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    User(String),
    Agent(String),
    Thought(String),
    Tool { id: String, kind: String, title: String, status: ToolStatus },
}

/// What the right side shows for one session.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub items: Vec<Item>,
    /// `session/load` is replaying it: user chunks are taken as user items.
    pub replaying: bool,
    /// The session is open in the running agent (loaded or new), so it can take prompts.
    pub live: bool,
    /// A plain sentence when opening or a turn failed.
    pub error: Option<String>,
}

impl Transcript {
    /// Fold one agent event for this session into the transcript.
    pub fn apply(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::Text { text, .. } => self.append(text, Item::Agent),
            AgentEvent::Thought { text, .. } => self.append(text, Item::Thought),
            AgentEvent::UserText { text, .. } if self.replaying => self.append(text, Item::User),
            AgentEvent::UserText { .. } => {}
            AgentEvent::ToolCall { id, title, kind, status, .. } => {
                // Agents may reuse a tool id in a later turn: only this turn's step is the same one.
                if let Some(Item::Tool { title: t, status: s, kind: k, .. }) = self.tool_this_turn(id) {
                    (*t, *s, *k) = (title.clone(), *status, kind.clone());
                } else {
                    self.items.push(Item::Tool {
                        id: id.clone(),
                        kind: kind.clone(),
                        title: title.clone(),
                        status: *status,
                    });
                }
            }
            AgentEvent::ToolCallUpdate { id, title, status, .. } => {
                if let Some(Item::Tool { title: t, status: s, .. }) = self.tool(id) {
                    if let Some(title) = title {
                        *t = title.clone();
                    }
                    if let Some(status) = status {
                        *s = *status;
                    }
                }
            }
            AgentEvent::Crashed { .. } => {}
        }
    }

    pub fn push_user(&mut self, text: &str) {
        self.items.push(Item::User(text.to_string()));
    }

    fn tool_this_turn(&mut self, id: &str) -> Option<&mut Item> {
        let start = self.items.iter().rposition(|i| matches!(i, Item::User(_))).map_or(0, |i| i + 1);
        self.items[start..]
            .iter_mut()
            .find(|item| matches!(item, Item::Tool { id: i, .. } if i == id))
    }

    fn tool(&mut self, id: &str) -> Option<&mut Item> {
        self.items
            .iter_mut()
            .rev()
            .find(|item| matches!(item, Item::Tool { id: i, .. } if i == id))
    }

    fn append(&mut self, text: &str, make: fn(String) -> Item) {
        let same = matches!(
            (self.items.last(), make(String::new())),
            (Some(Item::Agent(_)), Item::Agent(_))
                | (Some(Item::Thought(_)), Item::Thought(_))
                | (Some(Item::User(_)), Item::User(_))
        );
        match self.items.last_mut() {
            Some(Item::Agent(s) | Item::Thought(s) | Item::User(s)) if same => s.push_str(text),
            _ => self.items.push(make(text.to_string())),
        }
    }

    /// The last messages as plain text, for the assistant's context (at most `cap` bytes).
    pub fn tail_text(&self, cap: usize) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut size = 0;
        for item in self.items.iter().rev() {
            let line = match item {
                Item::User(t) => format!("You: {t}"),
                Item::Agent(t) => format!("Agent: {t}"),
                Item::Thought(_) => continue,
                Item::Tool { title, status, .. } => format!("[tool {}: {title}]", status_word(*status)),
            };
            size += line.len() + 2;
            if size > cap && !parts.is_empty() {
                break;
            }
            parts.push(line);
        }
        parts.reverse();
        let mut text = parts.join("\n\n");
        if text.len() > cap {
            let mut start = text.len() - cap;
            while !text.is_char_boundary(start) {
                start += 1;
            }
            text = text[start..].to_string();
        }
        text
    }
}

/// A tool step's status in plain words.
pub fn status_word(status: ToolStatus) -> &'static str {
    match status {
        ToolStatus::Pending | ToolStatus::InProgress => "running",
        ToolStatus::Completed => "done",
        ToolStatus::Failed => "failed",
    }
}

/// The sentence for a failed backend start: "not installed" when the command is missing.
pub fn start_error(name: &str, command: &str, raw: &str) -> String {
    if raw.contains("No such file or directory") || raw.contains("os error 2") {
        format!("{name} isn't installed here: `{command}` was not found.")
    } else {
        raw.to_owned()
    }
}

/// A backend problem as one short list line that names the backend:
/// "Codex: authentication required", "Claude Code: not installed".
pub fn problem_line(name: &str, raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with(&format!("{name} isn't installed here")) {
        return format!("{name}: not installed");
    }
    let mut rest = raw;
    for prefix in ["Could not list sessions:", "Could not open the session:", "Invalid params:", "Internal error:"] {
        rest = rest.strip_prefix(prefix).map_or(rest, str::trim_start);
    }
    let rest = rest.trim_end_matches('.').trim();
    let mut chars = rest.chars();
    let rest = match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => "stopped".to_string(),
    };
    format!("{name}: {rest}")
}

/// Whether a problem line stays visible while the list is filtered.
pub fn problem_matches(line: &str, query: &str) -> bool {
    let line = line.to_lowercase();
    query.to_lowercase().split_whitespace().all(|word| line.contains(word))
}

/// The sentence shown when a session could not be opened, without protocol words.
pub fn open_error(name: &str, raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("unknown session") || lower.contains("not found") {
        format!("{name} no longer has this session. Pick another one or start a new session.")
    } else if lower.contains("auth") {
        format!("{name} needs you to sign in before it can open this session.")
    } else {
        format!("{name} could not open this session. Pick it again to retry.")
    }
}

/// A new session's title, from the first message: its first meaningful line, trimmed.
pub fn title_from_message(text: &str) -> Option<String> {
    let line = first_meaningful_line(text)?;
    let mut title: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        title = format!("{}…", title.trim_end());
    }
    Some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_lines_name_the_backend_and_stay_short() {
        assert_eq!(
            problem_line("Codex", "Could not list sessions: Authentication required"),
            "Codex: authentication required"
        );
        assert_eq!(
            problem_line("Claude Code", "Claude Code isn't installed here: `claude-agent-acp` was not found."),
            "Claude Code: not installed"
        );
        assert!(problem_matches("Codex: authentication required", "codex"));
        assert!(!problem_matches("Codex: authentication required", "plan"));
    }

    #[test]
    fn open_error_hides_protocol_text() {
        let text = open_error("Fake agent", "Could not open the session: Invalid params: \"Unknown sessionId: fake-2\"");
        assert_eq!(text, "Fake agent no longer has this session. Pick another one or start a new session.");
        assert!(!text.contains("Invalid params"));
    }

    #[test]
    fn title_comes_from_the_first_message() {
        assert_eq!(title_from_message("  \n  Plan the trip to Leeds  \nmore").as_deref(), Some("Plan the trip to Leeds"));
        assert_eq!(title_from_message("   "), None);
        let long = "word ".repeat(30);
        let title = title_from_message(&long).unwrap_or_default();
        assert!(title.ends_with('…') && title.chars().count() <= 61);
    }

    fn entry(backend: &str, id: &str, updated: Option<&str>, title: Option<&str>) -> Entry {
        Entry {
            backend: backend.into(),
            id: id.into(),
            cwd: PathBuf::from("/home/j/unity"),
            title: title.map(Into::into),
            updated_at: updated.map(Into::into),
        }
    }

    #[test]
    fn newest_first_across_backends_undated_last() {
        let mut list = vec![
            entry("pi", "a", Some("2026-11-10T10:00:00Z"), None),
            entry("fake", "b", None, None),
            entry("claude", "c", Some("2026-11-13T08:00:00Z"), None),
        ];
        sort_newest(&mut list);
        let ids: Vec<_> = list.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["c", "a", "b"]);
    }

    #[test]
    fn filter_by_words_and_backend() {
        let e = entry("pi", "abc", None, Some("Plan the week"));
        assert!(matches(&e, "", None, "pi"));
        assert!(matches(&e, "week plan", None, "pi"));
        assert!(matches(&e, "unity", None, "pi"));
        assert!(!matches(&e, "codex", None, "pi"));
        assert!(matches(&e, "", Some("pi"), "pi"));
        assert!(!matches(&e, "", Some("claude"), "pi"));
    }

    #[test]
    fn label_skips_front_matter() {
        let e = entry("pi", "abcdefghijk", None, Some("---\nname: weekly\n---\n\nPlan the week"));
        assert_eq!(label(&e), "Plan the week");
        assert_eq!(label(&entry("pi", "abcdefghijk", None, None)), "Session abcdefgh");
        assert_eq!(label(&entry("pi", "x", None, Some("---\n"))), "Session x");
    }

    #[test]
    fn keys_round_trip() {
        assert_eq!(split_key(&key("fake", "old-1")), Some(("fake", "old-1")));
    }

    #[test]
    fn folder_and_time_labels() {
        let home = Path::new("/home/j");
        assert_eq!(folder_label(Path::new("/home/j"), home), "~");
        assert_eq!(folder_label(Path::new("/home/j/unity"), home), "~/unity");
        assert_eq!(folder_label(Path::new("/tmp"), home), "/tmp");
        assert_eq!(time_label("2026-11-13T08:05:00Z"), "13 Nov 08:05");
        assert_eq!(time_label("yesterday"), "yesterday");
    }

    #[test]
    fn rfc3339_from_epoch_seconds() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_794_584_760), "2026-11-13T15:46:00Z");
    }

    #[test]
    fn transcript_folds_replay_and_live_events() {
        let s = || "s".to_string();
        let mut t = Transcript { replaying: true, ..Default::default() };
        for e in [
            AgentEvent::UserText { session_id: s(), text: "earlier ".into() },
            AgentEvent::UserText { session_id: s(), text: "question".into() },
            AgentEvent::Thought { session_id: s(), text: "Looking".into() },
            AgentEvent::ToolCall { session_id: s(), id: "t".into(), title: "Read".into(), kind: "read".into(), status: ToolStatus::Pending },
            AgentEvent::ToolCallUpdate { session_id: s(), id: "t".into(), title: None, status: Some(ToolStatus::Completed) },
            AgentEvent::Text { session_id: s(), text: "earlier ".into() },
            AgentEvent::Text { session_id: s(), text: "answer".into() },
        ] {
            t.apply(&e);
        }
        t.replaying = false;
        t.apply(&AgentEvent::UserText { session_id: s(), text: "ignored live echo".into() });
        assert_eq!(
            t.items,
            vec![
                Item::User("earlier question".into()),
                Item::Thought("Looking".into()),
                Item::Tool { id: "t".into(), kind: "read".into(), title: "Read".into(), status: ToolStatus::Completed },
                Item::Agent("earlier answer".into()),
            ]
        );
        assert_eq!(t.tail_text(1000), "You: earlier question\n\n[tool done: Read]\n\nAgent: earlier answer");
        assert_eq!(t.tail_text(25), "Agent: earlier answer");
    }

    #[test]
    fn a_reused_tool_id_in_a_later_turn_is_a_new_step() {
        let s = || "s".to_string();
        let call = |status| AgentEvent::ToolCall { session_id: s(), id: "tool-1".into(), title: "Write".into(), kind: "edit".into(), status };
        let done = |status| AgentEvent::ToolCallUpdate { session_id: s(), id: "tool-1".into(), title: None, status: Some(status) };
        let mut t = Transcript::default();
        t.push_user("one");
        t.apply(&call(ToolStatus::Pending));
        t.apply(&done(ToolStatus::Completed));
        t.apply(&AgentEvent::Text { session_id: s(), text: "ok".into() });
        t.push_user("two");
        t.apply(&call(ToolStatus::Pending));
        t.apply(&done(ToolStatus::Failed));
        let statuses: Vec<_> = t.items.iter().filter_map(|i| match i { Item::Tool { status, .. } => Some(*status), _ => None }).collect();
        assert_eq!(statuses, [ToolStatus::Completed, ToolStatus::Failed]);
    }

    #[test]
    fn missing_command_reads_as_not_installed() {
        assert_eq!(
            start_error("Claude Code", "claude-agent-acp", "Internal error: \"No such file or directory (os error 2)\""),
            "Claude Code isn't installed here: `claude-agent-acp` was not found."
        );
        assert_eq!(start_error("X", "x", "boom"), "boom");
    }
}
