//! One script panel in a window. `standalone [name]` (default `hello`) shows
//! `ADOS_HOME/ui/<name>/`, seeding it from `fixtures/<name>/` when it does not exist.
//! It also registers `demo.ping`, an action that publishes context, so the sample's
//! button and live context text have something real to drive.
use ados_core::hub::{ContextHub, Snapshot};
use ados_script::ScriptPanel;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use std::path::Path;

fn seed(name: &str) -> Result<(), String> {
    let dir = ScriptPanel::dir_for(name);
    if dir.join("main.js").exists() { return Ok(()); }
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for file in ["main.js", "panel.json"] {
        if src.join(file).exists() {
            std::fs::copy(src.join(file), dir.join(file)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Copy of the shell's `crates/ados/src/theme.rs` so the trial looks and idles like the shell.
fn install_theme(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |t| {
        // The default caret blinks every 500 ms forever: a timer repainting an idle window
        // (about 45% CPU idle under software GL with the address bar focused). Blink while
        // the user works, then hold the caret steady after 10 quiet seconds.
        t.cursor_blink = gpui_kit::base::input::CursorBlink::blinking(std::time::Duration::from_millis(500))
            .with_idle_timeout(std::time::Duration::from_secs(10));
        let canvas = rgb(0x030405).into();
        let panel = rgb(0x08090C).into();
        let border = rgb(0x25282D).into();
        let text = rgb(0xD4D7DF).into();
        let muted = rgb(0x939BA8).into();
        let blue = rgb(ados_core::brand::BLUE).into();
        let red = rgb(ados_core::brand::RED).into();
        let c = &mut t.colors;
        c.background = canvas;
        c.foreground = text;
        c.border = border;
        c.window_border = border;
        c.sidebar = panel;
        c.sidebar_border = border;
        c.sidebar_foreground = muted;
        c.sidebar_accent = border;
        c.sidebar_accent_foreground = text;
        c.sidebar_primary = blue;
        c.sidebar_primary_foreground = text;
        c.tab = panel;
        c.tab_bar = panel;
        c.tab_active = canvas;
        c.tab_active_foreground = text;
        c.tab_foreground = muted;
        c.title_bar = panel;
        c.title_bar_border = border;
        c.status_bar = panel;
        c.status_bar_border = border;
        c.muted = panel;
        c.muted_foreground = muted;
        c.popover = panel;
        c.popover_foreground = text;
        c.input = canvas;
        c.list = panel;
        c.list_hover = border;
        c.list_active = blue;
        c.list_active_border = blue;
        c.list_head = panel;
        c.button = panel;
        c.button_foreground = text;
        c.button_hover = border;
        c.button_active = border;
        c.primary = blue;
        c.primary_hover = blue;
        c.primary_active = blue;
        c.primary_foreground = text;
        c.button_primary = blue;
        c.button_primary_hover = blue;
        c.button_primary_active = blue;
        c.button_primary_foreground = text;
        c.accent = border;
        c.accent_foreground = text;
        c.secondary = panel;
        c.secondary_foreground = text;
        c.ring = blue;
        c.selection = blue.opacity(0.3);
        c.drag_border = blue;
        c.drop_target = blue.opacity(0.15);
        c.caret = blue;
        c.link = blue;
        c.danger = red;
        c.button_danger = red;
        c.scrollbar = panel;
        c.scrollbar_thumb = border;
        c.scrollbar_thumb_hover = muted;
        t.radius = px(8.);
        t.radius_lg = px(10.);
        t.shadow = false;
        // PopupMenu rows are 1.625rem in gpui-kit 0.9.1; 20px keeps them above 32px.
        t.font_size = px(20.);
    });
}

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "hello".into());
    if let Err(e) = seed(&name) { eprintln!("Could not set up the {name} panel: {e}"); }
    gpui_kit::application().with_assets(gpui_kit::assets::AllAssets).run(move |cx| {
        gpui_kit::init(cx);
        ados_core::init(cx);
        ados_script::init(cx);
        install_theme(cx);
        let pings = std::rc::Rc::new(std::cell::Cell::new(0u32));
        ContextHub::publish(Snapshot { module: "demo".into(), title: "Demo".into(), text: "no pings yet".into(), ..Default::default() }, cx);
        ContextHub::set_active("demo", cx);
        ados_core::actions::register("demo.ping", "Demo ping", "test", None, cx, move |_, cx| {
            pings.set(pings.get() + 1);
            ContextHub::publish(Snapshot {
                module: "demo".into(), title: "Demo".into(),
                text: format!("{} ping(s)", pings.get()).into(), ..Default::default()
            }, cx);
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(900.), px(600.)), cx))),
            titlebar: Some(TitlebarOptions { title: Some("aDOs Script".into()), ..Default::default() }),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, move |window, cx| {
            window.on_window_should_close(cx, |_, cx| { cx.quit(); true });
            ScriptPanel::view(&name, window, cx)
        });
        match opened {
            Ok(_) => cx.activate(true),
            Err(e) => { eprintln!("Could not open the script window: {e:#}"); cx.quit(); }
        }
    });
}
