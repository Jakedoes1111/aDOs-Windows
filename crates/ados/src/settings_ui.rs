//! The Settings screen: one searchable list of every adjustable thing, grouped by module.
use crate::shell::Shell;
use ados_core::{brand, settings as store};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex, input::{Input, InputEvent, InputState}, switch::Switch, v_flex, ActiveTheme, Icon, IconName,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::path::PathBuf;

/// The theme's base font size (px) at 100%; see `theme.rs`.
const BASE_FONT: f32 = 20.;
const BASE_MONO: f32 = 13.;

pub enum SettingsEvent { Close, Do(Job) }
/// Things a row asks the shell to do after Settings has closed.
#[derive(Clone)]
pub enum Job { Shortcuts, SaveLayout, SaveLayoutAs, Revert, ResetLayouts, Module(&'static str) }

pub struct SettingsView {
    search: Entity<InputState>,
    /// A host typed for the Browser's "Open in Chromium" list, and why it was not added.
    site_input: Entity<InputState>,
    site_error: Option<String>,
    focus: FocusHandle,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<SettingsEvent> for SettingsView {}

struct Row { group: &'static str, id: &'static str, label: &'static str, desc: String }

/// Scale a pixel size by the interface zoom (the chrome drawn in pixels, not rems).
pub fn zoomed(value: f32, cx: &App) -> Pixels { px(value * store::get(cx).zoom_percent as f32 / 100.) }

/// Make the theme's font size follow the saved zoom. Every rem in the window follows it.
pub fn apply_zoom(cx: &mut App) {
    let scale = store::get(cx).zoom_percent as f32 / 100.;
    gpui_kit::component::Theme::update(cx, |t| {
        t.font_size = px(BASE_FONT * scale);
        t.mono_font_size = px(BASE_MONO * scale);
    });
    cx.refresh_windows();
}

/// Terminal font size lives in `state/terminal-font.txt` (the Terminal reads it when it opens).
fn terminal_font_path() -> PathBuf { ados_core::paths::home().join("state/terminal-font.txt") }
fn terminal_font() -> f32 {
    std::fs::read_to_string(terminal_font_path()).ok().and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite()).unwrap_or(14.).clamp(8., 32.)
}
fn set_terminal_font(size: f32) -> Result<(), String> {
    let path = terminal_font_path();
    if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
    std::fs::write(&path, size.to_string()).map_err(|e| format!("Could not save the terminal font size: {e}"))
}

fn open_path(path: PathBuf, cx: &mut App) {
    if !path.exists() {
        if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
        if path.extension().is_some() { let _ = std::fs::write(&path, ""); } else { let _ = std::fs::create_dir_all(&path); }
    }
    cx.open_url(&format!("file://{}", path.display()));
}

impl Shell {
    pub(crate) fn zoom_ui(&mut self, dir: i32, cx: &mut Context<Self>) {
        let now = store::get(cx).zoom_percent;
        self.set_zoom(store::zoom_step(now, dir), cx);
    }
    pub(crate) fn set_zoom(&mut self, percent: u32, cx: &mut Context<Self>) {
        self.error = store::update(cx, |s| s.zoom_percent = percent).err();
        apply_zoom(cx);
        if self.error.is_none() { self.error = Some(format!("Interface zoom {percent}%")); }
        cx.notify();
    }
    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = &self.settings {
            window.focus(&view.read(cx).search.focus_handle(cx), cx);
            return;
        }
        // The view focuses its search box as it is built, before the overlay guard opens: note
        // where the keyboard is now, so Esc returns it there (F5, `overlay_focus.rs`). A chord
        // that opened Settings never reached `note_key` (the binding consumed it).
        if self.overlay_before.is_none() { self.note_key(window, cx); }
        let view = cx.new(|cx| SettingsView::new(window, cx));
        self.settings_subscription = Some(cx.subscribe_in(&view, window, |s, _, event, w, cx| match event {
            SettingsEvent::Close => s.close_settings(w, cx),
            SettingsEvent::Do(job) => {
                s.close_settings(w, cx);
                s.run_job(job.clone(), w, cx);
            }
        }));
        ados_core::overlay::OverlayGuard::open(cx);
        self.settings = Some(view);
        cx.notify();
    }
    pub(crate) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.take().is_some() { ados_core::overlay::OverlayGuard::close(cx); }
        self.settings_subscription = None;
        window.focus(&self.focus, cx);
        window.refresh();
        cx.notify();
    }
    fn run_job(&mut self, job: Job, window: &mut Window, cx: &mut Context<Self>) {
        match job {
            Job::Shortcuts => self.open_keys(false, window, cx),
            Job::SaveLayout => { self.save_layout(None, cx); }
            Job::SaveLayoutAs => self.open_prompt(window, cx),
            Job::Revert => self.revert_layout(window, cx),
            Job::ResetLayouts => self.confirm_reset(window, cx),
            Job::Module(id) => self.focus_module(id, window, cx),
        }
    }
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings"));
        let site_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add a site, e.g. youtube.com"));
        let subscriptions = vec![
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) { cx.notify(); }
            }),
            cx.subscribe_in(&site_input, window, |v, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => v.add_site(window, cx),
                InputEvent::Change if v.site_error.is_some() => { v.site_error = None; cx.notify(); }
                _ => {}
            }),
        ];
        search.update(cx, |input, cx| input.focus(window, cx));
        Self { search, site_input, site_error: None, focus: cx.focus_handle(), error: None, _subscriptions: subscriptions }
    }

    /// Enter in the site box: a cleaned host joins the list; anything else is a sentence, nothing saved.
    fn add_site(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.site_input.read(cx).value().to_string();
        match store::clean_site(&typed) {
            Ok(host) if store::get(cx).chromium_sites.contains(&host) => {
                self.site_error = Some(format!("{host} is already on the list."));
            }
            Ok(host) => {
                self.site_error = store::update(cx, |s| s.chromium_sites.push(host)).err();
                self.site_input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            Err(sentence) => self.site_error = Some(sentence),
        }
        cx.notify();
    }
    fn remove_site(&mut self, host: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.site_error = store::update(cx, |s| s.chromium_sites.retain(|h| h != host)).err();
        // The keyboard stays in the list: the box, where the next entry is typed.
        self.site_input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    /// The "Open in Chromium" list: the box, then one row per site with its remove button.
    fn sites(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let sites = store::get(cx).chromium_sites;
        let mut list = v_flex().gap_1().pt_2()
            .child(div().relative().child(Input::new(&self.site_input)).child(ados_core::hints::mark_overlay()));
        if let Some(e) = &self.site_error {
            list = list.child(div().text_sm().text_color(rgb(brand::RED)).child(e.clone()));
        }
        if sites.is_empty() {
            list = list.child(div().py_1().text_sm().text_color(muted).child("No sites. Pages open in Chromium only when Servo cannot load them."));
        }
        for (i, host) in sites.into_iter().enumerate() {
            let tip = format!("Remove {host}");
            let remove = host.clone();
            list = list.child(h_flex().h(zoomed(36., cx)).pl_2().gap_2().items_center().rounded(px(8.))
                .border_1().border_color(cx.theme().border)
                .child(div().flex_1().min_w_0().truncate().text_sm().child(host))
                .child(ados_core::hints::marked_in(ados_core::hints::Layer::Overlay,
                    Button::new(SharedString::from(format!("remove-site-{i}"))).icon(IconName::Close).ghost().size(px(32.))
                        .tooltip(tip).on_click(cx.listener(move |v, _, w, cx| v.remove_site(&remove, w, cx))))));
        }
        list.into_any_element()
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let s = store::get(cx);
        let r = |group, id, label, desc: &str| Row { group, id, label, desc: desc.into() };
        let all = vec![
            r("Appearance", "zoom", "Interface zoom",
                "Scales all text and controls together. Ctrl+= / Ctrl+- / Ctrl+0 change it everywhere except inside a Browser page, which keeps its own page zoom. The Browser page and the Terminal's text keep their own size; inside the Terminal these chords also change that text size."),
            r("Layouts", "startup", "Start-up layout",
                "Which layout opens when aDOs starts. “Last used” reopens how you left it."),
            r("Layouts", "save", "Save layout", "Store the current arrangement under this layout's name."),
            r("Layouts", "saveas", "Save as new layout", "Store the current arrangement under a new name."),
            r("Layouts", "revert", "Revert to saved", "Put the current layout back to how it was last saved."),
            r("Layouts", "resetlayouts", "Reset layouts to defaults", "Rebuild Research, Code and Focus. Your named layouts are kept."),
            r("Shortcuts", "shortcuts", "Keyboard shortcuts", "Change any chord, resolve conflicts. Saved in keymap.toml."),
            r("Terminal", "termfont", "Terminal font size",
                &format!("Text size in the Terminal, {}–{}. Ctrl+= / Ctrl+- / Ctrl+0 inside the Terminal change it live; this row sets it for the next time the Terminal opens.", 8, 32)),
            r("Browser", "browser", "Browser engine and page",
                "The engine (Servo or Chromium) is switched with the button on the Browser's toolbar and remembered. Open the Browser to change it."),
            r("Browser", "chromiumsites", "Open in Chromium",
                "While the engine is Servo, these sites (and their subdomains) open in Chromium. Type a site and press Enter to add it."),
            r("Browser", "chromiumfail", "Try Chromium when Servo cannot load a page",
                "When Servo shows its own “could not load” page, the Browser switches to Chromium at the same address."),
            r("Assistant", "assistant", "Assistant backend",
                "Pick the agent the assistant talks to in the assistant card's header. Remembered between launches."),
            r("Assistant", "backends", "Agent backends file",
                &format!("Which agents aDOs can start, and how long an unused one keeps running (idle_stop_secs). Edited as a file: {}. Read again each time an agent starts.", ados_agents::config::backends_file().display())),
            r("Sessions", "sessions", "Sessions backend",
                "Which agent's sessions are listed is chosen at the top of the Sessions module and remembered."),
            r("Notes", "notes", "Notes folder",
                &format!("Where your notes are stored, one Markdown file each: {}. The location is fixed.", ados_core::paths::notes_dir().display())),
            r("Capture", "inbox", "Capture inbox folder",
                &format!("Where quick captures are stored: {}. The location is fixed.", ados_core::paths::inbox_dir().display())),
            r("Calendar", "calendar", "Calendar subscriptions",
                &format!("Read-only .ics calendars to show, listed in {}.", ados_core::paths::home().join("calendar/subscriptions.toml").display())),
            r("Data", "data", "Data folder", &format!("Everything aDOs keeps: {}.", ados_core::paths::home().display())),
            r("Data", "about", "About aDOs", &format!("Version {}.", env!("CARGO_PKG_VERSION"))),
        ];
        let _ = s;
        let query = self.search.read(cx).value().trim().to_lowercase();
        // Rows for modules left out of this build would do nothing; the Browser rows are about Servo and Chromium (Linux).
        let built = |row: &Row| (cfg!(feature = "terminal") || row.group != "Terminal") && ((cfg!(feature = "browser") && cfg!(unix)) || row.group != "Browser");
        all.into_iter().filter(built).filter(|row| query.is_empty()
            || [row.group, row.label, row.id].iter().any(|t| t.to_lowercase().contains(&query))
            || row.desc.to_lowercase().contains(&query)).collect()
    }

    fn close(&mut self, cx: &mut Context<Self>) { cx.emit(SettingsEvent::Close); }
    fn job(&mut self, job: Job, cx: &mut Context<Self>) { cx.emit(SettingsEvent::Do(job)); }

    fn btn(&self, id: &'static str, label: &str, tip: &str, cx: &mut Context<Self>, run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> AnyElement {
        ados_core::hints::marked_in(ados_core::hints::Layer::Overlay, Button::new(id).label(label.to_string()).outline()
            .tooltip(tip.to_string()).on_click(cx.listener(move |s, _, w, cx| run(s, w, cx)))).into_any_element()
    }
    fn icon_btn(&self, id: &'static str, icon: IconName, tip: &str, cx: &mut Context<Self>, run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> AnyElement {
        ados_core::hints::marked_in(ados_core::hints::Layer::Overlay, Button::new(id).icon(icon).outline()
            .tooltip(tip.to_string()).on_click(cx.listener(move |s, _, w, cx| run(s, w, cx)))).into_any_element()
    }
    fn value(&self, text: String) -> AnyElement {
        div().min_w(px(64.)).flex().justify_center().text_sm().child(text).into_any_element()
    }

    fn control(&mut self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let s = store::get(cx);
        let row = |items: Vec<AnyElement>| h_flex().gap_2().items_center().children(items).into_any_element();
        match id {
            "zoom" => row(vec![
                self.icon_btn("zoom-out", IconName::Minus, "Smaller · Ctrl+-", cx, |_, _, cx| { Self::zoom(-1, cx) }),
                self.value(format!("{}%", s.zoom_percent)),
                self.icon_btn("zoom-in", IconName::Plus, "Larger · Ctrl+=", cx, |_, _, cx| { Self::zoom(1, cx) }),
                self.icon_btn("zoom-reset", IconName::Undo2, "Reset to 100% · Ctrl+0", cx, |_, _, cx| { Self::zoom(0, cx) }),
            ]),
            "startup" => {
                let now = s.startup_layout.clone();
                let label = now.clone().unwrap_or_else(|| "Last used".into());
                row(vec![self.btn("startup-cycle", &label, "Next choice", cx, move |v, _, cx| {
                    let mut choices: Vec<Option<String>> = vec![None];
                    choices.extend(crate::storage::names().into_iter().map(Some));
                    let at = choices.iter().position(|c| *c == now).unwrap_or(0);
                    let next = choices[(at + 1) % choices.len()].clone();
                    v.error = store::update(cx, |s| s.startup_layout = next).err();
                    cx.notify();
                })])
            }
            "save" => row(vec![self.btn("save", "Save", "Save layout", cx, |s, _, cx| s.job(Job::SaveLayout, cx))]),
            "saveas" => row(vec![self.btn("saveas", "Save as…", "Save as new layout", cx, |s, _, cx| s.job(Job::SaveLayoutAs, cx))]),
            "revert" => row(vec![self.btn("revert", "Revert", "Revert to saved", cx, |s, _, cx| s.job(Job::Revert, cx))]),
            "resetlayouts" => row(vec![self.btn("resetlayouts", "Reset…", "Reset layouts to defaults", cx, |s, _, cx| s.job(Job::ResetLayouts, cx))]),
            "shortcuts" => row(vec![self.btn("shortcuts", "Edit…", "Open the shortcuts editor", cx, |s, _, cx| s.job(Job::Shortcuts, cx))]),
            "termfont" => {
                let size = terminal_font();
                row(vec![
                    self.icon_btn("font-out", IconName::Minus, "Smaller", cx, move |v, _, cx| Self::font(v, size - 1., cx)),
                    self.value(format!("{size:.0} px")),
                    self.icon_btn("font-in", IconName::Plus, "Larger", cx, move |v, _, cx| Self::font(v, size + 1., cx)),
                    self.icon_btn("font-reset", IconName::Undo2, "Reset to 14 px", cx, |v, _, cx| Self::font(v, 14., cx)),
                ])
            }
            "chromiumfail" => {
                let view = cx.entity().downgrade();
                row(vec![ados_core::hints::marked_in(ados_core::hints::Layer::Overlay,
                    Switch::new("chromium-on-failure").checked(s.chromium_on_failure)
                        .tooltip("Try Chromium when Servo cannot load a page")
                        .on_change(move |on, _, cx| {
                            let on = *on;
                            let error = store::update(cx, |s| s.chromium_on_failure = on).err();
                            let _ = view.update(cx, |v, cx| { v.error = error; cx.notify(); });
                        })).into_any_element()])
            }
            "browser" => row(vec![self.btn("open-browser", "Open Browser", "Go to the Browser", cx, |s, _, cx| s.job(Job::Module(ados_core::ids::BROWSER), cx))]),
            "assistant" => row(vec![self.btn("open-assistant", "Open assistant", "Open the assistant card", cx, |s, w, cx| {
                s.close(cx);
                w.dispatch_action(Box::new(crate::actions::ToggleAssistant), cx);
            })]),
            "backends" => row(vec![self.btn("open-backends", "Open file", "Open backends.toml", cx, |_, _, cx| {
                let _ = ados_agents::config::load();
                open_path(ados_agents::config::backends_file(), cx);
            })]),
            "sessions" => row(vec![self.btn("open-sessions", "Open Sessions", "Go to Sessions", cx, |s, _, cx| s.job(Job::Module(ados_core::ids::SESSIONS), cx))]),
            "notes" => row(vec![self.btn("open-notes", "Open folder", "Show the notes folder", cx, |_, _, cx| open_path(ados_core::paths::notes_dir(), cx))]),
            "inbox" => row(vec![self.btn("open-inbox", "Open folder", "Show the capture folder", cx, |_, _, cx| open_path(ados_core::paths::inbox_dir(), cx))]),
            "calendar" => row(vec![self.btn("open-subs", "Open file", "Open subscriptions.toml", cx, |_, _, cx| open_path(ados_core::paths::home().join("calendar/subscriptions.toml"), cx))]),
            "data" => row(vec![self.btn("open-data", "Open folder", "Show the data folder", cx, |_, _, cx| open_path(ados_core::paths::home(), cx))]),
            _ => row(vec![]),
        }
    }

    fn zoom(dir: i32, cx: &mut Context<Self>) {
        // Cannot reach the shell from here: change the store and the theme directly.
        let now = store::get(cx).zoom_percent;
        let next = store::zoom_step(now, dir);
        let _ = store::update(cx, |s| s.zoom_percent = next);
        apply_zoom(cx);
    }
    fn font(&mut self, size: f32, cx: &mut Context<Self>) {
        self.error = set_terminal_font(size.clamp(8., 32.)).err();
        cx.notify();
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        let muted = cx.theme().muted_foreground;
        let mut list = v_flex().gap_1();
        let mut group = "";
        for row in &rows {
            if row.group != group {
                list = list.child(div().pt_3().pb_1().px_3().text_xs().text_color(muted).child(row.group));
                group = row.group;
            }
            let control = self.control(row.id, cx);
            // The site list is too wide for the right-hand column: it sits under its description.
            let below = (row.id == "chromiumsites").then(|| self.sites(cx));
            list = list.child(
                h_flex().id(SharedString::from(format!("setting-{}", row.id)))
                    .min_h(zoomed(44., cx)).px_3().py_2().gap_4().items_center().rounded(px(8.))
                    .child(v_flex().flex_1().min_w_0().gap_1()
                        .child(div().text_sm().child(row.label))
                        .child(div().text_xs().text_color(muted).child(row.desc.clone()))
                        .children(below))
                    .child(div().flex_shrink_0().child(control)),
            );
        }
        if rows.is_empty() {
            list = list.child(div().p_6().text_sm().text_color(muted).child("No settings match."));
        }
        let viewport = window.viewport_size();
        let scale = store::get(cx).zoom_percent as f32 / 100.;
        div()
            .absolute().inset_0().flex().items_center().justify_center()
            .bg(cx.theme().background.opacity(0.9))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                v_flex()
                    .id("settings")
                    .key_context("SettingsScreen")
                    .track_focus(&self.focus)
                    .w((viewport.width - px(48.)).min(px(760. * scale)))
                    .h((viewport.height - px(48.)).min(px(720. * scale)))
                    .rounded(px(10.)).border_1().border_color(cx.theme().border)
                    .bg(cx.theme().popover).shadow_md()
                    .capture_key_down(cx.listener(|v, ev: &KeyDownEvent, w, cx| {
                        if ev.keystroke.modifiers.modified() { return; }
                        match ev.keystroke.key.as_str() {
                            "escape" => { v.close(cx); cx.stop_propagation(); }
                            "down" if v.search.focus_handle(cx).is_focused(w) => { w.focus_next(cx); cx.stop_propagation(); }
                            _ => {}
                        }
                    }))
                    .child(h_flex().h(zoomed(56., cx)).px_4().gap_2().items_center().flex_shrink_0()
                        .border_b_1().border_color(cx.theme().border)
                        .child(Icon::new(IconName::Settings).text_color(muted))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Settings"))
                        .child(div().flex_1())
                        .child(ados_core::hints::marked_in(ados_core::hints::Layer::Overlay,
                            Button::new("close-settings").icon(IconName::Close).ghost().tooltip("Close · Esc")
                                .on_click(cx.listener(|v, _, _, cx| v.close(cx))))))
                    .child(div().px_4().pt_3().flex_shrink_0().relative()
                        .child(Input::new(&self.search).prefix(Icon::new(IconName::Search)))
                        .child(ados_core::hints::mark_overlay()))
                    .when_some(self.error.clone(), |el, e| el.child(
                        div().px_4().pt_2().text_sm().text_color(rgb(brand::RED)).child(e)))
                    .child(div().id("settings-list").flex_1().min_h_0().overflow_y_scroll().px_2().pb_2().child(list))
                    .child(h_flex().px_4().h(zoomed(36., cx)).items_center().flex_shrink_0()
                        .border_t_1().border_color(cx.theme().border).text_xs().text_color(muted)
                        .child("Changes save at once · Tab moves · Esc closes")),
            )
    }
}

