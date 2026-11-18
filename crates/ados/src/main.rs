mod actions;
mod keys;
mod modules;
mod shell;
mod settings;
mod settings_ui;
mod rail;
mod theme;
mod workspace;
use ados::storage;
mod dock_skin;
mod home;
mod hints;
mod layouts;
mod links;
mod page_actions;
mod overlay_focus;
#[cfg(feature = "mcp")]
mod mcp;
mod single;

use gpui_kit::component::Root;
use gpui_kit::*;

fn main() {
    #[cfg(feature = "mcp")]
    mcp::bridge_mode();
    // One aDOs per data folder: a second launch shows the running window and exits.
    let guard = match single::claim(&ados_core::paths::home()) {
        single::Start::First(guard) => guard,
        single::Start::HandedOff => return,
        single::Start::Busy => {
            eprintln!("aDOs is already running on this data folder but did not answer. Close it, then start aDOs again.");
            std::process::exit(1);
        }
    };
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            if let Some(guard) = guard {
                single::serve(guard, cx);
            }
            ados_core::init(cx);
            theme::install(cx);
            let settings_problem = ados_core::settings::init(cx);
            settings_ui::apply_zoom(cx);
            settings_ui::bind(cx, settings_problem);
            modules::register(cx);
            actions::init(cx);
            links::init(cx);
            page_actions::init(cx);
            hints::init(cx);
            #[cfg(feature = "mcp")]
            mcp::start(cx);
            // After every crate attached its actions: load keymap.toml and watch it.
            ados_core::shortcuts::start(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let result = cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("aDOs".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1280.), px(800.)),
                        cx,
                    ))),
                    window_min_size: Some(size(px(320.), px(320.))),
                    // Client borders lose their paint on resize even with xfwm4 compositing.
                    // Keep an explicit diagnostic opt-in; default to reliable native frames.
                    window_decorations: Some(
                        if std::env::var("ADOS_CLIENT_DECORATIONS").as_deref() == Ok("1") {
                            WindowDecorations::Client
                        } else {
                            WindowDecorations::Server
                        },
                    ),
                    ..Default::default()
                },
                |window, cx| {
                    let shell = cx.new(|cx| shell::Shell::new(window, cx));
                    cx.new(|cx| Root::new(shell, window, cx))
                },
            );
            if let Err(error) = result {
                eprintln!("Could not open aDOs: {error}");
            }
            cx.activate(true);
        });
}
