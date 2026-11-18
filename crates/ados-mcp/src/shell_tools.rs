//! Tools over the shell: palette actions, ContextHub snapshots, layouts.
use std::rc::Rc;
use std::time::Duration;

use ados_core::actions::{ActionRegistry, PaletteAction};
use ados_core::hub::{ContextHub, Snapshot};
use gpui_kit::*;
use serde_json::{json, Value};

use crate::host::{toast, with_window, HostHooks};
use crate::protocol::ToolResult;

const LAYOUT_PREFIX: &str = "shell.layout.";

pub fn actions_list(cx: &App) -> ToolResult {
    let list: Vec<Value> = ActionRegistry::all(cx)
        .iter()
        .map(|a| json!({ "id": a.id.as_ref(), "title": a.title.as_ref(), "keywords": a.keywords.as_ref() }))
        .collect();
    Ok(json!(list))
}

fn find(id: &str, cx: &App) -> Option<PaletteAction> {
    ActionRegistry::all(cx).into_iter().find(|a| a.id.as_ref() == id)
}

/// (layout, focused module) as the owner sees them.
fn where_am_i(cx: &App) -> (Option<String>, Option<String>) {
    let layout = cx.try_global::<HostHooks>().and_then(|h| (h.0.current_layout)());
    (layout, ContextHub::active(cx).map(|s| s.module.to_string()))
}

/// Effects of an action land a frame or two later (deferred focus, layout loads).
async fn settle(cx: &mut AsyncApp) {
    cx.background_executor().timer(Duration::from_millis(250)).await;
}

fn changes(before: (Option<String>, Option<String>), after: (Option<String>, Option<String>)) -> Value {
    json!({
        "layout": { "before": before.0, "after": after.0, "changed": before.0 != after.0 },
        "focused_module": { "before": before.1, "after": after.1, "changed": before.1 != after.1 },
    })
}

pub async fn run_action(id: String, cx: &mut AsyncApp) -> ToolResult {
    let before = with_window(cx, |_, cx| Ok(where_am_i(cx)))?;
    let title = with_window(cx, |window, cx| {
        let action = find(&id, cx).ok_or_else(|| format!("No aDOs action has the id \"{id}\". Use actions_list."))?;
        (action.run)(window, cx);
        toast(format!("ran “{}”", action.title), None, window, cx);
        Ok(action.title.to_string())
    })?;
    settle(cx).await;
    let after = with_window(cx, |_, cx| Ok(where_am_i(cx)))?;
    let mut out = changes(before, after);
    out["ran"] = json!({ "id": id, "title": title });
    Ok(out)
}

fn layout_names(cx: &App) -> Vec<String> {
    ActionRegistry::all(cx)
        .iter()
        .filter_map(|a| a.id.strip_prefix(LAYOUT_PREFIX).map(str::to_string))
        .collect()
}

pub fn layouts_list(cx: &App) -> ToolResult {
    Ok(json!({ "layouts": layout_names(cx), "current": where_am_i(cx).0 }))
}

fn run_id(id: &str, window: &mut Window, cx: &mut App) -> Result<(), String> {
    let action = find(id, cx).ok_or_else(|| format!("aDOs has no action {id}."))?;
    (action.run)(window, cx);
    Ok(())
}

pub async fn switch_layout(name: String, cx: &mut AsyncApp) -> ToolResult {
    let before = with_window(cx, |_, cx| Ok(where_am_i(cx)))?;
    with_window(cx, |window, cx| {
        let names = layout_names(cx);
        if !names.contains(&name) {
            return Err(format!("There is no layout called \"{name}\". Layouts: {}.", names.join(", ")));
        }
        run_id(&format!("{LAYOUT_PREFIX}{name}"), window, cx)
    })?;
    settle(cx).await;
    let after = with_window(cx, |_, cx| Ok(where_am_i(cx)))?;
    // Only a real switch is news: switching to the layout already shown says nothing.
    if after.0 != before.0 {
        let prev = before.0.clone();
        with_window(cx, |window, cx| {
            let undo = prev.map(|prev| {
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    let _ = run_id(&format!("{LAYOUT_PREFIX}{prev}"), window, cx);
                }) as crate::host::Undo
            });
            toast(format!("switched to layout “{name}”"), undo, window, cx);
            Ok(())
        })?;
    }
    Ok(changes(before, after))
}

pub async fn save_layout(cx: &mut AsyncApp) -> ToolResult {
    let name = with_window(cx, |window, cx| {
        run_id("shell.save", window, cx)?;
        let name = where_am_i(cx).0.unwrap_or_else(|| "the current layout".into());
        toast(format!("saved layout “{name}”"), None, window, cx);
        Ok(name)
    })?;
    settle(cx).await;
    Ok(json!({ "saved": name }))
}

fn snapshot(s: &Snapshot) -> Value {
    json!({
        "module": s.module.as_ref(),
        "title": s.title.as_ref(),
        "location": s.location.as_ref().map(|l| l.to_string()),
        "text": s.text.as_ref(),
        "selection": s.selection.as_ref().map(|l| l.to_string()),
    })
}

pub fn context_active(cx: &App) -> ToolResult {
    Ok(ContextHub::active(cx).map(|s| snapshot(&s)).unwrap_or_else(|| json!({ "module": null, "note": "No module has reported what it shows yet." })))
}

pub fn context_module(id: &str, cx: &App) -> ToolResult {
    ContextHub::get(id, cx).map(|s| snapshot(&s)).ok_or_else(|| {
        let open: Vec<String> = ContextHub::all(cx).iter().map(|s| s.module.to_string()).collect();
        format!("Module \"{id}\" has not reported anything. Modules with context: {}.", open.join(", "))
    })
}

pub fn context_all(cx: &App) -> ToolResult {
    let active = ContextHub::active(cx).map(|s| s.module.to_string());
    let modules: Vec<Value> = ContextHub::all(cx).iter().map(snapshot).collect();
    Ok(json!({ "active": active, "modules": modules }))
}
