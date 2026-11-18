//! `ScriptPanel`: one script folder `ADOS_HOME/ui/<name>/` shown as a dock panel.
//!
//! Reload is event driven (inotify through `notify`), not gpui-shell's 250 ms poll, so an
//! idle panel does no work. A reload mounts a fresh view from disk; if the new code does
//! not load, the old view stays and a banner says why. In-memory script state is lost on
//! reload by design, so scripts keep what matters through `ados.storage_*`.
use crate::gate::{Ask, Decisions, Manifest};
use crate::host::{self, Storage};
use ados_core::hub::ContextHub;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::{button::{Button, ButtonVariants}, h_flex, v_flex, ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use gpui_shell::{policy::{self, Policy}, ScriptView, ShellRuntime};
use notify::Watcher as _;
use std::{path::PathBuf, rc::Rc, time::{Duration, Instant}};

/// One VM for every script panel; each panel still gets its own policy.
struct Runtime(Rc<ShellRuntime>);
impl Global for Runtime {}

fn runtime(cx: &mut App) -> Result<Rc<ShellRuntime>, String> {
    if let Some(r) = cx.try_global::<Runtime>() { return Ok(r.0.clone()); }
    let r = ShellRuntime::new_isolated().map_err(|e| format!("The script engine did not start: {e:#}"))?;
    cx.set_global(Runtime(r.clone()));
    Ok(r)
}

pub struct ScriptPanel {
    name: String,
    dir: PathBuf,
    title: SharedString,
    /// A bare `ScriptView`, not a `ShellRoot`: the root runs a 50 ms toast clock forever,
    /// and its toasts and dialogs only work as the window root anyway.
    root: Option<Entity<ScriptView>>,
    /// Load or reload failure, shown above the last good view.
    error: Option<String>,
    question: Option<Ask>,
    denied: Vec<Ask>,
    /// Milliseconds from the file event to the new view being mounted.
    pub last_reload_ms: Option<u128>,
    focus_handle: FocusHandle,
    _watcher: Option<notify::RecommendedWatcher>,
    _tasks: Vec<Task<()>>,
    _subs: Vec<Subscription>,
}

impl ScriptPanel {
    pub fn dir_for(name: &str) -> PathBuf { ados_core::paths::home().join("ui").join(name) }

    pub fn view(name: &str, window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(name, window, cx))
    }

    fn new(name: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dir = Self::dir_for(name);
        let mut this = Self {
            name: name.to_string(), dir, title: name.to_string().into(), root: None, error: None,
            question: None, denied: Vec::new(), last_reload_ms: None, focus_handle: cx.focus_handle(),
            _watcher: None, _tasks: Vec::new(), _subs: Vec::new(),
        };
        // Context changed somewhere in aDOs: let the script render it again.
        this._subs.push(cx.observe_global::<ContextHub>(|this, cx| {
            if let Some(view) = this.root.clone() { view.update(cx, |v, cx| v.refresh(cx)); }
        }));
        this.watch(window, cx);
        this.load(None, window, cx);
        this
    }

    fn watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (tx, rx) = async_channel::unbounded::<Instant>();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            let source = event.paths.iter().any(|p| {
                matches!(p.extension().and_then(|e| e.to_str()), Some("js" | "mjs") | Some("json"))
                    && !p.ends_with("state.json") && !p.ends_with("jsconfig.json")
            });
            if source && (event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove()) {
                let _ = tx.try_send(Instant::now());
            }
        });
        let mut watcher = match watcher {
            Ok(w) => w,
            Err(e) => { self.error = Some(format!("Changes to this panel will not show until restart: {e}.")); return; }
        };
        if let Err(e) = std::fs::create_dir_all(&self.dir)
            .map_err(|e| e.to_string())
            .and_then(|_| watcher.watch(&self.dir, notify::RecursiveMode::NonRecursive).map_err(|e| e.to_string()))
        {
            self.error = Some(format!("Changes to this panel will not show until restart: {e}."));
            return;
        }
        self._watcher = Some(watcher);
        self._tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                // An editor writes in bursts; wait until it settles.
                cx.background_executor().timer(Duration::from_millis(60)).await;
                while rx.try_recv().is_ok() {}
                if this.update_in(cx, |this, window, cx| this.load(Some(first), window, cx)).is_err() { break; }
            }
        }));
    }

    /// Reads the manifest and decisions, then mounts `main.js` under the granted policy.
    fn load(&mut self, started: Option<Instant>, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.try_load(window, cx);
        match result {
            Ok(root) => {
                self.root = Some(root);
                self.error = None;
                if let Some(t) = started {
                    let ms = t.elapsed().as_millis();
                    self.last_reload_ms = Some(ms);
                    eprintln!("ados-script: {} reloaded {ms} ms after the file changed", self.name);
                }
            }
            Err(e) => {
                eprintln!("ados-script: {} did not load: {e}", self.name);
                self.error = Some(e);
            }
        }
        cx.notify();
    }

    fn try_load(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<Entity<ScriptView>, String> {
        let manifest = Manifest::read(&self.dir)?;
        if let Some(t) = &manifest.title { self.title = t.clone().into(); }
        let asks = manifest.asks();
        let decisions = Decisions::load(&Decisions::file());
        self.question = decisions.pending(&self.name, &asks);
        self.denied = decisions.denied(&self.name, &asks);
        let grants = decisions.grants(&self.name, &asks);

        if !self.dir.join("main.js").is_file() {
            return Err(format!("This panel has no main.js in {}.", self.dir.display()));
        }
        let storage = Rc::new(Storage::open(self.dir.join("state.json"))?);
        let module = host::module(storage, grants.actions, window.window_handle());
        let policy = Policy::new()
            .with_application(format!("ados-{}", self.name))
            .with_capabilities(grants.capabilities())
            .with_host_module(module)
            .map_err(|e| format!("The ados module was refused: {e}"))?;
        let runtime = runtime(cx)?;
        // Loading and mounting read the default policy, so set it only for that long.
        policy::set_default(policy);
        let root = runtime.load_application(&self.dir, "main.js")
            .and_then(|app| runtime.mount_application(&app, window, cx));
        policy::set_default(Policy::new());
        let root = root.map_err(|e| first_line(&format!("{e:#}")))?;
        // The view may construct fine but fail its first render; treat that as a failed load.
        let failed = root.read(cx).build_error().map(str::to_owned);
        match failed {
            Some(e) if self.root.is_some() => Err(first_line(&e)),
            _ => Ok(root),
        }
    }

    fn answer(&mut self, allowed: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.question.take() else { return };
        let path = Decisions::file();
        let mut decisions = Decisions::load(&path);
        decisions.set(&self.name, &ask, allowed);
        if let Err(e) = decisions.save(&path) { self.error = Some(e); }
        // A new grant needs a new policy, so the script is mounted again.
        self.load(None, window, cx);
    }
}

