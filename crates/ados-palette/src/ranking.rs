//! Deliberate matching: words or contiguous runs, never scattered interior letters.
use ados_core::actions::PaletteAction;

pub(crate) fn destructive(action: &PaletteAction) -> bool {
    action.id.ends_with(".delete")
        || action.id.ends_with(".reset")
        || action.keywords.split(|c: char| !c.is_alphanumeric())
            .any(|word| word.eq_ignore_ascii_case("destructive"))
}

pub(crate) fn preselected(actions: &[PaletteAction], query: &str) -> Option<usize> {
    if query.trim().is_empty() {
        actions.iter().position(|action| !destructive(action))
    } else {
        (!actions.is_empty()).then_some(0)
    }
}

fn word_start(text: &[char], at: usize) -> bool {
    at == 0 || !text[at - 1].is_alphanumeric()
}

// A singleton must begin a word; other matches must belong to a contiguous run.
fn subsequence(query: &[char], text: &[char], initials_only: bool) -> bool {
    if query.len() > text.len() { return false; }
    let mut reachable = vec![vec![false; text.len() + 1]; query.len() + 1];
    reachable[0][0] = true;
    for q in 0..query.len() {
        for from in 0..=text.len() {
            if !reachable[q][from] { continue; }
            for start in from..text.len() {
                let mut len = 0;
                while q + len < query.len() && start + len < text.len()
                    && query[q + len] == text[start + len]
                {
                    len += 1;
                    if (len == 1 && word_start(text, start)) || (!initials_only && len >= 2) {
                        reachable[q + len][start + len] = true;
                    }
                    if initials_only { break; }
                }
            }
        }
    }
    reachable[query.len()].iter().any(|matched| *matched)
}

fn word_prefix(query: &str, text: &str) -> bool {
    text.match_indices(query).any(|(at, _)| {
        at == 0 || !text[..at].chars().next_back().is_some_and(char::is_alphanumeric)
    })
}

fn loose_match(query: &str, text: &str) -> bool {
    subsequence(&query.chars().collect::<Vec<_>>(), &text.chars().collect::<Vec<_>>(), false)
}

fn match_rank(query: &str, title: &str, keywords: &str) -> Option<u8> {
    // Title relevance always beats keyword-only matches, regardless of recency.
    if title == query { return Some(7); }
    if title.split(|c: char| !c.is_alphanumeric()).any(|word| word == query) {
        return Some(6);
    }
    if title.starts_with(query) { return Some(5); }
    if word_prefix(query, title) { return Some(4); }
    if title.contains(query) { return Some(3); }
    if word_prefix(query, keywords) { return Some(2); }
    (loose_match(query, title) || loose_match(query, keywords)).then_some(1)
}

pub(crate) fn ranked(actions: &[PaletteAction], query: &str, recent: &[String]) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut hits: Vec<_> = actions.iter().enumerate().filter_map(|(i, action)| {
        let rank = if query.is_empty() { 0 } else {
            match_rank(&query, &action.title.to_lowercase(), &action.keywords.to_lowercase())?
        };
        // Destructive actions are demoted only in the empty-query recency list.
        let dangerous = query.is_empty() && destructive(action);
        let recency = if dangerous { usize::MAX } else {
            recent.iter().position(|id| id == action.id.as_ref()).unwrap_or(usize::MAX)
        };
        Some((i, rank, dangerous, recency))
    }).collect();
    hits.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2))
        .then(a.3.cmp(&b.3)).then(a.0.cmp(&b.0)));
    hits.into_iter().map(|(i, _, _, _)| i).collect()
}

pub(crate) fn ordered(actions: Vec<PaletteAction>, query: &str, recent: &[String]) -> Vec<PaletteAction> {
    ranked(&actions, query, recent).into_iter().map(|i| actions[i].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn action(id: &str, title: &str, keywords: &str) -> PaletteAction {
        PaletteAction { id: id.to_owned().into(), title: title.to_owned().into(),
            keywords: keywords.to_owned().into(), shortcut: None,
            run: std::rc::Rc::new(|_, _| {}) }
    }
    #[test]
    fn rejects_scattered_interior_letters() {
        assert_eq!(match_rank("mine", "clear terminal", ""), None);
        assert_eq!(match_rank("mine", "focus terminal", ""), None);
        assert!(match_rank("ct", "clear terminal", "").is_some());
        assert!(match_rank("clterm", "clear terminal", "").is_some());
        assert_eq!(match_rank("crl", "clear terminal", ""), None);
        assert_eq!(match_rank("crm", "clear terminal", ""), Some(1));
    }
    #[test]
    fn ranks_match_classes_before_recency() {
        let actions = vec![action("a", "a notebook", ""), action("b", "noteworthy", ""),
            action("c", "note", ""), action("d", "denotes", "")];
        assert_eq!(ranked(&actions, "note", &["d".into()]), vec![2, 1, 0, 3]);
        assert_eq!(ranked(&actions, "no match", &[]), Vec::<usize>::new());
    }
    #[test]
    fn title_classes_beat_keywords_and_recency() {
        let actions = vec![
            action("keyword", "Open Home", "focus"),
            action("word", "Switch layout: Focus", ""),
            action("exact", "Focus", ""),
            action("prefix", "Focusing", ""),
            action("word-prefix", "Start focusing", ""),
            action("substring", "Refocus", ""),
            action("loose", "Find old captures under settings", ""),
            action("keyword-prefix", "Open Notes", "module focusing"),
        ];
        assert_eq!(ranked(&actions, " FOCUS ", &["keyword".into()]),
            vec![2, 1, 3, 4, 5, 0, 7, 6]);
    }
    #[test]
    fn typed_queries_do_not_demote_destructive_actions() {
        let actions = vec![action("notes.delete", "Delete note", ""),
            action("help", "Delete note help", "")];
        assert_eq!(ranked(&actions, "delete", &["notes.delete".into()]), vec![0, 1]);
        assert_eq!(preselected(&actions, "delete"), Some(0));
    }
    #[test]
    fn unicode_keywords_and_long_queries() {
        let actions = vec![action("a", "Écrire", "create edit"), action("b", "Note", "")];
        assert_eq!(ranked(&actions, "ÉC", &[]), vec![0]);
        assert_eq!(ranked(&actions, "EDIT", &[]), vec![0]);
        assert_eq!(ranked(&actions, "longer than either field", &[]), Vec::<usize>::new());
        assert_eq!(match_rank("cmi", "clear terminal", ""), Some(1));
    }
    #[test]
    fn destructive_recency_never_wins() {
        let actions = vec![action("notes.delete", "Delete note", ""),
            action("notes.new", "New note", ""), action("layout.reset", "Reset", ""),
            action("other", "Erase", "DESTRUCTIVE")];
        assert_eq!(ranked(&actions, "", &["notes.delete".into(), "other".into()]), vec![1, 0, 2, 3]);
        assert!(destructive(&actions[0]));
        assert!(destructive(&actions[2]));
        assert!(destructive(&actions[3]));
        assert_eq!(ranked(&actions, "delete", &[]), vec![0]);
        assert_eq!(preselected(&actions, ""), Some(1));
        assert_eq!(preselected(&actions[..1], ""), None);
        assert_eq!(preselected(&actions[2..], "  "), None);
        assert_eq!(preselected(&[], ""), None);
        assert_eq!(preselected(&actions[..1], "delete note"), Some(0));
        assert_eq!(preselected(&actions[2..], "reset"), Some(0));
        assert_eq!(preselected(&[], "delete note"), None);
    }
}
