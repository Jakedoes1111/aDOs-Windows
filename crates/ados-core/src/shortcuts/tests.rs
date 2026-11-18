use super::chord::{caps, label, normalize, warning, Warning};
use super::defaults::DEFAULTS;
use super::file::{parse, render};
use super::model::*;

fn all() -> Vec<Spec> { DEFAULTS.iter().map(|d| d.spec()).collect() }
fn ix(specs: &[Spec], id: &str) -> usize { specs.iter().position(|s| s.id == id).expect(id) }

#[test]
fn defaults_have_zero_conflicts() {
    let specs = all();
    let found = conflicts(&specs, &effective(&specs, &UserMap::new()));
    let names: Vec<String> = found.iter().map(|c| format!("{} vs {} on {}", specs[c.a].id, specs[c.b].id, c.chord)).collect();
    assert!(names.is_empty(), "default conflicts: {names:?}");
}

#[test]
fn default_ids_unique_and_chords_valid() {
    let mut ids: Vec<&str> = DEFAULTS.iter().map(|d| d.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), DEFAULTS.len());
    for d in DEFAULTS {
        for k in d.keys { assert_eq!(normalize(k).as_deref(), Ok(*k), "{} {}", d.id, k); }
        for o in d.overrides { assert!(DEFAULTS.iter().any(|x| x.id == *o), "{o}"); }
    }
}

#[test]
fn terminal_ctrl_l_is_an_intended_override() {
    let specs = all();
    let chords = effective(&specs, &UserMap::new());
    let clash = clashes(&specs, &chords).into_iter()
        .find(|c| c.chord == "ctrl-l").expect("ctrl-l is shared");
    assert!(clash.intended);
    let pair = [specs[clash.a].id.as_str(), specs[clash.b].id.as_str()];
    assert!(pair.contains(&"terminal.keep_ctrl_l") && pair.contains(&"browser.focus_address"));
}

#[test]
fn scope_rules() {
    assert!(overlaps(GLOBAL, "Terminal"));
    assert!(overlaps("AdosShell", "NotesList"));
    assert!(overlaps("NotesPanel", "NotesList"));
    assert!(!overlaps("NotesPanel", "Terminal"));
    assert!(!overlaps("AdosPalette", "NotesPanel"));
    let spec = |id: &str, scope: &str, keys: &[&str], ov: &[&str]| Spec {
        id: id.into(), title: id.into(), scope: scope.into(),
        defaults: keys.iter().map(|k| k.to_string()).collect(), overrides: ov.iter().map(|k| k.to_string()).collect(),
    };
    let specs = vec![
        spec("a", GLOBAL, &["ctrl-j"], &[]),
        spec("b", "Terminal", &["ctrl-j"], &[]),
        spec("c", "NotesPanel", &["ctrl-y"], &[]),
        spec("d", "Terminal", &["ctrl-y"], &[]),
        spec("e", "Terminal", &["ctrl-m"], &["f"]),
        spec("f", GLOBAL, &["ctrl-m"], &[]),
    ];
    let chords = effective(&specs, &UserMap::new());
    let found = conflicts(&specs, &chords);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].a, found[0].b), (0, 1));
    assert_eq!(conflict_for(&specs, &chords, 2, "ctrl-j"), Some(0));
    assert_eq!(conflict_for(&specs, &chords, 2, "ctrl-y"), None);
    assert_eq!(conflict_for(&specs, &chords, 5, "ctrl-m"), None);
}

#[test]
fn chord_normalize_and_display() {
    assert_eq!(normalize("Ctrl+Alt+N").as_deref(), Ok("ctrl-alt-n"));
    assert_eq!(normalize("shift-ctrl-K").as_deref(), Ok("ctrl-shift-k"));
    assert_eq!(normalize("ctrl-,").as_deref(), Ok("ctrl-,"));
    assert_eq!(normalize("ctrl--").as_deref(), Ok("ctrl--"));
    assert_eq!(normalize("esc").as_deref(), Ok("escape"));
    assert_eq!(normalize("F5").as_deref(), Ok("f5"));
    assert!(normalize("ctrl-").is_err() || normalize("ctrl-").as_deref() == Ok("ctrl--"));
    assert!(normalize("hyper-x").is_err());
    assert!(normalize("ctrl-banana").is_err());
    assert!(normalize("ctrl-k ctrl-s").is_err());
    assert!(normalize("ctrl").is_err());
    assert_eq!(caps("ctrl-shift-k"), vec!["Ctrl", "Shift", "K"]);
    assert_eq!(label("shift-escape"), "Shift+Esc");
    assert_eq!(label("ctrl--"), "Ctrl+-");
    assert_eq!(label("alt-space"), "Alt+Space");
}

