//! `browser_*` tools: thin glue from tool arguments to `ados_browser::control`, which does
//! the work (page scripts over the engine's Eval path, URL check, text shaping).
use std::time::Duration;

use ados_browser::control::{self, Op};
use ados_core::actions::ActionRegistry;
use gpui_kit::*;
use serde_json::{json, Value};

use crate::host::{toast, with_window};
use crate::protocol::ToolResult;

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}
fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// Tool arguments to the operation (pure, so it is tested).
fn op(tool: &str, args: &Value) -> Result<Op, String> {
    let element = || control::target(args.get("ref").and_then(Value::as_str), args.get("selector").and_then(Value::as_str));
    Ok(match tool {
        "browser_tabs" => Op::Tabs { select: args.get("select").and_then(Value::as_u64).map(|i| i as usize) },
        "browser_open" => Op::Open { url: text(args, "url").unwrap_or_default(), new_tab: flag(args, "new_tab") },
        "browser_read" => {
            let cursor = args.get("cursor").and_then(Value::as_i64).unwrap_or(0);
            if cursor < 0 {
                return Err("\"cursor\" cannot be negative.".into());
            }
            Op::Read { selector: text(args, "selector").filter(|s| !s.trim().is_empty()), cursor: cursor as usize }
        }
        "browser_click" => Op::Click { target: element()? },
        "browser_type" => Op::Type { target: element()?, text: text(args, "text").unwrap_or_default(), submit: flag(args, "submit") },
        "browser_selection" => Op::Selection,
        "browser_screenshot" => Op::Screenshot,
        other => return Err(format!("{other} is not implemented.")),
    })
}

pub async fn run(tool: &str, args: &Value, cx: &mut AsyncApp) -> ToolResult {
    let op = op(tool, args)?;
    let opening = matches!(op, Op::Open { .. });
    let tool = tool.to_string();
    let pending = with_window(cx, |window, cx| {
        let pending = control::start(op, window, cx)?;
        if opening {
            // Bring the Browser forward like the rail does.
            if let Some(action) = ActionRegistry::all(cx).into_iter().find(|a| a.id.as_ref() == "shell.focus.browser") {
                (action.run)(window, cx);
            }
        }
        Ok(pending)
    })?;
    let mut out = pending.await?;
    match tool.as_str() {
        "browser_open" => {
            // Let the page start loading, then report where the Browser is.
            cx.background_executor().timer(Duration::from_millis(1200)).await;
            if let Ok(tabs) = with_window(cx, |window, cx| control::start(Op::Tabs { select: None }, window, cx)) {
                if let Ok(tabs) = tabs.await {
                    out["tab"] = tabs["active"].clone();
                    let in_view = tabs["in_view"].as_bool().unwrap_or(false);
                    out["in_view"] = json!(in_view);
                    if !in_view {
                        let note = format!("{} The Browser could not be brought into view (the current layout may not have it), so the page loaded in the background.", out["note"].as_str().unwrap_or(""));
                        out["note"] = json!(note.trim());
                    }
                }
            }
            let url = out["opened"].as_str().unwrap_or("").to_string();
            let _ = with_window(cx, |window, cx| { toast(format!("opened {url} in the Browser"), None, window, cx); Ok(()) });
        }
        "browser_click" => {
            let what = out["text"].as_str().filter(|t| !t.is_empty()).unwrap_or("an element").to_string();
            let _ = with_window(cx, |window, cx| { toast(format!("clicked “{what}” in the Browser"), None, window, cx); Ok(()) });
        }
        "browser_type" => {
            let _ = with_window(cx, |window, cx| { toast("typed into the Browser page".into(), None, window, cx); Ok(()) });
        }
        _ => {}
    }
    Ok(json!(out))
}

#[cfg(test)]
mod tests {
    use super::{op, Op};
    use crate::tools;
    use serde_json::json;

    #[test]
    fn arguments_become_operations() {
        assert!(matches!(op("browser_tabs", &json!({})), Ok(Op::Tabs { select: None })));
        assert!(matches!(op("browser_open", &json!({"url": "https://a.b", "new_tab": true})), Ok(Op::Open { new_tab: true, .. })));
        assert!(matches!(op("browser_read", &json!({"cursor": 7})), Ok(Op::Read { cursor: 7, selector: None })));
        assert!(op("browser_read", &json!({"cursor": -1})).is_err());
        assert!(matches!(op("browser_type", &json!({"text": "hi", "ref": "e2", "submit": true})), Ok(Op::Type { submit: true, .. })));
        assert!(op("browser_click", &json!({})).unwrap_err().contains("ref"));
        assert!(op("browser_click", &json!({"ref": "nope"})).is_err());
    }

    #[test]
    fn browser_tools_are_listed_and_mutations_marked() {
        for name in ["browser_tabs", "browser_open", "browser_read", "browser_click", "browser_type", "browser_selection", "browser_screenshot"] {
            assert!(tools::find(name).is_some(), "{name}");
        }
        for (name, mutates) in [("browser_open", true), ("browser_click", true), ("browser_type", true), ("browser_read", false), ("browser_selection", false), ("browser_screenshot", false)] {
            assert_eq!(tools::find(name).map(|t| t.mutates), Some(mutates), "{name}");
        }
        let t = tools::find("browser_type").expect("tool");
        assert_eq!(tools::validate(t, &json!({"ref": "e1"})), Err("browser_type needs \"text\".".into()));
        assert!(tools::validate(t, &json!({"ref": "e1", "text": "x", "submit": true})).is_ok());
    }
}
