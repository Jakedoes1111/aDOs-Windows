use ados_notes::NotesPanel;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

fn main() {
    gpui_kit::application().with_assets(gpui_kit::assets::AllAssets).run(|cx| {
        gpui_kit::init(cx);
        ados_core::init(cx);
        ados_notes::init(cx);
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
                title: Some("aDOs Notes".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        match gpui_kit::open_window(options, cx, |window, cx| {
            window.on_window_should_close(cx, |_, cx| {
                cx.quit();
                true
            });
            let panel = cx.new(|cx| NotesPanel::new(window, cx));
            if std::env::var_os("ADOS_NOTES_DIAGNOSTICS").is_some() {
                eprintln!("RECENT {:?}", ados_notes::recent(2));
                eprintln!("HUB {:?}", ados_core::hub::ContextHub::get(ados_core::ids::NOTES, cx));
                for action in ados_core::actions::ActionRegistry::all(cx) {
                    if action.id.starts_with("notes.") { eprintln!("ACTION {} {}", action.id, action.title); }
                }
            }
            panel.read(cx).focus_handle(cx).focus(window, cx);
            panel
        }) {
            Ok(_) => cx.activate(true),
            Err(e) => {
                eprintln!("Could not open the notes window: {e:#}");
                cx.quit();
            }
        }
    });
}
