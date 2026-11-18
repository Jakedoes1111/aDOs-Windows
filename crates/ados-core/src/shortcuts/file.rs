//! `keymap.toml`: only the user's changes. `"notes.new" = "ctrl-alt-n"`, `"id" = ""` unbinds,
//! a list gives several chords. Unquoted dotted ids (`notes.new = …`) work too.
use super::{chord, model::{Spec, UserMap}};

pub struct Parsed { pub user: UserMap, pub problems: Vec<String> }

/// Never fails: bad entries are skipped and named in `problems`.
pub fn parse(text: &str, specs: &[Spec]) -> Parsed {
    let mut parsed = Parsed { user: UserMap::new(), problems: vec![] };
    let table = match text.parse::<toml::Table>() {
        Ok(table) => table,
        Err(error) => {
            let line = error.span().map(|s| text[..s.start.min(text.len())].lines().count().max(1));
            parsed.problems.push(match line {
                Some(line) => format!("keymap.toml line {line} could not be read; using default shortcuts"),
                None => "keymap.toml could not be read; using default shortcuts".into(),
            });
            return parsed;
        }
    };
    let mut flat = vec![];
    flatten("", &toml::Value::Table(table), &mut flat);
    for (id, value) in flat {
        if !specs.iter().any(|s| s.id == id) {
            parsed.problems.push(format!("Skipped “{id}” in keymap.toml: no such shortcut"));
            continue;
        }
        let texts: Vec<&str> = match &value {
            toml::Value::String(s) if s.trim().is_empty() => vec![],
            toml::Value::String(s) => vec![s.as_str()],
            toml::Value::Array(items) if items.iter().all(|v| v.is_str()) => {
                items.iter().filter_map(|v| v.as_str()).filter(|s| !s.trim().is_empty()).collect()
            }
            _ => {
                parsed.problems.push(format!("Skipped “{id}” in keymap.toml: use a chord in quotes, like \"ctrl-alt-n\""));
                continue;
            }
        };
        let mut chords = vec![];
        let mut bad = None;
        for text in texts {
            match chord::normalize(text) {
                Ok(c) => if !chords.contains(&c) { chords.push(c) },
                Err(e) => { bad = Some(e); break; }
            }
        }
        match bad {
            Some(e) => parsed.problems.push(format!("Skipped “{id}” in keymap.toml: {e}")),
            None => { parsed.user.insert(id, chords); }
        }
    }
    parsed
}

fn flatten(prefix: &str, value: &toml::Value, out: &mut Vec<(String, toml::Value)>) {
    match value {
        toml::Value::Table(table) => for (key, value) in table {
            let id = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
            flatten(&id, value, out);
        },
        other => out.push((prefix.to_string(), other.clone())),
    }
}

/// Stable text with a short header, one quoted id per line.
pub fn render(user: &UserMap, specs: &[Spec]) -> String {
    let mut out = String::from(
        "# aDOs shortcuts: only your changes. Edited by Settings → Shortcuts or by hand; saved changes apply at once.\n\
         # \"id\" = \"ctrl-alt-n\" rebinds, \"id\" = \"\" unbinds, [\"a\", \"b\"] gives several chords.\n",
    );
    for (id, chords) in user {
        let title = specs.iter().find(|s| &s.id == id).map(|s| s.title.as_str()).unwrap_or("");
        let value = match chords.as_slice() {
            [] => "\"\"".to_string(),
            [one] => format!("\"{one}\""),
            many => format!("[{}]", many.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ")),
        };
        out.push_str(&format!("\"{id}\" = {value}"));
        if !title.is_empty() { out.push_str(&format!("  # {title}")); }
        out.push('\n');
    }
    out
}
