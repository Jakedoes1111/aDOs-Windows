use std::rc::Rc;
use ados_core::{actions::{ActionRegistry, PaletteAction}, overlay::OverlayGuard};
use ados_palette::Palette;
use gpui_kit::*;
use gpui_kit::component::{ActiveTheme, Root, Theme, ThemeMode, input::{Input, InputState}};

actions!(palette_demo, [OpenActions, OpenCapture]);

#[derive(Clone)]
struct DemoPalette(Entity<Palette>);
impl Global for DemoPalette {}
struct Demo { palette: Entity<Palette>, editor: Entity<InputState> }
impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| InputState::new(window, cx).placeholder("Focus returns here after Escape or capture"));
        editor.update(cx, |input, cx| input.focus(window, cx));
        let palette = Palette::view(window, cx);
        cx.set_global(DemoPalette(palette.clone()));
        Self { palette, editor }
    }
}
impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().relative().size_full().bg(cx.theme().background).text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &OpenActions, window, cx| {
                this.palette.update(cx, |palette, cx| palette.open_actions(window, cx));
            }))
            .on_action(cx.listener(|this, _: &OpenCapture, window, cx| {
                this.palette.update(cx, |palette, cx| palette.open_capture(window, cx));
            }))
            .child(div().p_8().flex().flex_col().gap_4()
                .child("aDOs — palette standalone")
                .child("Ctrl+K · Actions     Ctrl+Shift+K · Capture")
                .child(Input::new(&self.editor)))
            .child(self.palette.clone())
    }
}

fn main() {
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(|cx| {
            gpui_kit::init(cx);
            ados_core::init(cx);
            ados_palette::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            {
                let theme = Theme::global_mut(cx);
                let c = &mut theme.colors;
                c.background = rgb(0x030405).into(); c.foreground = rgb(0xD4D7DF).into();
                c.popover = rgb(0x08090C).into(); c.border = rgb(0x25282D).into();
                c.muted_foreground = rgb(0x939BA8).into(); c.primary = rgb(0x376BFA).into();
                c.selection = rgba(0x376BFA30).into(); c.list_hover = rgb(0x10151D).into();
                c.input = rgb(0x08090C).into(); c.caret = rgb(0x376BFA).into();
            }
            // Register >8 representative actions to test N2 (8 rows + more hint)
            let actions: Vec<(&str, &str, &str, &str)> = vec![
                ("notes", "Open notes", "write markdown", "Ctrl+1"),
                ("terminal", "Open terminal", "shell command", "Ctrl+2"),
                ("browser", "Open browser", "web url", "Ctrl+3"),
                ("workspace", "Switch workspace", "project", "Ctrl+W"),
                ("settings", "Settings", "preferences appearance", "Ctrl+,"),
                ("inbox", "Open inbox", "capture thoughts", "Ctrl+I"),
                ("home", "Go home", "dashboard", "Ctrl+H"),
                ("search", "Search files", "find grep", "Ctrl+F"),
                ("command", "Run command", "execute task", "Ctrl+R"),
                ("debug", "Toggle debug", "inspect logs", "Ctrl+D"),
            ];
            for (id, title, keywords, shortcut) in actions {
                ActionRegistry::register(PaletteAction {
                    id: id.into(), title: title.into(), keywords: keywords.into(), shortcut: Some(shortcut.into()),
                    run: Rc::new(move |_, cx| {
                        eprintln!("ACTION {id}; overlay_open={}", OverlayGuard::any(cx));
                    }),
                }, cx);
            }
            // Deliberate panic action for testing N21 (gated by ADOS_TEST_PANIC=1)
            if std::env::var("ADOS_TEST_PANIC").is_ok() {
                ActionRegistry::register(PaletteAction {
                    id: "panic-test".into(),
                    title: "Panic test (N21)".into(),
                    keywords: "crash panic".into(),
                    shortcut: Some("Ctrl+P".into()),
                    run: Rc::new(move |window, cx| {
                        eprintln!("PANIC ACTION before run; overlay_open={}", OverlayGuard::any(cx));
                        // Exercise the harder case: a callback reopens before failing.
                        let palette = cx.global::<DemoPalette>().0.clone();
                        palette.update(cx, |palette, cx| palette.open_actions(window, cx));
                        panic!("Deliberate panic for N21 testing");
                    }),
                }, cx);
            }
            cx.bind_keys([
                KeyBinding::new("ctrl-k", OpenActions, None),
                KeyBinding::new("ctrl-shift-k", OpenCapture, None),
            ]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() { cx.quit(); }
            }).detach();
            let result = cx.open_window(WindowOptions {
                titlebar: Some(TitlebarOptions { title: Some("aDOs Palette standalone".into()), ..Default::default() }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1100.), px(760.)), cx))),
                window_decorations: Some(WindowDecorations::Server),
                ..Default::default()
            }, |window, cx| {
                let demo = cx.new(|cx| Demo::new(window, cx));
                cx.new(|cx| Root::new(demo, window, cx))
            });
            if let Err(error) = result { eprintln!("Could not open palette demo: {error}"); }
            cx.activate(true);
        });
}
