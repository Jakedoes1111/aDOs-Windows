//! Operations use DockArea's real tree editor and TabGroupContext commands.
use crate::{dock_skin::PanelCommand, shell::Shell};
use gpui_kit::base::dock::{self as base, PaneRef};
use gpui_kit::*;

impl Shell {
    pub(crate) fn undo_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(token) = self.undo_token.take() {
            ados_core::notifications::forget_undo(token, cx);
        }
        let Some((mut state, hidden)) = self.undo_close.take() else { return; };
        crate::layouts::fit(&mut state, window);
        if let Err(error) = self.dock.update(cx, |dock, cx| dock.load(state, window, cx)) {
            self.error = Some(format!("Could not reopen the module: {error}"));
            return;
        }
        self.home = false;
        self.resume = None;
        self.close_chain = None;
        self.error = None;
        // F2: the module that comes back owns the keyboard, as if it had been entered.
        self.focus_loaded(&hidden, window, cx);
        self.changed(window, cx);
    }
    pub(crate) fn panel_command(
        &mut self,
        command: &PanelCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home && command.module == "home" && command.operation == "hide" {
            self.resume_layout(window, cx);
            return;
        }
        let Some(panel) = self
            .panels(cx)
            .into_iter()
            .find(|p| p.panel_name(cx) == command.module)
        else {
            return;
        };
        let id = panel.panel_id(cx);
        match command.operation.as_str() {
            "hide" | "zoom" => {
                let target = cx
                    .global::<crate::dock_skin::TabTargets>()
                    .0
                    .get(&command.module)
                    .cloned();
                if let Some(target) = target {
                    if command.operation == "hide" {
                        let before = self.dump(cx);
                        if self.close_chain.is_none() { self.close_chain = Some(before.clone()); }
                        self.undo_close = Some((before, command.module.clone()));
                        let name = crate::modules::get(&command.module)
                            .map(|module| module.title).unwrap_or(&command.module);
                        self.error = Some(format!("{name} hidden."));
                        let shell = cx.entity().downgrade();
                        let token = ados_core::notifications::remember_undo(move |w, cx| {
                            let _ = shell.update(cx, |s, cx| s.undo_closed(w, cx));
                        }, cx);
                        self.undo_token = Some(token);
                        target.close(id, window, cx);
                        cx.defer_in(window, move |s, w, cx| {
                            let next = s
                                .panels(cx)
                                .iter()
                                .find(|p| p.panel_id(cx) != id)
                                .map(|p| p.panel_name(cx).to_string());
                            if let Some(next) = next {
                                s.focus_loaded(&next, w, cx);
                            } else {
                                s.show_home(w, cx);
                            }
                        });
                    } else {
                        if let Some(ix) = target.panels().iter().position(|p| p.panel_id(cx) == id) {
                            target.select_tab(ix, window, cx);
                        }
                        // Keep the keyboard where it is inside the module (Shift+Esc from the
                        // Sessions composer stays in the composer); focus the module only when
                        // zooming one the keyboard is not in. A module's own handle need not
                        // contain its fields (Sessions hands out its list), so ask the shell.
                        let module_focus = panel.focus_handle(cx);
                        if command.module != self.active && !module_focus.contains_focused(window, cx) {
                            window.focus(&module_focus, cx);
                        }
                        target.toggle_zoom(window, cx);
                        // Zoom is a presentation mode, not a changed arrangement.
                        window.refresh();
                        cx.notify();
                        return;
                    }
                }
            }
            op => {
                self.close_chain = None;
                let Some(tree) = self.dock.read(cx).layout(base::DockPlacement::Center) else {
                    return;
                };
                let Some(source) = tree.find_panel_node(id) else {
                    self.error = Some("Move this module into the centre before splitting it.".into());
                    cx.notify();
                    return;
                };
                let target = if op == "centre" {
                    let mut ids = Vec::new();
                    tree.root().walk(&mut |node| {
                        if let PaneRef::Tabs { panels, .. } = node.kind() {
                            ids.extend_from_slice(panels);
                        }
                    });
                    let node = ids
                        .iter()
                        .find_map(|other| {
                            if *other != id {
                                tree.find_panel_node(*other)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(source);
                    base::InsertTarget::Tabs {
                        node,
                        ix: None,
                        activate: true,
                    }
                } else {
                    let is_split = op.starts_with("split-");
                    if is_split
                        && tree
                            .find_node(source)
                            .map(|n| matches!(n.kind(), PaneRef::Tabs { panels, .. } if panels.len() < 2))
                            .unwrap_or(true)
                    {
                        self.error =
                            Some("To split a tab out, first drag another module into its tab group.".into());
                        cx.notify();
                        return;
                    }
                    base::InsertTarget::Split {
                        node: if is_split { source } else { tree.root().id() },
                        placement: match op {
                            "left" => gpui_kit::base::Placement::Left,
                            "top" => gpui_kit::base::Placement::Top,
                            "bottom" | "split-down" => gpui_kit::base::Placement::Bottom,
                            _ => gpui_kit::base::Placement::Right,
                        },
                        size: None,
                    }
                };
                self.dock
                    .update(cx, |dock, cx| dock.move_panel(id, target, window, cx));
            }
        }
        self.changed(window, cx);
    }
}
