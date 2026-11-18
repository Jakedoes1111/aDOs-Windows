use crate::actions;
use ados_core::hub::{ContextHub, Snapshot};
use gpui_kit::base::dock::{self as base};
use gpui_kit::base::Selectable;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    dock::Panel,
    h_flex, v_flex, ActiveTheme, Icon, IconName,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
#[derive(Default, PartialEq)]
pub(crate) struct HomeInfo { pub layout: String, pub only_home: bool }
impl Global for HomeInfo {}

pub struct HomePanel {
    _info: Subscription,
    focus: FocusHandle,
    /// Tracked on the Home root, so focus on any Home control is inside it.
    root: FocusHandle,
}
impl HomePanel {
    pub fn view(_: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            _info: cx.observe_global::<HomeInfo>(|_, cx| cx.notify()),
            focus: cx.focus_handle(),
            root: cx.focus_handle(),
        })
    }
}
impl HomePanel {
    pub fn root_focus(&self) -> FocusHandle {
        self.root.clone()
    }
    pub fn focus_primary(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }
}
#[cfg(feature = "notes")]
fn recent_notes() -> Vec<(String, std::path::PathBuf)> { ados_notes::recent(5) }
#[cfg(not(feature = "notes"))]
fn recent_notes() -> Vec<(String, std::path::PathBuf)> { Vec::new() }
impl Focusable for HomePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl EventEmitter<base::PanelEvent> for HomePanel {}
impl base::Panel for HomePanel {
    fn panel_name(&self) -> &'static str {
        ados_core::ids::HOME
    }
}
impl Panel for HomePanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child("Home")
    }
}
impl Render for HomePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let captures = ados_core::capture::list();
        let notes = recent_notes();
        let only_home = cx.try_global::<HomeInfo>().is_some_and(|s| s.only_home);
        let current_layout = cx.try_global::<HomeInfo>().map(|s| s.layout.clone()).unwrap_or_default();
        // Publish exactly the recent items this Home view lists, not hidden file contents.
        let text = std::iter::once("Recent notes".to_string())
            .chain(
                notes.iter().map(|(title, _)| title.clone()),
            )
            .chain(std::iter::once("Recent captures".to_string()))
            .chain(
                captures
                    .iter()
                    .take(5)
                    .map(|c| c.text.chars().take(140).collect::<String>()),
            )
            .collect::<Vec<_>>()
            .join("\n");
        if ContextHub::get("home", cx).is_none_or(|s| s.text.as_ref() != text) {
            ContextHub::publish(
                Snapshot {
                    module: "home".into(),
                    title: "Home".into(),
                    text: text.into(),
                    ..Default::default()
                },
                cx,
            );
        }
        v_flex()
            .id("home-content")
            .track_focus(&self.root)
            .size_full()
            .overflow_y_scroll()
            .capture_any_mouse_down(cx.listener(|s, _, w, cx| s.focus_primary(w, cx)))
            .bg(cx.theme().background)
            .p_8()
            .gap_6()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(Icon::new(IconName::LayoutDashboard).size_6())
                    .child(div().text_xl().child("Home")),
            )
            .child(
                h_flex()
                    .gap_3()
                    .when(only_home, |el| el.child(
                        Button::new("resume")
                            .track_focus(&self.focus)
                            .label("Resume last layout")
                            .icon(IconName::ArrowRight)
                            .primary()
                            .on_click(|_, w, cx| {
                                w.dispatch_action(Box::new(actions::ResumeLayout), cx)
                            }),
                    ))
                    .child(
                        Button::new("quick-capture")
                            .when(!only_home, |button| button.track_focus(&self.focus))
                            .label("Quick capture")
                            .icon(IconName::Plus)
                            .ghost()
                            .on_click(|_, w, cx| {
                                w.dispatch_action(Box::new(actions::OpenCapture), cx)
                            }),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("GET AROUND"),
            )
            .child(h_flex().gap_2().flex_wrap().children(get_around(cx)))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("LAYOUTS"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .children(crate::storage::names().into_iter().map(|name| {
                        div().rounded(px(8.)).border_1()
                            .border_color(if name == current_layout { cx.theme().primary } else { cx.theme().border })
                            .child(Button::new(SharedString::from(format!("home-layout-{name}")))
                            .label(name.clone())
                            .icon(IconName::PanelLeft)
                            .outline()
                            .selected(name == current_layout)
                            .on_click(move |_, w, cx| {
                                w.dispatch_action(
                                    Box::new(actions::SwitchLayout { name: name.clone() }),
                                    cx,
                                )
                            }))
                    })),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("RECENT NOTES"),
            )
            .when(notes.is_empty(), |el| {
                el.child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child("No notes yet. Open Notes to start one."),
                )
            })
            .children(notes.iter().enumerate().map(|(ix, (name, _))| {
                let name = name.clone();
                Button::new(("recent-note", ix))
                    .accessibility_label(name.clone())
                    .child(div().flex().items_center().gap_2().w_full().min_w_0()
                        .child(Icon::new(IconName::FileText))
                        .child(div().min_w_0().truncate().child(name)))
                    .w_full()
                    .ghost()
                    .on_click(|_, w, cx| {
                        w.dispatch_action(Box::new(actions::FocusModule { id: "notes".into() }), cx)
                    })
            }))
            .child(
                div()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt_5()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("RECENT CAPTURES"),
            )
            .when(captures.is_empty(), |el| {
                el.child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child("Nothing captured yet. Ctrl+Shift+K captures a thought."),
                )
            })
            .children(captures.into_iter().take(5).map(|capture| {
                let preview: String = capture.text.chars().take(140).collect();
                h_flex()
                    .gap_3()
                    .py_2()
                    .child(Icon::new(IconName::Inbox))
                    .child(preview)
            }))
    }
}

/// Where to go from Home: Tasks, Calendar, Sessions, the assistant and the palette, each
/// with its live shortcut (the owner may have changed it).
fn get_around(cx: &App) -> Vec<AnyElement> {
    let modules: [(&str, &str, IconName); 3] = [
        ("tasks", "Tasks", IconName::CircleCheck),
        ("calendar", "Calendar", IconName::Calendar),
        ("sessions", "Sessions", IconName::GalleryVerticalEnd),
    ];
    let label = |title: &str, id: &str| match ados_core::shortcuts::label_for(id, cx) {
        Some(chord) if !chord.is_empty() => format!("{title} · {chord}"),
        _ => title.to_string(),
    };
    let mut out: Vec<AnyElement> = modules
        .into_iter()
        .map(|(id, title, icon)| {
            Button::new(SharedString::from(format!("home-go-{id}")))
                .label(label(title, &format!("shell.focus.{id}")))
                .icon(icon)
                .outline()
                .on_click(move |_, w, cx| {
                    w.dispatch_action(Box::new(actions::FocusModule { id: id.into() }), cx)
                })
                .into_any_element()
        })
        .collect();
    out.push(
        Button::new("home-go-assistant")
            .label(label("Assistant", "assistant.toggle"))
            .icon(IconName::Bot)
            .outline()
            .on_click(|_, w, cx| w.dispatch_action(Box::new(actions::ToggleAssistant), cx))
            .into_any_element(),
    );
    out.push(
        Button::new("home-go-palette")
            .label(label("Everything else", "shell.palette"))
            .icon(IconName::Search)
            .outline()
            .on_click(|_, w, cx| w.dispatch_action(Box::new(actions::OpenPalette), cx))
            .into_any_element(),
    );
    out
}
