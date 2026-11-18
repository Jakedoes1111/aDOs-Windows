use ados_terminal::TerminalPanel;
use gpui_kit::component::{self, Root, Theme, ThemeMode};
use gpui_kit::*;

fn main() {
    gpui_kit::application().run(|cx: &mut App| {
        component::init(cx);
        ados_core::init(cx);
        ados_terminal::init(cx);
        // Opt-in smoke-test evidence; production panels do not log their contents.
        if std::env::var_os("ADOS_TERMINAL_TRACE").is_some() {
            let start = std::time::Instant::now();
            let mut last = String::new();
            cx.observe_global::<ados_core::hub::ContextHub>(move |cx| {
                if let Some(snapshot) = ados_core::hub::ContextHub::get("terminal", cx) {
                    if snapshot.text.as_ref() != last {
                        last = snapshot.text.to_string();
                        eprintln!(
                            "TERMINAL_SNAPSHOT {}",
                            serde_json::json!({
                                "elapsed_ms": start.elapsed().as_millis(),
                                "title": snapshot.title.as_ref(), "text": snapshot.text.as_ref()
                            })
                        );
                    }
                }
            })
            .detach();
        }
        Theme::change(ThemeMode::Dark, None, cx);
        let theme = cx.global_mut::<Theme>();
        theme.colors.background = rgb(0x030405).into();
        theme.colors.foreground = rgb(0xd4d7df).into();
        theme.colors.border = rgb(0x25282d).into();
        theme.colors.muted_foreground = rgb(0x939ba8).into();
        theme.colors.selection = rgb(0x244ba8).into();
        theme.colors.ring = rgb(0x376bfa).into();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(960.), px(640.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("aDOs Terminal — terminal3".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Err(error) = cx.open_window(options, |window, cx| {
            let panel = cx.new(|cx| TerminalPanel::new(window, cx));
            let focus = panel.focus_handle(cx);
            window.focus(&focus, cx);
            cx.new(|cx| Root::new(panel, window, cx))
        }) {
            eprintln!("Could not open terminal window: {error}");
            cx.quit();
        }
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