#[test]
fn warnings() {
    assert_eq!(warning("a", "NotesPanel"), Some(Warning::TextField));
    assert_eq!(warning("shift-a", GLOBAL), Some(Warning::TextField));
    assert_eq!(warning("backspace", GLOBAL), Some(Warning::TextField));
    assert_eq!(warning("left", GLOBAL), Some(Warning::TextField));
    assert_eq!(warning("ctrl-v", GLOBAL), Some(Warning::TextField));
    assert_eq!(warning("ctrl-z", "NotesList"), None);
    assert_eq!(warning("ctrl-alt-n", GLOBAL), None);
    assert_eq!(warning("alt-tab", GLOBAL), Some(Warning::WindowManager));
    assert_eq!(warning("alt-f4", GLOBAL), Some(Warning::WindowManager));
    assert_eq!(warning("super-e", GLOBAL), Some(Warning::WindowManager));
}

#[test]
fn keymap_parse_apply_and_skip_bad_entries() {
    let specs = all();
    let text = r#"
"notes.new" = "Ctrl+Alt+N"
shell.close = ""
"shell.zoom" = ["f11", "shift-escape"]
"no.such" = "ctrl-j"
"shell.palette" = "ctrl-banana"
"shell.capture" = 3
"#;
    let parsed = parse(text, &specs);
    assert_eq!(parsed.user.get("notes.new"), Some(&vec!["ctrl-alt-n".to_string()]));
    assert_eq!(parsed.user.get("shell.close"), Some(&vec![]));
    assert_eq!(parsed.user.get("shell.zoom"), Some(&vec!["f11".to_string(), "shift-escape".to_string()]));
    assert_eq!(parsed.user.len(), 3);
    assert_eq!(parsed.problems.len(), 3, "{:?}", parsed.problems);
    assert!(parsed.problems.iter().any(|p| p.contains("no.such")));
    assert!(parsed.problems.iter().any(|p| p.contains("shell.palette")));
    let chords = effective(&specs, &parsed.user);
    assert_eq!(chords[ix(&specs, "notes.new")], vec!["ctrl-alt-n"]);
    assert!(chords[ix(&specs, "shell.close")].is_empty());
    assert_eq!(chords[ix(&specs, "shell.palette")], vec!["ctrl-k"]);
    // Round trip.
    let again = parse(&render(&parsed.user, &specs), &specs);
    assert_eq!(again.user, parsed.user);
    assert!(again.problems.is_empty());
    // Unreadable file: defaults and one problem.
    let broken = parse("\"notes.new\" = ", &specs);
    assert!(broken.user.is_empty());
    assert_eq!(broken.problems.len(), 1);
}

#[test]
fn swap_and_replace() {
    let specs = all();
    let new = ix(&specs, "notes.new");
    let palette = ix(&specs, "shell.palette");
    let mut user = UserMap::new();
    let chords = effective(&specs, &user);
    assert_eq!(conflict_for(&specs, &chords, new, "ctrl-k"), Some(palette));
    swap(&mut user, &specs, new, palette, "ctrl-k");
    let chords = effective(&specs, &user);
    assert_eq!(chords[new], vec!["ctrl-k"]);
    assert_eq!(chords[palette], vec!["ctrl-n"]);
    // The palette's old chord now clashes with "Next result"; the editor counts it.
    let left = conflicts(&specs, &chords);
    assert_eq!(left.len(), 1);
    assert_eq!(specs[left[0].b].id, "palette.next");

    let mut user = UserMap::new();
    replace(&mut user, &specs, new, palette, "ctrl-k");
    let chords = effective(&specs, &user);
    assert_eq!(chords[new], vec!["ctrl-k"]);
    assert!(chords[palette].is_empty());
    assert!(conflicts(&specs, &chords).is_empty());

    // Setting a shortcut back to its default leaves no entry behind.
    set(&mut user, &specs[new], vec!["ctrl-n".into()]);
    assert!(!user.contains_key("notes.new"));
}
