//! The one host module a script panel can import: `import { ... } from "ados"`.
//!
//! - `context()` → the active module's snapshot, or null. `contexts()` → all of them.
//! - `actions()` → `[{ id, title, shortcut }]`. `run_action(id)` → true, or throws when
//!   the panel was not allowed to run actions or the id is unknown.
//! - `storage_get(key)` / `storage_set(key, value)` → the panel's `state.json`, so what the
//!   script keeps there survives a reload and a relaunch.
use ados_core::{actions::ActionRegistry, hub::{ContextHub, Snapshot}};
use gpui_kit::AnyWindowHandle;
use gpui_shell::{with_current_app, HostArguments, HostError, HostModule, HostObject, HostValue};
use serde_json::{Map, Value};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

pub const MODULE: &str = "ados";

pub fn module(state: Rc<Storage>, may_run_actions: bool, window: AnyWindowHandle) -> HostModule {
    let get = state.clone();
    let set = state;
    HostModule::new(MODULE)
        .function("context", |_| {
            Ok(with_current_app(|cx| ContextHub::active(cx)).flatten().map(snapshot).unwrap_or(HostValue::Null))
        })
        .function("contexts", |_| {
            let all = with_current_app(|cx| ContextHub::all(cx)).unwrap_or_default();
            Ok(HostValue::Array(all.into_iter().map(snapshot).collect()))
        })
        .function("actions", |_| {
            let all = with_current_app(|cx| ActionRegistry::all(cx)).unwrap_or_default();
            Ok(HostValue::Array(all.into_iter().map(|a| HostObject::new()
                .field("id", a.id.to_string())
                .field("title", a.title.to_string())
                .field("shortcut", a.shortcut.map(|s| s.to_string()))
                .into()).collect()))
        })
        .function("run_action", move |args| run_action(args, may_run_actions, window))
        .function("storage_get", move |args| {
            Ok(get.get(args.string(0)?).map(from_json).unwrap_or(HostValue::Null))
        })
        .function("storage_set", move |args| {
            set.set(args.string(0)?, to_json(args.value(1)?))?;
            Ok(HostValue::Bool(true))
        })
}

fn run_action(args: &HostArguments, allowed: bool, window: AnyWindowHandle) -> Result<HostValue, HostError> {
    if !allowed {
        return Err(HostError::new("This panel is not allowed to run aDOs actions."));
    }
    let id = args.string(0)?.to_string();
    let found = with_current_app(|cx| {
        let Some(action) = ActionRegistry::all(cx).into_iter().find(|a| a.id.as_ref() == id) else {
            return false;
        };
        // Deferred: an action may open overlays or publish context, which must not happen
        // while the script engine is still on the stack.
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| (action.run)(window, cx));
        });
        true
    });
    match found {
        Some(true) => Ok(HostValue::Bool(true)),
        Some(false) => Err(HostError::new(format!("There is no action called {id}."))),
        None => Err(HostError::new("Actions can only run from an event handler.")),
    }
}

fn snapshot(s: Snapshot) -> HostValue {
    HostObject::new()
        .field("module", s.module.to_string())
        .field("title", s.title.to_string())
        .field("location", s.location.map(|l| l.to_string()))
        .field("text", s.text.to_string())
        .field("selection", s.selection.map(|l| l.to_string()))
        .into()
}

/// `state.json` beside the script. Written through on every set, atomically.
pub struct Storage {
    path: PathBuf,
    data: RefCell<Map<String, Value>>,
}

impl Storage {
    /// A missing file is an empty store. A broken one is reported and not overwritten.
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let data = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("state.json is damaged and was left alone: {e}."))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(format!("state.json could not be opened: {e}.")),
        };
        Ok(Self { path, data: RefCell::new(data) })
    }

    pub fn get(&self, key: &str) -> Option<Value> { self.data.borrow().get(key).cloned() }

    pub fn set(&self, key: &str, value: Value) -> Result<(), HostError> {
        let mut data = self.data.borrow_mut();
        if data.get(key) == Some(&value) { return Ok(()); }
        data.insert(key.to_string(), value);
        let text = serde_json::to_string_pretty(&*data).map_err(|e| HostError::new(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text)
            .and_then(|_| std::fs::rename(&tmp, &self.path))
            .map_err(|e| HostError::new(format!("The panel state could not be saved: {e}.")))
    }
}

pub fn to_json(v: &HostValue) -> Value {
    match v {
        HostValue::Null => Value::Null,
        HostValue::Bool(b) => Value::Bool(*b),
        HostValue::Number(n) => serde_json::Number::from_f64(*n).map(Value::Number).unwrap_or(Value::Null),
        HostValue::Str(s) => Value::String(s.clone()),
        HostValue::Array(a) => Value::Array(a.iter().map(to_json).collect()),
        HostValue::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), to_json(v))).collect()),
    }
}

pub fn from_json(v: Value) -> HostValue {
    match v {
        Value::Null => HostValue::Null,
        Value::Bool(b) => HostValue::Bool(b),
        Value::Number(n) => HostValue::Number(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => HostValue::Str(s),
        Value::Array(a) => HostValue::Array(a.into_iter().map(from_json).collect()),
        Value::Object(o) => HostValue::Object(o.into_iter().map(|(k, v)| (k, from_json(v))).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip() {
        let v = HostValue::Object(vec![
            ("draft".into(), HostValue::Str("hi".into())),
            ("n".into(), HostValue::Number(2.5)),
            ("list".into(), HostValue::Array(vec![HostValue::Bool(true), HostValue::Null])),
        ]);
        assert_eq!(from_json(to_json(&v)), v);
    }

    #[test]
    fn storage_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("ados-script-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let s = Storage::open(path.clone()).unwrap();
        s.set("draft", Value::String("unsaved words".into())).unwrap();
        drop(s);
        let s = Storage::open(path.clone()).unwrap();
        assert_eq!(s.get("draft"), Some(Value::String("unsaved words".into())));
        std::fs::write(&path, "{broken").unwrap();
        assert!(Storage::open(path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
