//! One registry for every aDOs shortcut. Crates register here, never with `cx.bind_keys`.
//! The user's changes live in `ADOS_HOME/keymap.toml` and are hot-reloaded on save.
mod chord;
pub mod defaults;
mod file;
mod model;
#[cfg(test)]
mod tests;

pub use chord::{caps, label, normalize, warning, Warning};
pub use model::{assign, clashes, conflict_for, overlaps, replace, scope_title, set, swap, Clash, Spec, UserMap, GLOBAL};
use gpui_kit::*;
use std::{path::PathBuf, rc::Rc, time::{Duration, SystemTime}};

/// Marks the bindings this registry owns so a rebuild keeps the library's own bindings.
const OURS: KeyBindingMetaIndex = KeyBindingMetaIndex(0xAD05);

struct Entry { spec: Spec, action: Box<dyn Action> }

pub enum Notice { Problem(String), Conflicts(usize) }

#[derive(Default)]
pub struct Shortcuts {
    entries: Vec<Entry>,
    user: UserMap,
    chords: Vec<Vec<String>>,
    /// Chords the embedded browser page must hand back to aDOs, with their entry index.
    native: Vec<(String, usize)>,
    notices: Vec<Notice>,
    stamp: Option<SystemTime>,
    version: u64,
    started: bool,
}
impl Global for Shortcuts {}

fn state(cx: &mut App) -> &mut Shortcuts {
    if !cx.has_global::<Shortcuts>() { cx.set_global(Shortcuts::default()); }
    cx.global_mut::<Shortcuts>()
}

pub fn file_path() -> PathBuf { crate::paths::home().join("keymap.toml") }

/// Attach an action to a shortcut from `defaults::DEFAULTS`. Several actions may not share an id.
pub fn bind<A: Action>(cx: &mut App, id: &str, action: A) {
    let Some(def) = defaults::find(id) else {
        state(cx).notices.push(Notice::Problem(format!("Shortcut “{id}” is not in the registry")));
        return;
    };
    let spec = def.spec();
    let initial: Vec<KeyBinding> = spec.defaults.iter()
        .filter_map(|c| binding(c, &spec.scope, action.boxed_clone(), cx)).collect();
    let s = state(cx);
    s.entries.retain(|e| e.spec.id != id);
    s.entries.push(Entry { spec, action: action.boxed_clone() });
    // Before `start`, bind this one at once so standalone module examples work too.
    if s.started { apply(cx); } else { cx.bind_keys(initial); }
}

fn binding(chord: &str, scope: &str, action: Box<dyn Action>, cx: &App) -> Option<KeyBinding> {
    binding_unless(chord, scope, &[], action, cx)
}
/// `unless`: scopes where a shortcut that overrides this one owns the chord. GPUI ranks a
/// binding without context as the deepest match, so a global chord would beat the override
/// whenever focus sits in a nested context (an input inside the Browser); excluding those
/// scopes here lets the override win everywhere inside them.
fn binding_unless(chord: &str, scope: &str, unless: &[&str], action: Box<dyn Action>, cx: &App) -> Option<KeyBinding> {
    let mut parts: Vec<String> = if scope == GLOBAL { vec![] } else { vec![scope.to_string()] };
    parts.extend(unless.iter().map(|u| format!("!{u}")));
    let predicate = if parts.is_empty() { None } else {
        Some(Rc::new(KeyBindingContextPredicate::parse(&parts.join(" && ")).ok()?))
    };
    KeyBinding::load(chord, action, predicate, false, None, cx).ok().map(|b| b.with_meta(OURS))
}

/// Scopes whose shortcuts must still work while the embedded browser page has X focus.
fn native_scope(scope: &str) -> bool { matches!(scope, GLOBAL | "AdosShell" | "BrowserPanel") }

/// Rebuild GPUI's keymap from the registry and the user's changes.
pub fn apply(cx: &mut App) {
    let s = state(cx);
    let specs: Vec<Spec> = s.entries.iter().map(|e| e.spec.clone()).collect();
    let chords = model::effective(&specs, &s.user);
    let actions: Vec<Box<dyn Action>> = s.entries.iter().map(|e| e.action.boxed_clone()).collect();
    let kept: Vec<KeyBinding> = cx.key_bindings().borrow().bindings()
        .filter(|b| b.meta() != Some(OURS)).cloned().collect();
    let mut ours = vec![];
    let mut native = vec![];
    for (index, spec) in specs.iter().enumerate() {
        for chord in &chords[index] {
            let unless: Vec<&str> = specs.iter().enumerate()
                .filter(|(other, o)| o.overrides.contains(&spec.id) && chords[*other].contains(chord) && o.scope != GLOBAL)
                .map(|(_, o)| o.scope.as_str()).collect();
            if let Some(b) = binding_unless(chord, &spec.scope, &unless, actions[index].boxed_clone(), cx) { ours.push(b); }
            if native_scope(&spec.scope) && !is_no_action(actions[index].as_ref()) {
                native.push((chord.clone(), index));
            }
        }
    }
    cx.clear_key_bindings();
    cx.bind_keys(kept);
    cx.bind_keys(ours);
    let s = cx.global_mut::<Shortcuts>();
    s.chords = chords;
    s.native = native;
    s.version += 1;
}