fn first_line(e: &str) -> String {
    let line = e.lines().find(|l| !l.trim().is_empty()).unwrap_or("The script failed to load.");
    format!("The last change did not load, so the previous version is still showing: {line}")
}

impl Render for ScriptPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let banner = |id: &'static str, text: String, color: Hsla| {
            h_flex().id(id).w_full().min_h(px(40.)).px_3().py_2().gap_2()
                .border_b_1().border_color(theme.border).bg(theme.sidebar)
                .child(div().flex_shrink_0().w(px(3.)).h(px(20.)).rounded(px(2.)).bg(color))
                .child(div().flex_1().min_w_0().text_sm().text_color(theme.foreground).child(text))
        };
        let red: Hsla = rgb(ados_core::brand::RED).into();
        let blue: Hsla = rgb(ados_core::brand::BLUE).into();
        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(theme.background)
            .when_some(self.question.clone(), |el, ask| {
                el.child(banner("script-question", format!("{} wants to {}. Allow it?", self.title, ask.question()), blue)
                    .child(Button::new("script-deny").label("Deny").ghost().small().h(px(32.))
                        .on_click(cx.listener(|this, _, window, cx| this.answer(false, window, cx))))
                    .child(Button::new("script-allow").label("Allow").primary().small().h(px(32.))
                        .on_click(cx.listener(|this, _, window, cx| this.answer(true, window, cx)))))
            })
            .when_some(self.error.clone(), |el, e| el.child(banner("script-error", e, red)))
            .when(!self.denied.is_empty(), |el| {
                let list = self.denied.iter().map(|a| a.question()).collect::<Vec<_>>().join("; ");
                el.child(div().px_3().py_1().text_xs().text_color(theme.muted_foreground)
                    .child(format!("Blocked: {list}.")))
            })
            .child(div().flex_1().min_h_0().children(self.root.clone()))
    }
}

impl EventEmitter<PanelEvent> for ScriptPanel {}
impl Focusable for ScriptPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() }
}
impl Panel for ScriptPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { self.title.clone() }
    fn inner_padding(&self, _: &App) -> bool { false }
}
impl BasePanel for ScriptPanel {
    fn panel_name(&self) -> &'static str { "script" }
}
