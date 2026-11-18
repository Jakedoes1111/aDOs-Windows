//! A thin chrome adapter. DockSkin still renders content, splitters and drop targets.
use crate::{actions, modules};
use gpui_kit::base::{dock::*, dock::AnyDrag, ResizeHandleContext, Selectable};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    dock::DockSkin,
    h_flex,
    menu::{ContextMenuExt, DropdownMenu, PopupMenu},
    ActiveTheme, Icon, IconName,
};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder;
use serde::Deserialize;
use std::{cell::Cell, collections::HashMap, rc::Rc, sync::Arc};

#[derive(Default)]
pub struct TabTargets(pub HashMap<String, TabGroupContext>);
impl Global for TabTargets {}

#[derive(Action, Clone, PartialEq, Eq, Deserialize)]
#[action(namespace = ados, no_json)]
pub struct PanelCommand {
    pub module: String,
    pub operation: String,
}
fn command(module: &str, op: &str) -> Box<dyn Action> {
    Box::new(PanelCommand {
        module: module.into(),
        operation: op.into(),
    })
}
pub fn guard(menu: PopupMenu, cx: &mut Context<PopupMenu>) -> PopupMenu {
    ados_core::overlay::OverlayGuard::open(cx);
    let open = Rc::new(Cell::new(true));
    let dismissed = open.clone();
    cx.subscribe(&cx.entity(), move |_, _, _: &DismissEvent, cx| {
        if dismissed.replace(false) {
            ados_core::overlay::OverlayGuard::close(cx);
        }
    })
    .detach();
    cx.on_release(move |_, cx| {
        if open.replace(false) {
            ados_core::overlay::OverlayGuard::close(cx);
        }
    })
    .detach();
    menu
}
fn menu(mut menu: PopupMenu, module: &str, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let id = module.to_string();
    let split_id = id.clone();
    menu = menu
        .label(modules::get(module).map(|m| m.title).unwrap_or("Module"))
        .submenu("Move", window, cx, move |m, _, _| {
            m.menu("Left", command(&id, "left"))
                .menu("Right", command(&id, "right"))
                .menu("Top", command(&id, "top"))
                .menu("Bottom", command(&id, "bottom"))
                .menu("Centre", command(&id, "centre"))
        })
        .submenu("Split", window, cx, move |m, _, _| {
            m.menu("Right", command(&split_id, "split-right"))
                .menu("Down", command(&split_id, "split-down"))
        })
        .separator()
        .menu_with_icon("Zoom", IconName::Maximize, command(module, "zoom"))
        .menu_with_icon("Hide module", IconName::EyeOff, command(module, "hide"))
        .separator()
        .menu_with_icon("Save layout", IconName::Check, Box::new(actions::SaveLayout))
        .menu_with_icon(
            "Save as new layout…",
            IconName::Plus,
            Box::new(actions::SaveLayoutAs),
        );
    guard(menu, cx)
}

