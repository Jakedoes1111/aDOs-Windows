use crate::{
    modules,
    shell::Shell,
    storage::{self, Layout, Working},
};
use gpui_kit::base::dock::{self as base, DockAreaState, DockLayout, PanelView};
use gpui_kit::*;
use std::sync::Arc;

pub fn default_layout(name: &str, window: &mut Window, cx: &mut App) -> DockLayout {
    let create = |id: &str, w: &mut Window, cx: &mut App| -> Arc<dyn PanelView> {
        modules::get(id).expect("static module id").panel(w, cx)
    };
    let tab =
        |id, w: &mut Window, cx: &mut App| DockLayout::tabs().panel_view(create(id, w, cx), cx);
    let half_width = (window.viewport_size().width - px(96.)) / 2.;
    let half_height = (window.viewport_size().height - px(73.)) / 2.;
    match name {
        "Code" => {
            let notes = tab("notes", window, cx);
            let terminal = tab("terminal", window, cx);
            DockLayout::h_split()
                .child(terminal, Some(half_width))
                .child(notes, Some(half_width))
        }
        "Focus" => tab("notes", window, cx),
        _ => {
            let browser = tab("browser", window, cx);
            let notes = tab("notes", window, cx);
            let terminal = tab("terminal", window, cx);
            let right = DockLayout::v_split()
                .child(notes, Some(half_height))
                .child(terminal, Some(half_height));
            DockLayout::h_split()
                .child(browser, Some(half_width))
                .child(right, Some(half_width))
        }
    }
}
/// The dock schema stores absolute sizes. Fit their ratios to this window at load time.
pub fn fit(state: &mut DockAreaState, window: &Window) {
    storage::fit_to_viewport(
        state,
        (window.viewport_size().width - px(96.)).max(px(100.)),
        (window.viewport_size().height - px(73.)).max(px(100.)),
    );
}
impl Shell {
    pub(crate) fn dump(&self, cx: &App) -> DockAreaState {
        self.resume
            .as_ref()
            .cloned()
            .unwrap_or_else(|| self.dock.read(cx).dump(cx))
    }
    pub(crate) fn persist(&mut self, cx: &mut App) {
        self.arrangements.insert(self.name.clone(), self.dump(cx));
        let working = Working {
            layout_version: storage::VERSION,
            layout: Layout::new(&self.name, self.dump(cx)),
            saved: self.saved.clone(),
            arrangements: self.arrangements.clone(),
            active_module: if self.home {
                self.resume_active.clone()
            } else {
                self.active.clone()
            },
        };
        if let Err(e) = storage::save_working(&working) {
            self.error = Some(format!("Could not save the workspace: {e}"));
        }
    }
    pub(crate) fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        self.modified = !storage::same_arrangement(&self.dump(cx), &self.saved);
        self.watch_focus(window, cx);
        self.persist(cx);
        window.refresh();
        cx.notify();
    }
    pub(crate) fn switch(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_to(name, false, window, cx);
    }
    pub(crate) fn revert_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_to(&self.name.clone(), true, window, cx);
    }
    pub(crate) fn switch_to(&mut self, name: &str, revert: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !revert { self.persist(cx); }
        let mut baseline = match storage::load(name) {
            Ok(layout) => layout.dock,
            Err(e) => {
                self.error = Some(format!("Could not open layout {name}: {e}"));
                cx.notify();
                return;
            }
        };
        fit(&mut baseline, window);
        let mut state = if revert { baseline.clone() } else {
            self.arrangements.get(name).cloned().unwrap_or_else(|| baseline.clone())
        };
        fit(&mut state, window);
        self.home = false;
        self.resume = None;
        self.close_chain = None;
        match self
            .dock
            .update(cx, |dock, cx| dock.load(state, window, cx))
        {
            Ok(()) => {
                self.name = name.into();
                self.saved = baseline;
                self.modified = !storage::same_arrangement(&self.dock.read(cx).dump(cx), &self.saved);
                self.watch_focus(window, cx);
                let next = self
                    .panels(cx)
                    .first()
                    .map(|p| p.panel_name(cx).to_string());
                if let Some(next) = next {
                    self.focus_module(&next, window, cx);
                }
                self.persist(cx);
                window.refresh();
                cx.notify();
            }
            Err(e) => {
                self.error = Some(format!("Could not restore this layout: {e}"));
                cx.notify();
            }
        }
    }
    pub(crate) fn save_layout(&mut self, new_name: Option<String>, cx: &mut Context<Self>) -> bool {
        let name = new_name.as_deref().unwrap_or(&self.name);
        if let Err(e) = storage::validate_name(name) {
            self.error = Some(e.to_string());
            cx.notify();
            return false;
        }
        if new_name.is_some() && storage::exists(name) {
            self.error =
                Some("A layout with that name already exists. Choose another name.".into());
            cx.notify();
            return false;
        }
        let layout = Layout::new(name, self.dump(cx));
        if let Err(e) = storage::save(&layout) {
            self.error = Some(format!("Could not save this layout: {e}"));
            cx.notify();
            return false;
        }
        self.name = layout.name;
        self.saved = layout.dock;
        self.modified = false;
        self.error = Some(format!("Saved layout {}", self.name));
        crate::actions::register_layouts(cx);
        self.persist(cx);
        cx.defer(|cx| cx.refresh_windows());
        cx.notify();
        true
    }
    pub(crate) fn show_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        let current = self.dock.read(cx).dump(cx);
        // Panel removal is deferred by the dock: its tree can still contain the
        // last closing panel here. Never snapshot that transient tree for Resume.
        self.resume = Some(self.close_chain.take().unwrap_or(current));
        self.resume_active = self.active.clone();
        self.home = true;
        self.persist(cx);
        let panel = modules::MODULES[0].panel(window, cx);
        let home = DockLayout::tabs().panel_view(panel, cx);
        self.dock
            .update(cx, |dock, cx| dock.set_center(home, window, cx));
        self.watch_focus(window, cx);
        modules::focus_primary("home", window, cx);
        self.set_active("home", cx);
        window.refresh();
        cx.notify();
    }
    pub(crate) fn resume_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut state) = self.resume.take() else {
            // Home can be the only module without Home mode (the user hid the others),
            // so nothing was snapshotted. "Resume last layout" then reopens the saved one.
            self.revert_layout(window, cx);
            return;
        };
        fit(&mut state, window);
        fit(&mut self.saved, window);
        self.home = false;
        self.modified = !storage::same_arrangement(&state, &self.saved);
        self.undo_close = None;
        self.error = None;
        if let Err(e) = self
            .dock
            .update(cx, |dock, cx| dock.load(state, window, cx))
        {
            self.error = Some(format!("Could not resume the workspace: {e}"));
        }
        self.watch_focus(window, cx);
        self.focus_loaded(&self.resume_active.clone(), window, cx);
        window.refresh();
        cx.notify();
    }
    /// Restore real keyboard focus and context together; legacy active ids may have been dropped.
    pub(crate) fn focus_loaded(
        &mut self,
        preferred: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panels = self.panels(cx);
        if let Some(panel) = panels
            .iter()
            .find(|p| p.panel_name(cx) == preferred)
            .or(panels.first())
        {
            let id = panel.panel_name(cx).to_string();
            let panel_id = panel.panel_id(cx);
            self.dock
                .update(cx, |dock, cx| dock.select_panel(panel_id, window, cx));
            modules::focus_primary(&id, window, cx);
            self.set_active(&id, cx);
        }
    }
    pub(crate) fn focus_module(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        // F1: a module chord pressed while the palette is open moves the keyboard; the palette
        // must not stay drawn over it as if it still had it.
        #[cfg(feature = "palette")]
        self.palette.update(cx, |palette, cx| palette.close(window, cx));
        if id == "agent" {
            window.dispatch_action(Box::new(crate::actions::ToggleAssistant), cx);
            return;
        }
        if self.home {
            self.resume_layout(window, cx);
        }
        self.close_chain = None;
        let Some(module) = modules::get(id) else {
            return;
        };
        let panel = self
            .panels(cx)
            .into_iter()
            .find(|p| p.panel_name(cx) == id)
            .unwrap_or_else(|| {
                let panel = module.panel(window, cx);
                self.dock.update(cx, |dock, cx| {
                    dock.add_panel_view(
                        panel.clone(),
                        base::DockPlacement::Center,
                        None,
                        window,
                        cx,
                    )
                });
                panel
            });
        let panel_id = panel.panel_id(cx);
        self.dock
            .update(cx, |dock, cx| dock.select_panel(panel_id, window, cx));
        modules::focus_primary(id, window, cx);
        self.set_active(id, cx);
        self.watch_focus(window, cx);
    }
}
