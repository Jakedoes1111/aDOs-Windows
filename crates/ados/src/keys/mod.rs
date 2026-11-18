//! Shortcuts editor: search, record a chord, resolve conflicts. Writes `keymap.toml`.
mod shell;
mod view;

use ados_core::shortcuts::{self as sc, Spec, UserMap};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::collections::HashMap;

pub enum KeysEvent { Close }

pub(crate) enum Mode {
    Idle,
    /// Listening for a chord for registry index `index`.
    Recording { index: usize, chord: Option<String>, error: Option<String> },
    /// The pressed chord is already used: Swap, Replace or Cancel.
    Conflict { index: usize, chord: String, other: usize },
}

pub struct KeysEditor {
    pub(crate) search: Entity<InputState>,
    pub(crate) focus: FocusHandle,
    pub(crate) rows: HashMap<String, FocusHandle>,
    pub(crate) swap_focus: FocusHandle,
    pub(crate) only_conflicts: bool,
    pub(crate) mode: Mode,
    pub(crate) error: Option<String>,
    interceptor: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<KeysEvent> for KeysEditor {}

/// A row as the view needs it.
pub(crate) struct Row {
    pub index: usize,
    pub spec: Spec,
    pub chords: Vec<String>,
    pub changed: bool,
    pub conflicts_with: Vec<usize>,
    pub overrides: Vec<usize>,
}

impl KeysEditor {
    pub fn new(only_conflicts: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search shortcuts"));
        let subscriptions = vec![
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) { cx.notify(); }
            }),
            cx.observe_global::<sc::Shortcuts>(|_, cx| cx.notify()),
        ];
        let focus = cx.focus_handle();
        // Open with the keyboard in the search box, so typing filters at once.
        search.update(cx, |input, cx| input.focus(window, cx));
        Self {
            search, focus, rows: HashMap::new(), swap_focus: cx.focus_handle(), only_conflicts,
            mode: Mode::Idle, error: None, interceptor: None, _subscriptions: subscriptions,
        }
    }

    pub(crate) fn rows(&self, cx: &App) -> Vec<Row> {
        let specs = sc::specs(cx);
        let chords = sc::chords(cx);
        let user = sc::user(cx);
        let clashes = sc::clashes(&specs, &chords);
        let order = |id: &str| sc::defaults::DEFAULTS.iter().position(|d| d.id == id).unwrap_or(usize::MAX);
        let mut rows: Vec<Row> = specs.iter().enumerate().map(|(index, spec)| {
            let partners = |intended: bool| clashes.iter().filter(|c| c.intended == intended)
                .filter_map(|c| if c.a == index { Some(c.b) } else if c.b == index { Some(c.a) } else { None })
                .collect::<Vec<_>>();
            Row {
                index, spec: spec.clone(), chords: chords[index].clone(),
                changed: user.contains_key(&spec.id),
                conflicts_with: partners(false),
                overrides: partners(true).into_iter().filter(|o| spec.overrides.contains(&specs[*o].id)).collect(),
            }
        }).collect();
        rows.sort_by_key(|r| order(&r.spec.id));
        let query = self.search.read(cx).value().to_lowercase();
        rows.retain(|r| {
            (!self.only_conflicts || !r.conflicts_with.is_empty())
                && (query.is_empty() || [r.spec.title.to_lowercase(), r.spec.id.clone(), sc::scope_title(&r.spec.scope).to_lowercase(),
                    r.chords.iter().map(|c| sc::label(c).to_lowercase()).collect::<Vec<_>>().join(" ")]
                    .iter().any(|text| text.contains(query.trim())))
        });
        rows
    }

    pub(crate) fn row_focus(&mut self, id: &str, cx: &mut Context<Self>) -> FocusHandle {
        self.rows.entry(id.to_string()).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

    pub(crate) fn record(&mut self, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.mode = Mode::Recording { index, chord: None, error: None };
        let editor = cx.entity().downgrade();
        // Runs before key bindings: Ctrl+K while recording is recorded, not run.
        self.interceptor = Some(cx.intercept_keystrokes(move |event, window, cx| {
            let keystroke = event.keystroke.clone();
            if matches!(keystroke.key.as_str(), "shift" | "control" | "alt" | "platform" | "function") { return; }
            if editor.update(cx, |e, cx| e.key(&keystroke, window, cx)).is_ok() { cx.stop_propagation(); }
        }));
        cx.notify();
    }

    fn key(&mut self, keystroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        let Mode::Recording { index, chord, .. } = &self.mode else { return };
        let (index, chord) = (*index, chord.clone());
        let m = keystroke.modifiers;
        let plain = !m.control && !m.alt && !m.shift && !m.platform;
        if plain && keystroke.key == "escape" { return self.cancel(window, cx); }
        if plain && keystroke.key == "enter" {
            if let Some(chord) = chord { self.commit(index, &chord, None, window, cx); }
            return;
        }
        let mut text = String::new();
        for (on, name) in [(m.control, "ctrl-"), (m.alt, "alt-"), (m.shift, "shift-"), (m.platform, "super-")] {
            if on { text.push_str(name); }
        }
        text.push_str(&keystroke.key);
        match sc::normalize(&text) {
            Ok(chord) => {
                let specs = sc::specs(cx);
                match sc::conflict_for(&specs, &sc::chords(cx), index, &chord) {
                    Some(other) => {
                        self.interceptor = None;
                        self.mode = Mode::Conflict { index, chord, other };
                        let swap = self.swap_focus.clone();
                        window.defer(cx, move |window, cx| window.focus(&swap, cx));
                    }
                    None => self.mode = Mode::Recording { index, chord: Some(chord), error: None },
                }
            }
            Err(error) => self.mode = Mode::Recording { index, chord, error: Some(error) },
        }
        cx.notify();
    }

    /// `resolve`: `Some(true)` swap, `Some(false)` replace, `None` plain assignment.
    pub(crate) fn commit(&mut self, index: usize, chord: &str, resolve: Option<(usize, bool)>, window: &mut Window, cx: &mut Context<Self>) {
        let specs = sc::specs(cx);
        let mut user = sc::user(cx);
        match resolve {
            Some((other, true)) => sc::swap(&mut user, &specs, index, other, chord),
            Some((other, false)) => sc::replace(&mut user, &specs, index, other, chord),
            None => sc::assign(&mut user, &specs, index, chord),
        }
        self.save(user, cx);
        self.finish(index, window, cx);
    }

    pub(crate) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = match self.mode { Mode::Recording { index, .. } | Mode::Conflict { index, .. } => index, Mode::Idle => return };
        self.finish(index, window, cx);
    }

    fn finish(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.interceptor = None;
        self.mode = Mode::Idle;
        if let Some(id) = sc::specs(cx).get(index).map(|s| s.id.clone()) {
            let focus = self.row_focus(&id, cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn reset(&mut self, index: usize, cx: &mut Context<Self>) {
        let specs = sc::specs(cx);
        let Some(spec) = specs.get(index) else { return };
        let mut user = sc::user(cx);
        sc::set(&mut user, spec, spec.defaults.clone());
        self.save(user, cx);
    }

    pub(crate) fn reset_all(&mut self, cx: &mut Context<Self>) { self.save(UserMap::new(), cx); }

    fn save(&mut self, user: UserMap, cx: &mut Context<Self>) {
        self.error = sc::save(user, cx).err();
        if sc::conflicts(cx).is_empty() { self.only_conflicts = false; }
        cx.notify();
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.interceptor = None;
        cx.emit(KeysEvent::Close);
    }

    /// Up/Down between rows, like a list.
    pub(crate) fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.rows(cx).into_iter().map(|r| r.spec.id).collect();
        if ids.is_empty() { return; }
        let current = ids.iter().position(|id| self.rows.get(id).is_some_and(|f| f.is_focused(window)));
        let next = match current {
            Some(i) => (i as isize + delta).clamp(0, ids.len() as isize - 1) as usize,
            None => 0,
        };
        let focus = self.row_focus(&ids[next], cx);
        window.focus(&focus, cx);
        cx.notify();
    }
}

impl Focusable for KeysEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle { self.search.focus_handle(cx) }
}
