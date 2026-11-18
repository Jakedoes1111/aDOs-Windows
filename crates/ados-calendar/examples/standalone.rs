use ados_calendar::CalendarPanel;
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::*;

fn main() {
    gpui_kit::application().with_assets(gpui_kit::assets::AllAssets).run(|cx| {
        gpui_kit::init(cx);
        ados_core::init(cx);
        ados_calendar::init(cx);
        // The shell normally supplies these tokens. The isolated example does too.
        Theme::change(ThemeMode::Dark, None, cx);
        Theme::update(cx, |t| {
            t.background = rgb(0x030405).into();
            t.sidebar = rgb(0x08090C).into();
            t.popover = t.sidebar;
            t.muted = t.sidebar;
            t.foreground = rgb(0xD4D7DF).into();
            t.popover_foreground = t.foreground;
            t.sidebar_foreground = t.foreground;
            t.secondary_foreground = t.foreground;
            t.muted_foreground = rgb(0x939BA8).into();
            t.border = rgb(0x25282D).into();
            t.input = t.border;
            t.primary = rgb(0x376BFA).into();
            t.ring = t.primary;
            t.selection = t.primary.opacity(0.3);
            t.accent = t.primary.opacity(0.2);
            t.accent_foreground = t.foreground;
            t.danger = rgb(0xF04B55).into();
            t.radius = px(8.);
            t.radius_lg = px(10.);
            t.shadow = false;
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1000.), px(700.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("aDOs Calendar".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        match gpui_kit::open_window(options, cx, |window, cx| {
            window.on_window_should_close(cx, |_, cx| {
                cx.quit();
                true
            });
            let panel = cx.new(|cx| CalendarPanel::new(window, cx));
            if std::env::var_os("ADOS_CALENDAR_DIAGNOSTICS").is_some() {
                eprintln!("HUB {:?}", ados_core::hub::ContextHub::get(ados_core::ids::CALENDAR, cx));
            }
            panel.update(cx, |p, cx| p.focus_primary(window, cx));
            cx.new(|cx| Root::new(panel, window, cx))
        }) {
            Ok(_) => cx.activate(true),
            Err(e) => {
                eprintln!("Could not open the calendar window: {e:#}");
                cx.quit();
            }
        }
    });
}
