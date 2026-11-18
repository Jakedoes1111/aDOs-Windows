//! Pure helpers for the assistant: prompt text, the update-notice filter, the
//! backend picker and the chats order. No GPUI, so they are unit tested.
use ados_agents::{BackendConfig, SessionSummary};
use std::path::Path;

/// A piece of workspace context sent with a prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextItem {
    pub label: String,
    pub text: String,
}

/// The aDOs preamble the assistant's new chats get with their first prompt.
/// Edit `preamble.md` to change it.
pub const PREAMBLE: &str = include_str!("preamble.md");

/// A new chat's first prompt: the text, then the preamble under its own rule.
pub fn with_preamble(prompt: &str) -> String {
    format!("{prompt}\n\n---\n{}", PREAMBLE.trim())
}

/// The text sent to the agent: the user's words, then each context item under its label.
/// ACP prompts are plain content blocks, so context travels inside the text.
pub fn prompt_text(text: &str, context: &[ContextItem]) -> String {
    let mut out = text.trim_end().to_owned();
    if context.is_empty() {
        return out;
    }
    out.push_str("\n\n---\nContext from aDOs:");
    for item in context {
        out.push_str(&format!("\n\n[{}]\n{}", item.label, item.text.trim_end()));
    }
    out
}

/// pi prints "New version available …" into its replies; pi-acp forwards it as agent text.
fn is_notice(line: &str) -> bool {
    line.trim_start().starts_with("New version available")
}

/// Split agent text into update notices and the reply proper.
/// Returns `(notices, reply)`; the reply keeps every other line as sent.
pub fn split_notice(text: &str) -> (Vec<String>, String) {
    if !text.lines().any(is_notice) {
        return (Vec::new(), text.to_owned());
    }
    let mut notices = Vec::new();
    let mut reply = Vec::new();
    for line in text.lines() {
        if is_notice(line) {
            notices.push(line.trim().to_owned());
        } else {
            reply.push(line);
        }
    }
    let reply = reply.join("\n");
    (notices, reply.trim_start_matches('\n').to_owned())
}

/// The backend the picker shows: the saved id if it still exists, else the first configured one.
pub fn pick_backend<'a>(saved: Option<&str>, backends: &'a [BackendConfig]) -> Option<&'a BackendConfig> {
    saved
        .and_then(|id| backends.iter().find(|b| b.id == id))
        .or_else(|| backends.first())
}

