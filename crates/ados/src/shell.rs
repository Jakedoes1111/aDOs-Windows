use crate::{
    actions,
    dock_skin::{PanelCommand, ShellSkin},
    modules,
    layouts, storage,
};
use gpui_kit::base::dock::{self as base, DockArea, DockAreaState, DockEvent, PaneRef, PanelView};
use gpui_kit::component::{
    WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    menu::DropdownMenu,
    v_flex, ActiveTheme, IconName,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{collections::HashSet, sync::Arc};

pub struct Shell {
    pub(crate) dock: Entity<DockArea>,
    pub(crate) focus: FocusHandle,
    pub(crate) name: String,
    pub(crate) saved: DockAreaState,
    pub(crate) modified: bool,
    pub(crate) home: bool,
    pub(crate) resume: Option<DockAreaState>,
    pub(crate) active: String,
    pub(crate) resume_active: String,
    pub(crate) arrangements: std::collections::BTreeMap<String, DockAreaState>,
    pub(crate) close_chain: Option<DockAreaState>,
    /// The arrangement before the last hide, and the module that was hidden (it gets the keyboard back).
    pub(crate) undo_close: Option<(DockAreaState, String)>,
    pub(crate) error: Option<String>,
    notice_seen: Option<String>,
    notice_epoch: u64,
    overlay_count: usize,
    /// Focus to give back when the last overlay closes (`overlay_focus.rs`).
    pub(crate) overlay_before: Option<FocusHandle>,
    pub(crate) pre_click: Option<(FocusHandle, std::time::Instant)>,
    pub(crate) undo_token: Option<u64>,
    _toast_watcher: Task<()>,
    notification_seen: std::collections::HashSet<EntityId>,
    subscriptions: Vec<Subscription>,
    watched: HashSet<base::PanelId>,
    pub(crate) prompt: Option<Entity<InputState>>,
    pub(crate) prompt_subscription: Option<Subscription>,
    pub(crate) keys: Option<Entity<crate::keys::KeysEditor>>,
    pub(crate) keys_subscription: Option<Subscription>,
    pub(crate) settings: Option<Entity<crate::settings_ui::SettingsView>>,
    pub(crate) settings_subscription: Option<Subscription>,
    hints: Entity<crate::hints::HintLayer>,
    #[cfg(feature = "palette")]
    pub(crate) palette: Entity<ados_palette::Palette>,
    #[cfg(feature = "assistant")]
    assistant: Entity<ados_assistant::Assistant>,
}
impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dock = ShellSkin::area(window, cx);
        cx.set_global(crate::dock_skin::TabTargets::default());
        let toast_watcher = cx.spawn_in(window, async move |entity, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
                if entity.update_in(cx, |s: &mut Self, window, cx| {
                    for note in window.notifications(cx).iter() {
                        if !s.notification_seen.insert(note.entity_id()) { continue; }
                        // Component 0.9.1 hides ordinary toasts after five seconds,
                        // but action toasts otherwise live forever. Bound those too.
                        let weak = note.downgrade();
                        cx.spawn_in(window, async move |_, cx| {
                            cx.background_executor().timer(std::time::Duration::from_secs(8)).await;
                            let _ = weak.update_in(cx, |note, window, cx| note.dismiss(window, cx));
                        }).detach();
                    }
                }).is_err() { break; }
            }
        });
        let mut shell = Self {
            dock,
            focus: cx.focus_handle(),
            name: "Research".into(),
            saved: DockAreaState::default(),
            modified: false,
            home: false,
            resume: None,
            active: "browser".into(),
            resume_active: "browser".into(),
            arrangements: Default::default(),
            close_chain: None,
            undo_close: None,
            error: None,
            notice_seen: None,
            notice_epoch: 0,
            overlay_count: 0,
            overlay_before: None,
            pre_click: None,
            undo_token: None,
            _toast_watcher: toast_watcher,
            notification_seen: Default::default(),
            subscriptions: Vec::new(),
            watched: HashSet::new(),
            prompt: None,
            prompt_subscription: None,
            keys: None,
            keys_subscription: None,
            settings: None,
            settings_subscription: None,
            hints: cx.new(|cx| crate::hints::HintLayer::new(window, cx)),
            #[cfg(feature = "palette")]
            palette: ados_palette::Palette::view(window, cx),
            #[cfg(feature = "assistant")]
            assistant: ados_assistant::Assistant::view(window, cx),
        };
        for name in storage::DEFAULTS {
            if !storage::exists(name) {
                let layout = layouts::default_layout(name, window, cx);
                shell
                    .dock
                    .update(cx, |dock, cx| dock.set_center(layout, window, cx));
                if let Err(e) =
                    storage::save(&storage::Layout::new(name, shell.dock.read(cx).dump(cx)))
                {
                    shell.error = Some(format!("Could not create the default layouts: {e}"));
                }
            }
        }
        let mut first_run = false;
        match storage::working() {
            Ok(Some(mut state)) => {
                layouts::fit(&mut state.layout.dock, window);
                layouts::fit(&mut state.saved, window);
                match shell
                    .dock
                    .update(cx, |dock, cx| dock.load(state.layout.dock, window, cx))
                {
                    Ok(()) => {
                        shell.arrangements = state.arrangements;
                        shell.name = state.layout.name;
                        shell.saved = state.saved;
                        shell.active = state.active_module;
                    }
                    Err(e) => {
                        shell.error = Some(format!("Could not restore the workspace: {e}"));
                        shell.load_default(window, cx);
                    }
                }
            }
            Ok(None) => {
                shell.load_default(window, cx);
                first_run = true;
            }
            Err(e) => {
                shell.error = Some(format!(
                    "Could not read the last workspace: {e}. Opening Research instead."
                ));
                shell.load_default(window, cx);
            }
        }
        shell.modified = !storage::same_arrangement(&shell.dock.read(cx).dump(cx), &shell.saved);
        cx.observe_in(&shell.dock, window, |s, _, window, cx| {
            ados_core::overlay::bump_layout_epoch(cx);
            let names: Vec<String> = s.panels(cx).iter().map(|p| p.panel_name(cx).to_string()).collect();
            crate::modules::sync_in_layout(&names, window, cx);
        })
        .detach();
        shell.subscriptions.push(cx.observe_global_in::<ados_core::overlay::OverlayGuard>(window, |s, window, cx| {
            let count = ados_core::overlay::OverlayGuard::count(cx);
            if count < s.overlay_count {
                window.defer(cx, |window, cx| ados_core::overlay::clear_pointer(window, cx));
            }
            let before = s.overlay_count;
            s.overlay_count = count;
            s.overlay_changed(before, count, window, cx);
        }));
        shell.subscriptions.push(cx.observe_global_in::<ados_core::shortcuts::Shortcuts>(window, |s, window, cx| {
            s.shortcut_notices(window, cx);
            cx.notify();
        }));
        cx.defer_in(window, |s, window, cx| s.shortcut_notices(window, cx));
        // One place for every removal (an undo, trash, refresh or the assistant took away the
        // focused row): focus goes to the nearest part still on screen (a list), else to the
        // active module, so the keyboard never goes dead.
        shell.subscriptions.push(cx.on_focus_lost(window, |s, window, cx| {
            // Only a target inside a module: a layout switch rebuilds the dock, and for a frame
            // the nearest survivor is the dock's own tab panel, which owns no keys (the rail
            // would mark a module while nothing had the keyboard).
            let in_module = |target: &FocusHandle, window: &Window, cx: &App| s.panels(cx).iter()
                .any(|panel| crate::modules::root_focus(panel, cx).contains(target, window));
            match window.focus_lost_restore_target(cx) {
                Some(target) if in_module(&target, window, cx) => window.focus(&target, cx),
                _ => crate::modules::focus_primary(&s.active.clone(), window, cx),
            }
        }));
        shell.watch_focus(window, cx);
        shell.focus_loaded(&shell.active.clone(), window, cx);
        shell
            .subscriptions
            .push(cx.subscribe_in(&shell.dock, window, |s, _, event, w, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    s.changed(w, cx);
                }
            }));
        // A chosen start-up layout replaces the last arrangement.
        if let Some(name) = ados_core::settings::get(cx).startup_layout {
            if !first_run && name != shell.name && storage::exists(&name) {
                shell.switch_to(&name, false, window, cx);
            }
        }
        if let Some(problem) = cx.try_global::<crate::settings_ui::StartupProblem>() {
            shell.error = Some(problem.0.clone());
        }
        if first_run {
            // A fresh install starts on Home (where everything is pointed to); Resume opens Research.
            shell.show_home(window, cx);
        }
        shell
    }
    fn load_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let layout = layouts::default_layout("Research", window, cx);
        self.dock
            .update(cx, |dock, cx| dock.set_center(layout, window, cx));
        self.saved = self.dock.read(cx).dump(cx);
    }
    pub(crate) fn panels(&self, cx: &App) -> Vec<Arc<dyn PanelView>> {
        let area = self.dock.read(cx);
        let mut ids = Vec::new();
        for placement in [
            base::DockPlacement::Center,
            base::DockPlacement::Left,
            base::DockPlacement::Right,
            base::DockPlacement::Bottom,
        ] {
            if let Some(tree) = area.layout(placement) {
                tree.root().walk(&mut |node| {
                    if let PaneRef::Tabs { panels, .. } = node.kind() {
                        ids.extend_from_slice(panels);
                    }
                });
            }
        }
        ids.into_iter()
            .filter_map(|id| area.panel(id).cloned())
            .collect()
    }
    pub(crate) fn set_active(&mut self, id: &str, cx: &mut App) {
        self.active = id.to_string();
        ados_core::hub::ContextHub::set_active(id, cx);
    }
    pub(crate) fn watch_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for panel in self.panels(cx) {
            if !self.watched.insert(panel.panel_id(cx)) {
                continue;
            }
            let module = panel.panel_name(cx).to_string();
            // The root contains every part of the module (list, editor, search, form), so
            // focus anywhere in it makes it the active module, whichever part has focus.
            let root = crate::modules::root_focus(&panel, cx);
            self.subscriptions.push(cx.on_focus_in(&root, window, move |s, w, cx| {
                s.set_active(&module, cx);
                s.persist(cx);
                w.refresh();
                cx.notify();
            }));
            // The root itself only gets focus from a click on empty space or a focus restore:
            // hand it on to the part the module would focus, as before the root was tracked.
            let target = panel.clone();
            self.subscriptions.push(cx.on_focus(&root, window, move |_, w, cx| {
                let handle = target.focus_handle(cx);
                if !handle.is_focused(w) {
                    w.focus(&handle, cx);
                }
            }));
        }
    }
    #[cfg(not(all(feature = "palette", feature = "assistant")))]
    fn unavailable(&mut self, what: &str, cx: &mut Context<Self>) {
        self.error = Some(format!("{what} is not enabled in this shell build."));
        cx.defer(|cx| cx.refresh_windows());
        cx.notify();
    }
}
impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // In Home mode the layout is the one Resume brings back, not the Home view.
        self.modified = !storage::same_arrangement(&self.dump(cx), &self.saved);
        if cx.try_global::<ados_core::overlay::BottomNotice>().is_none_or(|s| s.0 != self.error.is_some()) {
            cx.set_global(ados_core::overlay::BottomNotice(self.error.is_some()));
        }
        let home_info = crate::home::HomeInfo { layout: self.name.clone(), only_home: self.panels(cx).len() == 1 && self.panels(cx).first().is_some_and(|p| p.panel_name(cx) == ados_core::ids::HOME) };
        if cx.try_global::<crate::home::HomeInfo>() != Some(&home_info) { cx.set_global(home_info); }
        if self.notice_seen != self.error {
            self.notice_seen = self.error.clone();
            self.notice_epoch += 1;
            if self.error.is_some() {
                let epoch = self.notice_epoch;
                let undo = self.error.as_deref().is_some_and(|text| text.ends_with(" hidden."));
                let token = if undo { self.undo_token } else { None };
                cx.spawn(async move |entity, cx| {
                    cx.background_executor().timer(std::time::Duration::from_secs(if undo { 8 } else { 4 })).await;
                    let _ = entity.update(cx, |s, cx| {
                        if s.notice_epoch == epoch {
                            s.error = None;
                            cx.notify();
                        }
                        if let Some(token) = token { ados_core::notifications::forget_undo(token, cx); }
                    });
                }).detach();
            }
        }
        let layout_names = storage::names();
        let current_layout = self.name.clone();
        let menu_focus = self.focus.clone();
        let top = h_flex()
            .h(crate::settings_ui::zoomed(48., cx))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .items_center()
            .bg(cx.theme().title_bar)
            .child(
                Button::new("layout-switch")
                    .label(self.name.clone())
                    .child(ados_core::hints::mark())
                    .icon(IconName::ChevronDown)
                    .ghost()
                    .tooltip("Switch layout")
                    .dropdown_menu(move |mut menu, _, cx| {
                        for name in &layout_names {
                            menu = menu.menu(
                                format!("{}{}", if *name == current_layout { "✓  " } else { "" }, name),
                                Box::new(actions::SwitchLayout { name: name.clone() }),
                            );
                        }
                        let menu = menu
                            .separator()
                            .menu("Save layout", Box::new(actions::SaveLayout))
                            .menu("Save as new layout…", Box::new(actions::SaveLayoutAs))
                            .menu("Revert to saved", Box::new(actions::RevertLayout))
                            .action_context(menu_focus.clone());
                        crate::dock_skin::guard(menu, cx)
                    }),
            )
            .when(self.modified, |el| el.child(
                h_flex().gap_2().items_center()
                    .child(div().id("layout-modified").size(px(8.)).rounded_full().bg(cx.theme().muted_foreground)
                        .tooltip(|w, cx| gpui_kit::component::tooltip::Tooltip::new("Modified").build(w, cx)))
                    .when(f32::from(window.viewport_size().width) > 1000., |el| el.child(div().text_sm().text_color(cx.theme().muted_foreground).child("Modified")))
            ))
            .child(
                Button::new("save-layout")
                    .label("Save layout")
                    .child(ados_core::hints::mark())
                    .ghost()
                    .tooltip(ados_core::shortcuts::tooltip("Save layout", "shell.save", cx))
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.save_layout(None, cx);
                    })),
            )
            .child(div().id("drag-titlebar").flex_1().h_full().on_mouse_down(
                MouseButton::Left,
                |e, w, _| {
                    if e.click_count == 2 {
                        w.zoom_window();
                    } else {
                        w.start_window_move();
                    }
                },
            ))
            .child(ados_core::hints::marked(
                Button::new("capture")
                    .icon(IconName::Plus)
                    .ghost()
                    .tooltip(ados_core::shortcuts::tooltip("Quick capture", "shell.capture", cx))
                    .on_click(|_, w, cx| w.dispatch_action(Box::new(actions::OpenCapture), cx)),
            ))
            .child(ados_core::hints::marked(
                Button::new("assistant")
                    .icon(IconName::Bot)
                    .ghost()
                    .tooltip(ados_core::shortcuts::tooltip("Assistant", "assistant.toggle", cx))
                    .on_click(|_, w, cx| w.dispatch_action(Box::new(actions::ToggleAssistant), cx)),
            ))
            .when(matches!(window.window_decorations(), Decorations::Client { .. }), |top| top
            .child(div().w(px(1.)).h(px(20.)).bg(cx.theme().border))
            .child(
                Button::new("minimize-window")
                    .icon(IconName::Minus)
                    .ghost()
                    .tooltip("Minimize window")
                    .on_click(|_, w, _| w.minimize_window()),
            )
            .child(
                Button::new("maximize-window")
                    .icon(IconName::Maximize)
                    .ghost()
                    .tooltip("Maximize or restore window")
                    .on_click(|_, w, _| w.zoom_window()),
            )
            .child(
                Button::new("close-window")
                    .icon(IconName::Close)
                    .ghost()
                    .tooltip("Close aDOs")
                    .on_click(cx.listener(|s, _, w, cx| {
                        s.persist(cx);
                        w.remove_window();
                    })),
            ));
        let rail = self.rail(cx);
        let mut root = div()
            .id("ados-shell")
            .key_context("AdosShell")
            .track_focus(&self.focus)
            .capture_any_mouse_down(cx.listener(|s, _, w, cx| s.note_press(w, cx)))
            .capture_key_down(cx.listener(|s, _, w, cx| s.note_key(w, cx)))
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|s, a: &actions::SwitchLayout, w, cx| s.switch(&a.name, w, cx)))
            .on_action(
                cx.listener(|s, a: &actions::FocusModule, w, cx| s.focus_module(&a.id, w, cx)),
            )
            .on_action(cx.listener(|s, _: &actions::SaveLayout, _, cx| {
                s.save_layout(None, cx);
            }))
            .on_action(cx.listener(|s, _: &actions::SaveLayoutAs, w, cx| s.open_prompt(w, cx)))
            .on_action(cx.listener(|s, _: &actions::RevertLayout, w, cx| s.revert_layout(w, cx)))
            .on_action(cx.listener(|s, _: &actions::ResetLayouts, w, cx| s.confirm_reset(w, cx)))
            .on_action(cx.listener(|_, _: &actions::OpenDataFolder, _, cx| {
                cx.open_url(&format!("file://{}", ados_core::paths::home().display()));
            }))
            .on_action(cx.listener(|s, _: &actions::About, _, cx| {
                s.error = Some(format!("aDOs {} · Data: {}", env!("CARGO_PKG_VERSION"), ados_core::paths::home().display()));
                cx.notify();
            }))
            .on_action(cx.listener(|s, _: &actions::BrowserAddress, w, cx| {
                s.focus_module(ados_core::ids::BROWSER, w, cx);
                modules::focus_browser_address(w, cx);
            }))
            .on_action(cx.listener(|s, a: &crate::links::OpenInBrowser, w, cx| s.open_in_browser(&a.text, w, cx)))
            .on_action(cx.listener(|_, _: &crate::page_actions::PageToNote, w, cx| crate::page_actions::run_page(crate::page_actions::Kind::Note, w, cx)))
            .on_action(cx.listener(|_, _: &crate::page_actions::PageAppendNote, w, cx| crate::page_actions::run_page(crate::page_actions::Kind::Append, w, cx)))
            .on_action(cx.listener(|_, _: &crate::page_actions::PageToTask, w, cx| crate::page_actions::run_page(crate::page_actions::Kind::Task, w, cx)))
            .on_action(cx.listener(|_, _: &crate::page_actions::PageCopyLink, w, cx| crate::page_actions::run_page(crate::page_actions::Kind::CopyLink, w, cx)))
            .on_action(cx.listener(|s, _: &actions::ReloadBrowser, w, cx| {
                #[cfg(feature = "browser")]
                if let Some(panel) = s.panels(cx).into_iter().find(|panel| panel.panel_name(cx) == "browser") {
                    panel.focus_handle(cx).focus(w, cx);
                    w.dispatch_action(Box::new(ados_browser::Reload), cx);
                    return;
                }
                s.error = Some("Open the Browser module first".into());
                cx.notify();
            }))
            .on_action(cx.listener(|s, _: &actions::UndoLast, w, cx| {
                // Never silent: with nothing left to undo, say so in the bottom notice.
                if !ados_core::notifications::undo_latest(w, cx) {
                    s.error = Some("Nothing to undo.".into());
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|s, _: &actions::ResumeLayout, w, cx| s.resume_layout(w, cx)))
            .on_action(cx.listener(|s, _: &actions::EditShortcuts, w, cx| s.open_keys(false, w, cx)))
            .on_action(cx.listener(|s, _: &crate::settings_ui::OpenSettings, w, cx| s.open_settings(w, cx)))
            .on_action(cx.listener(|s, _: &crate::settings_ui::ZoomIn, _, cx| s.zoom_ui(1, cx)))
            .on_action(cx.listener(|s, _: &crate::settings_ui::ZoomOut, _, cx| s.zoom_ui(-1, cx)))
            .on_action(cx.listener(|s, _: &crate::settings_ui::ZoomReset, _, cx| s.zoom_ui(0, cx)))
            .on_action(cx.listener(|s, _: &actions::ShowShortcutConflicts, w, cx| s.open_keys(true, w, cx)))
            .on_action(cx.listener(|s, _: &actions::CloseModule, w, cx| {
                s.panel_command(
                    &PanelCommand {
                        module: s.active.clone(),
                        operation: "hide".into(),
                    },
                    w,
                    cx,
                )
            }))
            .on_action(cx.listener(|s, _: &actions::ZoomModule, w, cx| {
                s.panel_command(
                    &PanelCommand {
                        module: s.active.clone(),
                        operation: "zoom".into(),
                    },
                    w,
                    cx,
                )
            }))
            .on_action(cx.listener(|s, a: &PanelCommand, w, cx| s.panel_command(a, w, cx)))
            .on_action(cx.listener(|s, _: &crate::hints::HintMode, w, cx| s.hints.update(cx, |h, cx| h.toggle(w, cx))))
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" && s.prompt.is_some() {
                    s.close_prompt(w, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                h_flex().size_full().child(rail).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(top)
                        .child(div().h(px(1.)).flex_shrink_0().w_full().bg(linear_gradient(
                            90.,
                            linear_color_stop(rgb(ados_core::brand::BLUE), 0.),
                            linear_color_stop(rgb(ados_core::brand::RED), 1.),
                        )))
                        .child(div().flex_1().min_h_0().child(self.dock.clone())),
                ),
            );
        #[cfg(not(feature = "palette"))]
        {
            root = root
                .on_action(cx.listener(|s, _: &actions::OpenPalette, _, cx| {
                    s.unavailable("The command palette", cx)
                }))
                .on_action(cx.listener(|s, _: &actions::OpenCapture, _, cx| {
                    s.unavailable("Quick capture", cx)
                }));
        }
        #[cfg(feature = "palette")]
        {
            root = root
                .on_action(cx.listener(|s, _: &actions::OpenPalette, window, cx| {
                    s.palette.update(cx, |p, cx| p.open_actions(window, cx))
                }))
                .on_action(cx.listener(|s, _: &actions::OpenCapture, window, cx| {
                    s.palette.update(cx, |p, cx| p.open_capture(window, cx))
                }));
        }
        #[cfg(feature = "assistant")]
        {
            root = root.on_action(cx.listener(|s, _: &actions::ToggleAssistant, window, cx| {
                s.assistant.update(cx, |a, cx| a.toggle(window, cx))
            })).on_action(cx.listener(|s, _: &actions::NewAssistantChat, window, cx| {
                s.assistant.update(cx, |a, cx| a.new_chat(window, cx))
            }));
        }
        #[cfg(not(feature = "assistant"))]
        {
            root = root.on_action(cx.listener(|s, _: &actions::ToggleAssistant, _, cx| {
                s.unavailable("The assistant", cx)
            }));
        }
        if let Some(error) = &self.error {
            root = root.child(
                h_flex()
                    .absolute()
                    .bottom(px(16.))
                    .right_3()
                    .max_w((window.viewport_size().width - px(100.)).min(px(520.)))
                    .p_3()
                    .gap_3()
                    .items_center()
                    .rounded_lg()
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(ados_core::hints::cover(ados_core::hints::Layer::Float))
                    .child(div().flex_1().text_sm().child(error.clone()))
                    .when(self.undo_close.is_some() && error.ends_with(" hidden."), |el| el.child(
                        Button::new("undo-close").label("Undo").ghost().tooltip(ados_core::shortcuts::tooltip("Undo", "shell.undo", cx)).child(ados_core::hints::mark_in(ados_core::hints::Layer::Float))
                            .on_click(cx.listener(|s, _, w, cx| s.undo_closed(w, cx)))
                    ))
                    .child(
                        ados_core::hints::marked_in(ados_core::hints::Layer::Float, Button::new("dismiss-error")
                            .icon(IconName::Close)
                            .ghost()
                            .tooltip("Dismiss message")
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.error = None;
                                cx.notify();
                            }))),
                    ),
            );
        }
        if let Some(input) = &self.prompt {
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(cx.theme().background.opacity(0.9))
                    .child(
                        v_flex()
                            .w(px(420.))
                            .p_5()
                            .gap_4()
                            .rounded_lg()
                            .bg(cx.theme().popover)
                            .border_1()
                            .border_color(cx.theme().border)
                            .child("Save as new layout")
                            .child(Input::new(input))
                            .when_some(self.error.clone(), |el, e| {
                                el.child(div().text_sm().text_color(cx.theme().danger).child(e))
                            })
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("cancel-layout")
                                            .label("Cancel")
                                            .ghost()
                                            .on_click(
                                                cx.listener(|s, _, w, cx| s.close_prompt(w, cx)),
                                            ),
                                    )
                                    .child(
                                        Button::new("confirm-layout")
                                            .label("Save")
                                            .primary()
                                            .on_click(cx.listener(|s, _, w, cx| {
                                                let name = s
                                                    .prompt
                                                    .as_ref()
                                                    .map(|i| i.read(cx).value().to_string());
                                                if let Some(name) = name {
                                                    if s.save_layout(Some(name), cx) {
                                                        s.close_prompt(w, cx);
                                                    }
                                                }
                                            })),
                                    ),
                            ),
                    ),
            );
        }
        if let Some(editor) = &self.keys {
            root = root.child(editor.clone());
        }
        if let Some(view) = &self.settings {
            root = root.child(view.clone());
        }
        #[cfg(feature = "palette")]
        {
            root = root.child(self.palette.clone());
        }
        #[cfg(feature = "assistant")]
        {
            root = root.child(self.assistant.clone());
        }
        root = root.child(self.hints.clone());
        root.child(canvas(|_, _, _| (), |_, _, window, _| {
            window.on_mouse_event(|event: &MouseExitEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture && event.pressed_button.is_none() {
                    window.defer(cx, |window, cx| ados_core::overlay::clear_pointer(window, cx));
                }
            });
        }).absolute().size_full())
    }
}
