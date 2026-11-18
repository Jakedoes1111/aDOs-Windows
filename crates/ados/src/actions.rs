use crate::{modules, storage};
use ados_core::actions::{ActionRegistry, PaletteAction};
use gpui_kit::*;
use serde::Deserialize;

actions!(
    ados,
    [
        OpenPalette,
        OpenCapture,
        ToggleAssistant,
        NewAssistantChat,
        RevertLayout,
        ResetLayouts,
        OpenDataFolder,
        About,
        ReloadBrowser,
        BrowserAddress,
        SaveLayout,
        SaveLayoutAs,
        CloseModule,
        ZoomModule,
        UndoLast,
        ResumeLayout,
        EditShortcuts,
        ShowShortcutConflicts
    ]
);
#[derive(Action, Clone, PartialEq, Eq, Deserialize)]
#[action(namespace = ados, no_json)]
pub struct SwitchLayout {
    pub name: String,
}
#[derive(Action, Clone, PartialEq, Eq, Deserialize)]
#[action(namespace = ados, no_json)]
pub struct FocusModule {
    pub id: String,
}

pub fn init(cx: &mut App) {
    use ados_core::shortcuts::bind;
    bind(cx, "shell.palette", OpenPalette);
    bind(cx, "shell.capture", OpenCapture);
    bind(cx, "shell.shortcuts", EditShortcuts);
    bind(cx, "assistant.toggle", ToggleAssistant);
    bind(cx, "assistant.new_chat", NewAssistantChat);
    for module in modules::MODULES {
        bind(cx, &format!("shell.focus.{}", module.id), FocusModule { id: module.id.into() });
    }
    bind(cx, "browser.focus_address", BrowserAddress);
    bind(cx, "shell.close", CloseModule);
    bind(cx, "shell.undo", UndoLast);
    bind(cx, "shell.zoom", ZoomModule);
    bind(cx, "shell.save", SaveLayout);
    bind(cx, "shell.save-as", SaveLayoutAs);
    bind(cx, "shell.revert", RevertLayout);
    register_layouts(cx);
    for module in modules::MODULES {
        let id = module.id.to_string();
        ActionRegistry::register(
            PaletteAction {
                id: format!("shell.focus.{}", module.id).into(),
                title: format!("Open {}", module.title).into(),
                keywords: "module focus".into(),
                shortcut: None,
                run: std::rc::Rc::new(move |window, cx| {
                    window.dispatch_action(Box::new(FocusModule { id: id.clone() }), cx)
                }),
            },
            cx,
        );
    }
    type Factory = fn() -> Box<dyn Action>;
    // Shortcut labels come from the shortcut registry (same ids), so they are always live.
    for (id, label, action) in [
        ("assistant.toggle", "Toggle assistant", (|| Box::new(ToggleAssistant)) as Factory),
        ("assistant.new_chat", "New assistant chat", (|| Box::new(NewAssistantChat)) as Factory),
        ("shell.capture", "Quick capture", (|| Box::new(OpenCapture)) as Factory),
        ("shell.shortcuts", "Edit shortcuts", (|| Box::new(EditShortcuts)) as Factory),
        ("shell.revert", "Revert layout to saved", (|| Box::new(RevertLayout)) as Factory),
        ("browser.reload", "Reload page", (|| Box::new(ReloadBrowser)) as Factory),
        ("shell.save", "Save layout", (|| Box::new(SaveLayout)) as Factory),
        ("shell.save-as", "Save as new layout", (|| Box::new(SaveLayoutAs)) as Factory),
        ("shell.undo", "Undo last close/delete", (|| Box::new(UndoLast)) as Factory),
        ("shell.close", "Hide module", (|| Box::new(CloseModule)) as Factory),
        ("shell.zoom", "Zoom module", (|| Box::new(ZoomModule)) as Factory),
    ] {
        ActionRegistry::register(
            PaletteAction {
                id: id.into(),
                title: label.into(),
                keywords: if id == "shell.shortcuts" { "keyboard keys keybindings hotkeys keymap settings".into() } else { "workspace layout".into() },
                shortcut: None,
                run: std::rc::Rc::new(move |window, cx| window.dispatch_action(action(), cx)),
            },
            cx,
        );
    }
}
pub fn register_layouts(cx: &mut App) {
    for name in storage::names() {
        let label = format!("Switch layout: {name}");
        ActionRegistry::register(
            PaletteAction {
                id: format!("shell.layout.{name}").into(),
                title: label.into(),
                keywords: "switch layout workspace".into(),
                shortcut: None,
                run: std::rc::Rc::new(move |window, cx| {
                    window.dispatch_action(Box::new(SwitchLayout { name: name.clone() }), cx)
                }),
            },
            cx,
        );
    }
}
