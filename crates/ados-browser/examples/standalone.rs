use ados_browser::BrowserPanel;
use ados_core::overlay::OverlayGuard;
use gpui_kit::base::dock::Panel;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use gpui_kit::{
    assets::Assets,
    component::{ActiveTheme, Root, Theme, ThemeMode},
};

actions!(browser_example, [ToggleOverlay, TogglePanel, Quit]);
struct Standalone {
    browser: Entity<BrowserPanel>,
    focus: FocusHandle,
    overlay: bool,
    visible: bool,
    _hub_subscription: Subscription,
}
impl Render for Standalone {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .key_context("BrowserExample")
            .track_focus(&self.focus)
            .capture_key_down(|event, window, _| {
                if std::env::var_os("ADOS_BROWSER_TRACE_KEYS").is_some() {
                    eprintln!(
                        "key={:?} active={}",
                        event.keystroke,
                        window.is_window_active()
                    );
                }
            })
            .bg(cx.theme().background)
            .on_action(cx.listener(|this, _: &ToggleOverlay, _, cx| {
                this.overlay = !this.overlay;
                if this.overlay {
                    OverlayGuard::open(cx);
                } else {
                    OverlayGuard::close(cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TogglePanel, window, cx| {
                this.visible = !this.visible;
                this.browser
                    .update(cx, |panel, cx| panel.set_active(this.visible, window, cx));
                this.focus.focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &Quit, _, cx| cx.quit()))
            .when(self.visible, |el| el.child(self.browser.clone()))
            .when(self.overlay, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(cx.theme().background.opacity(0.95))
                        .child(
                            div()
                                .p_4()
                                .rounded_lg()
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().popover)
                                .child("Overlay stand-in — Ctrl+Shift+O closes"),
                        ),
                )
            })
    }
}
fn main() {
    gpui_kit::application().with_assets(Assets).run(|cx| {
        gpui_kit::init(cx);
        ados_core::init(cx);
        ados_browser::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        let theme = Theme::global_mut(cx);
        theme.colors.background = rgb(0x030405).into();
        theme.colors.secondary = rgb(0x08090C).into();
        theme.colors.popover = rgb(0x08090C).into();
        theme.colors.input = rgb(0x08090C).into();
        theme.colors.border = rgb(0x25282D).into();
        theme.colors.foreground = rgb(0xD4D7DF).into();
        theme.colors.muted_foreground = rgb(0x939BA8).into();
        theme.colors.primary = rgb(0x376BFA).into();
        theme.colors.selection = rgb(0x376BFA).into();
        cx.bind_keys([
            KeyBinding::new("f8", ToggleOverlay, Some("BrowserExample")),
            KeyBinding::new("alt-f8", ToggleOverlay, Some("BrowserExample")),
            KeyBinding::new("ctrl-shift-o", ToggleOverlay, Some("BrowserExample")),
            KeyBinding::new("f9", TogglePanel, Some("BrowserExample")),
            KeyBinding::new("ctrl-shift-h", TogglePanel, Some("BrowserExample")),
            KeyBinding::new("ctrl-q", Quit, None),
        ]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1100.), px(760.)),
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("aDOs Browser Test".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let browser = BrowserPanel::view(window, cx);
                let view = cx.new(|cx| Standalone {
                    browser,
                    focus: cx.focus_handle(),
                    overlay: false,
                    visible: true,
                    _hub_subscription: cx.observe_global::<ados_core::hub::ContextHub>(|_, cx| {
                        if let Some(page) =
                            ados_core::hub::ContextHub::get(ados_core::ids::BROWSER, cx)
                        {
                            eprintln!(
                                "ContextHub: title={:?} url={:?} text_chars={}",
                                page.title,
                                page.location,
                                page.text.chars().count()
                            );
                        }
                    }),
                });
                view.read(cx).focus.clone().focus(window, cx);
                cx.new(|cx| Root::new(view, window, cx))
            },
        );
        if let Err(error) = result {
            eprintln!("Could not open browser test window: {error}");
            cx.quit();
        }
    });
}