/// Load `keymap.toml`, apply it and watch it for changes. Call once, after every crate registered.
pub fn start(cx: &mut App) {
    state(cx).started = true;
    load(cx, true);
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(Duration::from_millis(500)).await;
        cx.update(|cx| {
            let stamp = std::fs::metadata(file_path()).and_then(|m| m.modified()).ok();
            if stamp != cx.global::<Shortcuts>().stamp { load(cx, false); }
        });
    }).detach();
}

fn load(cx: &mut App, first: bool) {
    let path = file_path();
    let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            state(cx).notices.push(Notice::Problem(format!("Could not read keymap.toml: {e}")));
            None
        }
    };
    let all: Vec<Spec> = defaults::DEFAULTS.iter().map(defaults::Def::spec).collect();
    let parsed = file::parse(text.as_deref().unwrap_or(""), &all);
    let s = state(cx);
    s.stamp = stamp;
    s.user = parsed.user;
    s.notices.extend(parsed.problems.into_iter().map(Notice::Problem));
    apply(cx);
    let count = conflicts(cx).len();
    if count > 0 && (first || text.is_some()) { state(cx).notices.push(Notice::Conflicts(count)); }
}

/// Save the user's changes to `keymap.toml` and apply them at once.
pub fn save(user: UserMap, cx: &mut App) -> Result<(), String> {
    let specs = specs(cx);
    let text = file::render(&user, &specs);
    let path = file_path();
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("Could not save keymap.toml: {e}"))?;
    let s = state(cx);
    s.user = user;
    s.stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    apply(cx);
    Ok(())
}

pub fn specs(cx: &App) -> Vec<Spec> {
    cx.try_global::<Shortcuts>().map(|s| s.entries.iter().map(|e| e.spec.clone()).collect()).unwrap_or_default()
}
pub fn user(cx: &App) -> UserMap { cx.try_global::<Shortcuts>().map(|s| s.user.clone()).unwrap_or_default() }
/// Effective chords per registry entry, in `specs` order.
pub fn chords(cx: &App) -> Vec<Vec<String>> {
    cx.try_global::<Shortcuts>().map(|s| model::effective(&specs(cx), &s.user)).unwrap_or_default()
}
pub fn conflicts(cx: &App) -> Vec<Clash> { model::conflicts(&specs(cx), &chords(cx)) }
pub fn version(cx: &App) -> u64 { cx.try_global::<Shortcuts>().map_or(0, |s| s.version) }

/// Live label for a registry id: `None` if no such shortcut, `Some("")` if it is unbound.
pub fn label_for(id: &str, cx: &App) -> Option<String> {
    let s = cx.try_global::<Shortcuts>()?;
    let index = s.entries.iter().position(|e| e.spec.id == id)?;
    let chords = s.chords.get(index).cloned().unwrap_or_else(|| s.entries[index].spec.defaults.clone());
    Some(chords.first().map(|c| label(c)).unwrap_or_default())
}
/// `"Title · Ctrl+K"`, or just the title when unbound.
pub fn tooltip(title: &str, id: &str, cx: &App) -> String {
    match label_for(id, cx) {
        Some(l) if !l.is_empty() => format!("{title} · {l}"),
        _ => title.to_string(),
    }
}

/// Chords the browser page grabs natively; the index is for `native_action`.
pub fn native_keys(cx: &App) -> Vec<String> {
    cx.try_global::<Shortcuts>().map(|s| s.native.iter().map(|(c, _)| c.clone()).collect()).unwrap_or_default()
}
pub fn native_action(index: usize, cx: &App) -> Option<Box<dyn Action>> {
    let s = cx.try_global::<Shortcuts>()?;
    let (_, entry) = s.native.get(index)?;
    s.entries.get(*entry).map(|e| e.action.boxed_clone())
}

/// Pending toasts for the shell. Reads first so observers do not loop on an empty queue.
pub fn take_notices(cx: &mut App) -> Vec<Notice> {
    if cx.try_global::<Shortcuts>().is_none_or(|s| s.notices.is_empty()) { return vec![]; }
    std::mem::take(&mut cx.global_mut::<Shortcuts>().notices)
}
