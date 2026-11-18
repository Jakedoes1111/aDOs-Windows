//! Isolated assistant fixture. ADOS_HOME must point at throwaway data for testing.
use ados_assistant::Assistant;
use ados_core::hub::{ContextHub, Snapshot};
use gpui_kit::component::{
    input::{Textarea, TextareaState},
    ActiveTheme, Root, Theme, ThemeMode,
};
use gpui_kit::*;

actions!(fixture, [ToggleAssistant, NewAssistantChat]);
struct Fixture {
    input: Entity<TextareaState>,
    assistant: Entity<Assistant>,
    focus: FocusHandle,
    _subscription: Subscription,
}
impl Fixture {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let input = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx).rows(12);
            state.set_value("The module test marker is COBALT-17.\n\nThis editable document is the active module. Select text to attach a Selection chip.\nClicks here must still work while the assistant card is open.", window, cx);
            state
        });
        let assistant = Assistant::view(window, cx);
        let subscription = cx.observe(&input, |this, _, cx| this.publish(cx));
        let fixture = Self {
            input,
            assistant,
            focus,
            _subscription: subscription,
        };
        fixture.publish(cx);
        ContextHub::publish(
            Snapshot {
                module: "reference".into(),
                title: "Reference module".into(),
                location: Some("fixture://reference".into()),
                text: "The reference marker is QUARTZ-23.".into(),
                selection: None,
            },
            cx,
        );
        fixture
    }
    fn publish(&self, cx: &mut App) {
        let input = self.input.read(cx);
        let selection = input.selected_text().to_string();
        let text = input.value();
        let selection = if selection.is_empty() {
            None
        } else {
            Some(SharedString::from(selection))
        };
        let changed = ContextHub::get("fixture", cx)
            .is_none_or(|s| s.text != text || s.selection != selection);
        if changed {
            ContextHub::publish(
                Snapshot {
                    module: "fixture".into(),
                    title: "Test document".into(),
                    location: Some("fixture://document".into()),
                    text,
                    selection,
                },
                cx,
            );
            ContextHub::set_active("fixture".into(), cx);
        }
    }
}
impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .tab_group()
            .on_action(cx.listener(|_, _: &ToggleAssistant, window, cx| {
                run_action("assistant.toggle", window, cx)
            }))
            .on_action(cx.listener(|_, _: &NewAssistantChat, window, cx| {
                run_action("assistant.new_chat", window, cx)
            }))
            .child(
                div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w(px(510.))
                    .child("Assistant test document")
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Alt+Space: toggle · Alt+N: new chat · @: attach context"),
                    )
                    .child(Textarea::new(&self.input).w_full()),
            )
            .child(self.assistant.clone())
    }
}
fn run_action(id: &str, window: &mut Window, cx: &mut App) {
    if let Some(action) = ados_core::actions::ActionRegistry::all(cx)
        .into_iter()
        .find(|a| a.id.as_ref() == id)
    {
        (action.run)(window, cx);
    }
}
fn midnight(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    let colors = &mut theme.colors;
    let canvas = rgb(0x030405).into();
    let panel = rgb(0x08090c).into();
    let fg = rgb(0xd4d7df).into();
    let muted = rgb(0x939ba8).into();
    let line = rgb(0x25282d).into();
    colors.background = canvas;
    colors.foreground = fg;
    colors.popover = panel;
    colors.popover_foreground = fg;
    colors.muted = panel;
    colors.muted_foreground = muted;
    colors.border = line;
    colors.input = line;
    colors.primary = rgb(0x376bfa).into();
    colors.primary_hover = colors.primary;
    colors.primary_active = colors.primary;
    colors.primary_foreground = fg;
    colors.ring = colors.primary;
    colors.selection = colors.primary.opacity(0.25);
    colors.danger = rgb(0xf04b55).into();
    colors.accent = line;
    colors.accent_foreground = fg;
    colors.input = panel;
    colors.caret = colors.primary;
    colors.list_hover = line;
    theme.font_size = px(14.);
    theme.radius = px(8.);
}
fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            ados_core::init(cx);
            ados_assistant::init(cx);
            midnight(cx);
            cx.bind_keys([
                KeyBinding::new("alt-space", ToggleAssistant, None),
                KeyBinding::new("alt-n", NewAssistantChat, None),
            ]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            if let Err(error) = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.), px(0.)),
                        size: size(px(1000.), px(760.)),
                    })),
                    window_min_size: Some(size(px(480.), px(360.))),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title("aDOs Assistant — standalone");
                    let view = cx.new(|cx| Fixture::new(window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            ) {
                eprintln!("Couldn't open assistant fixture: {error}");
            }
            cx.activate(true);
        });
}