pub struct ShellSkin {
    inner: Rc<DockSkin>,
}
impl ShellSkin {
    pub fn area(window: &mut Window, cx: &mut App) -> Entity<DockArea> {
        cx.new(|cx| {
            let inner = DockSkin::new(cx);
            DockArea::new("workspace", Some(1), window, cx).with_renderer(Rc::new(Self { inner }))
        })
    }
}
impl DockAreaRenderer for ShellSkin {
    fn render_dock(
        &self,
        ctx: &DockContext,
        content: AnyElement,
        w: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.inner.render_dock(ctx, content, w, cx)
    }
    fn frame(&self, w: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.frame(w, cx)
    }
    fn split_frame(&self, node: NodeId, axis: Axis, w: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.split_frame(node, axis, w, cx).gap_2()
    }
    fn center_frame(&self, w: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.center_frame(w, cx).p_3()
    }
    fn render_split_handle(
        &self,
        handle: &ResizeHandleContext,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.inner.render_split_handle(handle, w, cx)
    }
    fn build_placeholder(
        &self,
        state: &PanelState,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn PanelView>> {
        self.inner.build_placeholder(state, w, cx)
    }
    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(Self {
            inner: self.inner.clone(),
        })
    }
}
impl TabGroupRenderer for ShellSkin {
    fn frame(&self, group: &TabGroupContext, w: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner
            .tab_group_renderer()
            .frame(group, w, cx)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .overflow_hidden()
    }
    fn content_frame(&self, group: &TabGroupContext, w: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.tab_group_renderer().content_frame(group, w, cx)
    }
    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        w: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.inner
            .tab_group_renderer()
            .render_active_panel(panel, group, w, cx)
    }
    fn render_drop_indicator(
        &self,
        indicator: DropIndicator,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.inner
            .tab_group_renderer()
            .render_drop_indicator(indicator, w, cx)
    }
    fn render_tab_bar(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> AnyElement {
        let droppable = group.is_droppable();
        let mut tabs = h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .overflow_hidden();
        for (ix, panel) in group.panels().iter().enumerate() {
            let id = panel.panel_name(cx).to_string();
            cx.global_mut::<TabTargets>().0.insert(id.clone(), group.clone());
            let module = modules::get(&id);
            let name = ados_core::overlay::tab_title(&id, cx)
                .unwrap_or_else(|| module.map(|m| m.title).unwrap_or("Module").to_string());
            let icon = module.map(|m| m.icon.clone()).unwrap_or(IconName::Square);
            let selected = ix == group.active_ix();
            let select = group.clone();
            let focus = panel.focus_handle(cx);
            // Dispatch menu commands through the group's displayed panel: an inactive tab's
            // panel is not rendered, so its own handle is outside the dispatch tree and
            // "Hide module" on an inactive tab did nothing. The command names its module.
            let menu_focus = group
                .panels()
                .get(group.active_ix())
                .map(|active| active.focus_handle(cx))
                .unwrap_or_else(|| focus.clone());
            let primary_id = id.clone();
            // The active tab keeps its width; inactive ones shrink first (label truncates,
            // down to the icon), so a narrow pane never hides the module being shown.
            let tab = div()
                .id(("module-tab", ix))
                .h_full()
                .flex()
                .min_w(px(36.))
                .map(|tab| if selected { tab.flex_shrink_0() } else { tab.flex_shrink(1.).overflow_hidden() })
                .border_b_2()
                .border_color(if selected {
                    cx.theme().primary
                } else {
                    cx.theme().tab_bar
                })
                .child(
                    Button::new(("select", ix))
                        .label(name.clone())
                        .icon(icon.clone())
                        .ghost()
                        .min_w_0()
                        .max_w_full()
                        .selected(selected)
                        .tooltip(name.clone())
                        .on_click(move |_, w, cx| {
                            select.select_tab(ix, w, cx);
                            modules::focus_primary(&primary_id, w, cx);
                        }),
                )
                .child(ados_core::hints::mark());
            let tab = if let Some(drag) = group.drag_panel(ix, cx) {
                tab.on_drag(drag, move |_, _, _, cx| {
                    cx.new(|_| DragLabel {
                        name: name.clone(),
                        icon: icon.clone(),
                    })
                })
            } else {
                tab
            };
            // Base installs drop handling on the content region only, so a
            // tab bar built by a custom renderer has to accept the drop
            // itself or a tab dragged onto it snaps back.
            let tab = if droppable {
                let drop_group = group.clone();
                let item_group = group.clone();
                tab.drag_over::<DragPanel>(|this, _, _, cx| this.border_color(cx.theme().primary))
                    .on_drop(move |drag: &DragPanel, w: &mut Window, cx: &mut App| {
                        drop_group.drop_panel(drag.clone(), Some(ix), true, w, cx);
                    })
                    .on_drop(move |item: &AnyDrag, w: &mut Window, cx: &mut App| {
                        item_group.drop_item(item.clone(), None, w, cx);
                    })
            } else {
                tab
            };
            tabs = tabs.child(
                tab.context_menu(move |m, w, cx| menu(m.action_context(menu_focus.clone()), &id, w, cx)),
            );
        }
        // The empty strip past the last tab joins the group too.
        let empty = div()
            .id("tab-bar-empty")
            .h_full()
            .flex_grow_1()
            .min_w_0();
        let empty = if droppable {
            let drop_group = group.clone();
            let item_group = group.clone();
            empty
                .on_drop(move |drag: &DragPanel, w: &mut Window, cx: &mut App| {
                    drop_group.drop_panel(drag.clone(), None, true, w, cx);
                })
                .on_drop(move |item: &AnyDrag, w: &mut Window, cx: &mut App| {
                    item_group.drop_item(item.clone(), None, w, cx);
                })
        } else {
            empty
        };
        let mut bar = h_flex()
            .w_full()
            .min_w_0()
            .h(crate::settings_ui::zoomed(40., cx))
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().tab_bar)
            .child(tabs.child(empty));
        if let Some(panel) = group.active_panel() {
            let id = panel.panel_name(cx).to_string();
            let hide_id = id.clone();
            let menu_id = id.clone();
            let menu_focus = panel.focus_handle(cx);
            // Keep the ellipsis in view: the tab strip shrinks and clips, the
            // actions stay at their natural width. `...` is leftmost, so a
            // very narrow bar drops zoom and hide before it does the menu.
            let actions = h_flex()
                .flex_shrink_0()
                .h_full()
                .items_center()
                .child(
                    ados_core::hints::marked(Button::new("module-menu")
                        .icon(IconName::Ellipsis)
                        .ghost()
                        .tooltip("Layout")
                        .dropdown_menu(move |m, w, cx| {
                            menu(m.action_context(menu_focus.clone()), &menu_id, w, cx)
                        })),
                )
                .child(
                    ados_core::hints::marked(Button::new("zoom")
                        .icon(if group.is_zoomed() {
                            IconName::Minimize
                        } else {
                            IconName::Maximize
                        })
                        .ghost()
                        .tooltip("Zoom module · Shift+Esc")
                        .on_click(move |_, w, cx| w.dispatch_action(command(&id, "zoom"), cx))),
                )
                .child(
                    ados_core::hints::marked(Button::new("hide")
                        .icon(IconName::Close)
                        .ghost()
                        .tooltip("Hide module · Ctrl+W")
                        .on_click(move |_, w, cx| w.dispatch_action(command(&hide_id, "hide"), cx))),
                );
            bar = bar.child(actions);
        }
        bar.into_any_element()
    }
}
struct DragLabel {
    name: String,
    icon: IconName,
}
impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .px_3()
            .h(crate::settings_ui::zoomed(36., cx))
            .items_center()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().primary)
            .rounded_lg()
            .child(Icon::new(self.icon.clone()))
            .child(self.name.clone())
    }
}
