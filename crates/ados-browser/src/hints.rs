//! Link hints inside the page: one engine-neutral script (`hints.js`) and its settings.
//! Servo loads it from a `--userscripts` folder; Chromium through CDP. aDOs's own hint
//! mode drives the same script through `Runtime::eval`.
use ados_core::hints::Style;
use gpui_kit::App;
use serde_json::{json, Value};
use std::path::PathBuf;

pub const SCRIPT: &str = include_str!("../hints.js");

/// Effective chords and look, as the script's `config` object.
pub fn config(cx: &App) -> Value {
    let chords = |id: &str| -> Vec<String> {
        let specs = ados_core::shortcuts::specs(cx);
        let chords = ados_core::shortcuts::chords(cx);
        match specs.iter().position(|s| s.id == id) {
            Some(i) => chords.get(i).cloned().unwrap_or_default(),
            None => ados_core::shortcuts::defaults::find(id).map(|d| d.spec().defaults).unwrap_or_default(),
        }
    };
    let (style, _) = Style::load();
    json!({
        "hints": chords("browser.hints"),
        "hintsNew": chords("browser.hints_new"),
        "bg": Style::css(style.background),
        "fg": Style::css(style.text),
        "typed": Style::css(style.typed),
        "timeout": ados_core::hints::TIMEOUT_MS,
    })
}

/// Apply `config` on the current page, if the script is there.
pub fn config_call(config: &Value) -> String {
    format!("window.__adosHints && window.__adosHints.config({config})")
}

/// A (non-modifier) key release whose press this document never saw (the chord went to aDOs, then the engine
/// reported the release when focus left the page) is dropped before page code runs. Pairs stay whole.
const KEY_GUARD: &str = "(function(){if(window.__adosKeyGuard)return;window.__adosKeyGuard=1;const down=new Set();\
addEventListener('keydown',e=>{down.add(e.code||e.key)},true);\
addEventListener('keyup',e=>{const k=e.code||e.key;if(!down.delete(k)&&!['Control','Shift','Alt','Meta','AltGraph'].includes(e.key)){e.stopImmediatePropagation();e.preventDefault();}},true);\
addEventListener('blur',e=>{if(e.target===window)down.clear()},true);})();";

/// The full script for a new document: the hint code, the key guard, then its settings.
pub fn source(config: &Value) -> String {
    format!("{SCRIPT}\n{KEY_GUARD}\n;{};\n", config_call(config))
}

/// Folder for Servo's `--userscripts`, holding only the hint script.
pub fn userscripts(config: &Value) -> std::io::Result<PathBuf> {
    let dir = ados_core::paths::home().join("browser").join("userscripts");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("ados-hints.js"), source(config))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_installs_once_and_configures() {
        let config = json!({"hints":["f"]});
        let text = source(&config);
        assert!(text.contains("if (window.__adosHints) return;"));
        assert!(text.contains("__adosKeyGuard"));
        assert!(text.ends_with("window.__adosHints.config({\"hints\":[\"f\"]});\n"));
    }
}
