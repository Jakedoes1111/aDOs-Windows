//! Pure shortcut model: scopes, effective chords, conflicts, swap and replace.
use std::collections::BTreeMap;

pub const GLOBAL: &str = "global";

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub id: String,
    pub title: String,
    /// `global` or a GPUI key context such as `Terminal`.
    pub scope: String,
    pub defaults: Vec<String>,
    /// Ids this shortcut deliberately shadows inside its scope (e.g. Terminal's Ctrl+L).
    pub overrides: Vec<String>,
}

/// Only the user's changes: id → chords (empty = unbound).
pub type UserMap = BTreeMap<String, Vec<String>>;

/// The parent of a key context. Every module context lives inside the shell window.
pub fn parent(scope: &str) -> Option<&'static str> {
    match scope {
        GLOBAL => None,
        "AdosShell" => Some(GLOBAL),
        "NotesList" => Some("NotesPanel"),
        _ => Some("AdosShell"),
    }
}

fn is_ancestor_or_self(ancestor: &str, scope: &str) -> bool {
    let mut current = Some(scope);
    while let Some(s) = current {
        if s == ancestor { return true; }
        current = parent(s);
    }
    false
}

/// Two scopes can be active together when one contains the other.
pub fn overlaps(a: &str, b: &str) -> bool { is_ancestor_or_self(a, b) || is_ancestor_or_self(b, a) }

/// Readable scope name for the editor.
pub fn scope_title(scope: &str) -> String {
    match scope {
        GLOBAL | "AdosShell" => "Everywhere".into(),
        super::defaults::ZOOM_SCOPE => "Everywhere except a Browser page".into(),
        "NotesPanel" => "Notes".into(),
        "NotesList" => "Notes list".into(),
        "BrowserPanel" => "Browser".into(),
        "BrowserPage" => "Browser page".into(),
        "AdosPalette" => "Command palette".into(),
        "TasksList" => "Tasks list".into(),
        "TasksAdd" => "Tasks quick add".into(),
        "TasksEdit" => "Tasks editing".into(),
        "SessionsPanel" => "Sessions".into(),
        "CalendarGrid" => "Calendar".into(),
        "CalendarForm" => "Calendar event form".into(),
        other => other.into(),
    }
}

pub fn effective(specs: &[Spec], user: &UserMap) -> Vec<Vec<String>> {
    specs.iter().map(|s| user.get(&s.id).cloned().unwrap_or_else(|| s.defaults.clone())).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clash { pub chord: String, pub a: usize, pub b: usize, pub intended: bool }

fn intended(a: &Spec, b: &Spec) -> bool { a.overrides.contains(&b.id) || b.overrides.contains(&a.id) }

/// Every pair sharing a chord in overlapping scopes. `intended` pairs are overrides, not conflicts.
pub fn clashes(specs: &[Spec], chords: &[Vec<String>]) -> Vec<Clash> {
    let mut out = vec![];
    for a in 0..specs.len() {
        for b in a + 1..specs.len() {
            if !overlaps(&specs[a].scope, &specs[b].scope) { continue; }
            for chord in &chords[a] {
                if chords[b].contains(chord) {
                    out.push(Clash { chord: chord.clone(), a, b, intended: intended(&specs[a], &specs[b]) });
                }
            }
        }
    }
    out
}

pub fn conflicts(specs: &[Spec], chords: &[Vec<String>]) -> Vec<Clash> {
    clashes(specs, chords).into_iter().filter(|c| !c.intended).collect()
}

/// Who else would fire for `chord` if shortcut `index` used it (intended overrides excluded).
pub fn conflict_for(specs: &[Spec], chords: &[Vec<String>], index: usize, chord: &str) -> Option<usize> {
    (0..specs.len()).find(|&o| o != index
        && overlaps(&specs[o].scope, &specs[index].scope)
        && !intended(&specs[o], &specs[index])
        && chords[o].iter().any(|c| c == chord))
}

/// Store chords for one shortcut, keeping only real changes in the user map.
pub fn set(user: &mut UserMap, spec: &Spec, chords: Vec<String>) {
    if chords == spec.defaults { user.remove(&spec.id); } else { user.insert(spec.id.clone(), chords); }
}

/// Plain assignment: `index` now uses only `chord`.
pub fn assign(user: &mut UserMap, specs: &[Spec], index: usize, chord: &str) {
    set(user, &specs[index], vec![chord.to_string()]);
}

/// `index` takes `chord`; `other` loses it and gets `index`'s old chords instead.
pub fn swap(user: &mut UserMap, specs: &[Spec], index: usize, other: usize, chord: &str) {
    let chords = effective(specs, user);
    let mut theirs: Vec<String> = chords[other].iter().filter(|c| *c != chord).cloned().collect();
    for old in &chords[index] {
        if old != chord && !theirs.contains(old) { theirs.push(old.clone()); }
    }
    set(user, &specs[other], theirs);
    set(user, &specs[index], vec![chord.to_string()]);
}

/// `index` takes `chord`; `other` simply loses it.
pub fn replace(user: &mut UserMap, specs: &[Spec], index: usize, other: usize, chord: &str) {
    let chords = effective(specs, user);
    let theirs = chords[other].iter().filter(|c| *c != chord).cloned().collect();
    set(user, &specs[other], theirs);
    set(user, &specs[index], vec![chord.to_string()]);
}
