//! Chord text: one canonical spelling (`ctrl-alt-shift-super-key`), display caps and warnings.

const NAMED: &[&str] = &[
    "space", "enter", "escape", "tab", "backspace", "delete", "insert", "home", "end",
    "pageup", "pagedown", "up", "down", "left", "right",
];

/// Canonical form of a single chord, or a plain sentence saying what is wrong.
/// Accepts GPUI spelling (`ctrl-shift-k`) and display spelling (`Ctrl+Shift+K`).
pub fn normalize(text: &str) -> Result<String, String> {
    let text = text.trim().to_lowercase();
    if text.is_empty() { return Err("No keys given".into()); }
    if text.split_whitespace().count() > 1 { return Err(format!("“{text}” is a key sequence; use one chord")); }
    let sep = if text.contains('+') && !text.contains('-') { '+' } else { '-' };
    let mut parts: Vec<&str> = text.split(sep).collect();
    // A trailing separator means the separator key itself, e.g. `ctrl--`.
    let key_is_sep = text.ends_with(sep) && parts.len() >= 2 && parts[parts.len() - 2].is_empty();
    let key: String = if key_is_sep {
        parts.truncate(parts.len() - 2);
        sep.to_string()
    } else {
        parts.pop().unwrap_or_default().to_string()
    };
    let (mut ctrl, mut alt, mut shift, mut sup) = (false, false, false, false);
    for part in parts {
        match part {
            "ctrl" | "control" => ctrl = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            "super" | "cmd" | "win" | "meta" | "platform" => sup = true,
            other => return Err(format!("“{other}” is not a modifier")),
        }
    }
    let key = match key.as_str() {
        "esc" => "escape".to_string(),
        "return" => "enter".to_string(),
        "del" => "delete".to_string(),
        "pgup" => "pageup".to_string(),
        "pgdn" => "pagedown".to_string(),
        "ctrl" | "control" | "alt" | "shift" | "super" | "cmd" | "win" => {
            return Err("A chord needs a key besides modifiers".into())
        }
        _ => key,
    };
    let valid = NAMED.contains(&key.as_str())
        || (key.chars().count() == 1 && key.chars().all(|c| c.is_ascii_graphic()))
        || key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n));
    if !valid { return Err(format!("“{key}” is not a key aDOs knows")); }
    let mut out = String::new();
    for (on, name) in [(ctrl, "ctrl-"), (alt, "alt-"), (shift, "shift-"), (sup, "super-")] {
        if on { out.push_str(name); }
    }
    out.push_str(&key);
    Ok(out)
}

/// Display caps, e.g. `["Ctrl", "Shift", "K"]`.
pub fn caps(chord: &str) -> Vec<String> {
    let (mods, key) = split(chord);
    let mut out: Vec<String> = mods.iter().map(|m| match *m {
        "ctrl" => "Ctrl", "alt" => "Alt", "shift" => "Shift", _ => "Super",
    }.to_string()).collect();
    out.push(match key {
        "escape" => "Esc".into(),
        "pageup" => "PgUp".into(),
        "pagedown" => "PgDn".into(),
        "backspace" => "Backspace".into(),
        k if k.len() == 1 => k.to_uppercase(),
        k => { let mut c = k.chars(); c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default() }
    });
    out
}

/// `Ctrl+Shift+K`.
pub fn label(chord: &str) -> String { caps(chord).join("+") }

fn split(chord: &str) -> (Vec<&str>, &str) {
    if let Some(stripped) = chord.strip_suffix("--") {
        let mods = if stripped.is_empty() { vec![] } else { stripped.split('-').collect() };
        return (mods, "-");
    }
    let mut parts: Vec<&str> = chord.split('-').collect();
    let key = parts.pop().unwrap_or_default();
    (parts, key)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning { TextField, WindowManager }
impl Warning {
    pub fn sentence(self) -> &'static str {
        match self {
            Warning::TextField => "Will be eaten by text fields",
            Warning::WindowManager => "The window manager usually takes this",
        }
    }
}

/// Chords that are risky regardless of other shortcuts. `chord` must be canonical.
pub fn warning(chord: &str, scope: &str) -> Option<Warning> {
    let (mods, key) = split(chord);
    if mods.contains(&"super") || matches!(chord, "alt-tab" | "alt-shift-tab" | "alt-f4" | "ctrl-alt-delete" | "ctrl-alt-backspace") {
        return Some(Warning::WindowManager);
    }
    // A list of rows holds no text field of its own.
    if scope == "NotesList" { return None; }
    let printable = key.len() == 1 || key == "space";
    let text_key = printable || matches!(key, "backspace" | "up" | "down" | "left" | "right");
    let eaten = (mods.is_empty() && text_key)
        || (mods == ["shift"] && printable)
        || (mods == ["ctrl"] && matches!(key, "a" | "c" | "v" | "x" | "z"));
    eaten.then_some(Warning::TextField)
}