actions!(ados, [OpenSettings, ZoomIn, ZoomOut, ZoomReset]);

/// Chords and palette entries for Settings and the zoom. `problem`: a broken settings.json, said once.
pub fn bind(cx: &mut App, problem: Option<String>) {
    use ados_core::actions::{ActionRegistry, PaletteAction};
    use ados_core::shortcuts::bind;
    bind(cx, "shell.settings", OpenSettings);
    bind(cx, "shell.ui_zoom_in", ZoomIn);
    bind(cx, "shell.ui_zoom_out", ZoomOut);
    bind(cx, "shell.ui_zoom_reset", ZoomReset);
    type Factory = fn() -> Box<dyn Action>;
    for (id, title, keywords, action) in [
        ("shell.settings", "Settings", "settings preferences options customise zoom", (|| Box::new(OpenSettings)) as Factory),
        ("shell.ui_zoom_in", "Zoom interface in", "settings zoom larger bigger text", (|| Box::new(ZoomIn)) as Factory),
        ("shell.ui_zoom_out", "Zoom interface out", "settings zoom smaller text", (|| Box::new(ZoomOut)) as Factory),
        ("shell.ui_zoom_reset", "Reset interface zoom", "settings zoom reset 100", (|| Box::new(ZoomReset)) as Factory),
    ] {
        ActionRegistry::register(PaletteAction {
            id: id.into(), title: title.into(), keywords: keywords.into(), shortcut: None,
            run: std::rc::Rc::new(move |window, cx| window.dispatch_action(action(), cx)),
        }, cx);
    }
    if let Some(text) = problem { cx.set_global(StartupProblem(text)); }
}
pub struct StartupProblem(pub String);
impl Global for StartupProblem {}