/// Newest first by `updated_at` (RFC 3339 strings sort as text); undated sessions last,
/// in the agent's own order.
pub fn newest_first(mut sessions: Vec<SessionSummary>) -> Vec<SessionSummary> {
    sessions.sort_by(|a, b| match (&a.updated_at, &b.updated_at) {
        (Some(a), Some(b)) => b.cmp(a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    sessions
}

/// The label for a chat in the list: the first meaningful line of its title (front
/// matter, `---` rules and blank lines skipped), else a short id.
pub fn chat_label(session: &SessionSummary) -> String {
    match session.title.as_deref().and_then(first_meaningful_line) {
        Some(line) => line.chars().take(60).collect(),
        None => format!("Chat {}", session.id.chars().take(8).collect::<String>()),
    }
}

/// The first line worth showing: skips a leading front-matter block (`---` … `---`),
/// bare `---` rules and blank lines.
pub fn first_meaningful_line(text: &str) -> Option<&str> {
    let mut lines = text.lines().map(str::trim).peekable();
    while lines.peek().is_some_and(|l| l.is_empty()) {
        lines.next();
    }
    if lines.peek() == Some(&"---") {
        lines.next();
        let rest: Vec<&str> = lines.clone().collect();
        // Front matter only if it is closed; otherwise `---` was just a rule.
        if let Some(end) = rest.iter().position(|l| *l == "---") {
            if rest[..end].iter().all(|l| l.is_empty() || l.contains(':')) {
                for _ in 0..=end {
                    lines.next();
                }
            }
        }
    }
    lines.find(|l| !l.is_empty() && l.chars().any(|c| c != '-'))
}

/// The chats the list shows: those started in `cwd`, or every chat with `all_folders`.
pub fn chats_in(sessions: &[SessionSummary], cwd: &Path, all_folders: bool) -> Vec<SessionSummary> {
    sessions
        .iter()
        .filter(|s| all_folders || same_dir(&s.cwd, cwd))
        .cloned()
        .collect()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a.components().eq(b.components())
}

/// The folder shown under a chat's title when every folder is listed: its last segment.
pub fn folder_label(cwd: &Path) -> String {
    cwd.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.display().to_string())
}

/// The sentence for a failed backend start: a plain "not installed" when the command
/// does not exist, else the agent layer's own error.
pub fn start_error(name: &str, command: &str, raw: &str) -> String {
    if raw.contains("No such file or directory") || raw.contains("os error 2") {
        format!("{name} isn't installed here: `{command}` was not found.")
    } else {
        raw.to_owned()
    }
}

/// The sentence for a failed `session/new`: one plain sentence, no JSON, no error chains.
/// A missing program (ENOENT, "executable not found") reads as "not installed".
pub fn chat_start_error(name: &str, raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("enoent") || lower.contains("executable not found") || lower.contains("os error 2")
        || lower.contains("no such file or directory") {
        return format!("Couldn't start a chat: {name} isn't installed here. Install it, then Retry.");
    }
    // Drop JSON details and "Internal error:" style prefixes; keep the agent's own sentence.
    let text = raw.split(['{', '[']).next().unwrap_or(raw);
    let text = text.rsplit(": ").find(|part| part.trim().len() > 12).unwrap_or(text);
    let mut sentence: String = text.trim().trim_end_matches([':', ' ']).chars().take(160).collect();
    if sentence.is_empty() {
        sentence = format!("{name} did not answer");
    }
    if !sentence.ends_with(['.', '!', '?']) {
        sentence.push('.');
    }
    format!("Couldn't start a chat: {sentence}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn backend(id: &str) -> BackendConfig {
        BackendConfig {
            id: id.into(),
            name: id.to_uppercase(),
            command: id.into(),
            args: vec![],
            env: Default::default(),
        }
    }

    fn session(id: &str, updated: Option<&str>, title: Option<&str>) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            cwd: PathBuf::from("/"),
            title: title.map(Into::into),
            updated_at: updated.map(Into::into),
        }
    }

    #[test]
    fn notice_lines_are_split_from_the_reply() {
        let (notices, reply) =
            split_notice("New version available: 0.9 (you have 0.8). Run npm i -g pi\nHello there.");
        assert_eq!(notices, vec!["New version available: 0.9 (you have 0.8). Run npm i -g pi"]);
        assert_eq!(reply, "Hello there.");
    }

    #[test]
    fn plain_reply_is_untouched() {
        let text = "A new version of the doc is available.\n\nSecond paragraph.\n";
        assert_eq!(split_notice(text), (vec![], text.to_owned()));
    }

    #[test]
    fn notice_alone_leaves_empty_reply() {
        let (notices, reply) = split_notice("  New version available …\n");
        assert_eq!(notices.len(), 1);
        assert_eq!(reply, "");
    }

    #[test]
    fn preamble_follows_the_first_prompt() {
        let p = with_preamble("hi");
        assert!(p.starts_with("hi\n\n---\nAbout aDOs:"));
        assert!(p.contains("MCP server named \"ados\""));
    }

    #[test]
    fn prompt_without_context_is_the_text() {
        assert_eq!(prompt_text("hi\n", &[]), "hi");
    }

    #[test]
    fn prompt_carries_context_under_labels() {
        let context = [ContextItem { label: "This module · Notes".into(), text: "Body".into() }];
        assert_eq!(
            prompt_text("Summarise", &context),
            "Summarise\n\n---\nContext from aDOs:\n\n[This module · Notes]\nBody"
        );
    }

    #[test]
    fn picker_keeps_saved_backend_or_falls_back_to_first() {
        let backends = [backend("pi"), backend("claude"), backend("codex")];
        assert_eq!(pick_backend(Some("claude"), &backends).map(|b| b.id.as_str()), Some("claude"));
        assert_eq!(pick_backend(Some("gone"), &backends).map(|b| b.id.as_str()), Some("pi"));
        assert_eq!(pick_backend(None, &backends).map(|b| b.id.as_str()), Some("pi"));
        assert!(pick_backend(Some("pi"), &[]).is_none());
    }

    #[test]
    fn chats_sort_newest_first_with_undated_last() {
        let sorted = newest_first(vec![
            session("a", None, None),
            session("b", Some("2026-11-01T10:00:00Z"), None),
            session("c", Some("2026-11-13T09:00:00Z"), None),
        ]);
        let ids: Vec<_> = sorted.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["c", "b", "a"]);
    }

    #[test]
    fn chat_label_prefers_title() {
        assert_eq!(chat_label(&session("old-1", None, Some("Earlier work"))), "Earlier work");
        assert_eq!(chat_label(&session("0123456789", None, Some("  "))), "Chat 01234567");
        assert_eq!(chat_label(&session("0123456789", None, None)), "Chat 01234567");
    }

    #[test]
    fn title_skips_front_matter_and_rules() {
        let front = "---\nname: plan\ntags: [a]\n---\n\nWrite the release notes\nmore";
        assert_eq!(first_meaningful_line(front), Some("Write the release notes"));
        assert_eq!(first_meaningful_line("\n---\n\nFix the card\n"), Some("Fix the card"));
        assert_eq!(first_meaningful_line("  Plain title  "), Some("Plain title"));
        assert_eq!(first_meaningful_line("---\n---\n"), None);
        assert_eq!(first_meaningful_line(""), None);
        assert_eq!(chat_label(&session("abcdefghij", None, Some("---\n"))), "Chat abcdefgh");
        assert_eq!(chat_label(&session("x", None, Some("---\ntitle: T\n---\nBody line"))), "Body line");
    }

    #[test]
    fn chats_filter_by_folder_unless_all() {
        let mut home = session("h", None, None);
        home.cwd = PathBuf::from("/home/me");
        let mut slash = session("s", None, None);
        slash.cwd = PathBuf::from("/home/me/");
        let mut other = session("o", None, None);
        other.cwd = PathBuf::from("/home/me/unity");
        let all = [home, slash, other];
        let ids = |v: Vec<SessionSummary>| v.into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(ids(chats_in(&all, Path::new("/home/me"), false)), ["h", "s"]);
        assert_eq!(ids(chats_in(&all, Path::new("/home/me"), true)), ["h", "s", "o"]);
        assert_eq!(folder_label(Path::new("/home/me/unity")), "unity");
        assert_eq!(folder_label(Path::new("/")), "/");
    }

    #[test]
    fn missing_command_says_not_installed() {
        let raw = "Could not start Claude Code (claude-agent-acp): Internal error: \"No such file or directory (os error 2)\"";
        assert_eq!(
            start_error("Claude Code", "claude-agent-acp", raw),
            "Claude Code isn't installed here: `claude-agent-acp` was not found."
        );
        assert_eq!(start_error("pi", "npx", "Could not start pi (npx): it exited."), "Could not start pi (npx): it exited.");
    }

    #[test]
    fn chat_start_error_is_one_plain_sentence() {
        let raw = "Could not start a session: Internal error: Could not start pi: executable not found (command: pi). Pi needs to be installed before it can run in ACP clients. Then try again.: {\n  \"code\": \"ENOENT\"\n}";
        assert_eq!(chat_start_error("pi", raw), "Couldn't start a chat: pi isn't installed here. Install it, then Retry.");
        let other = chat_start_error("Fake", "Could not start a session: Internal error: the model refused the request: {\"code\": 7}");
        assert_eq!(other, "Couldn't start a chat: the model refused the request.");
        assert!(!other.contains('{'));
        let long = chat_start_error("Fake", &"x".repeat(500));
        assert!(long.len() < 200);
    }
}
