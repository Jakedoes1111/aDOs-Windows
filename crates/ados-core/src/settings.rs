//! App-wide settings: `ADOS_HOME/settings.json`. Saved at once on every change.
use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Whole-interface zoom steps, in percent. 100 is the default.
pub const ZOOM_STEPS: [u32; 6] = [75, 90, 100, 110, 125, 150];
pub const ZOOM_DEFAULT: u32 = 100;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Whole-interface zoom in percent (one of `ZOOM_STEPS`).
    pub zoom_percent: u32,
    /// A layout name to open on launch; `None` reopens the last arrangement.
    pub startup_layout: Option<String>,
    /// Hosts the Browser opens in Chromium while its engine is Servo (each matches its subdomains).
    pub chromium_sites: Vec<String>,
    /// The Browser switches to Chromium when Servo shows its own load-failure page.
    pub chromium_on_failure: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            zoom_percent: ZOOM_DEFAULT,
            startup_layout: None,
            chromium_sites: DEFAULT_CHROMIUM_SITES.iter().map(|s| s.to_string()).collect(),
            chromium_on_failure: true,
        }
    }
}

/// Sites Servo cannot serve well (video playback): opened in Chromium out of the box.
pub const DEFAULT_CHROMIUM_SITES: [&str; 3] = ["youtube.com", "youtu.be", "youtube-nocookie.com"];

/// The host of an address, lower-cased, without scheme, user, port, path or a trailing dot.
/// `None` for an address with no host (`about:blank`, a search phrase with spaces).
pub fn host_of(address: &str) -> Option<String> {
    let rest = address.trim();
    let rest = match rest.find("://") {
        Some(at) if rest[..at].chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) => &rest[at + 3..],
        // about:blank, mailto:… have a scheme and no host; `localhost:8765` is a host and port.
        _ if rest.split_once(':').is_some_and(|(_, after)| !after.starts_with(|c: char| c.is_ascii_digit())) => return None,
        _ => rest,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = if let Some(v6) = host.strip_prefix('[') {
        format!("[{}]", v6.split(']').next().unwrap_or_default())
    } else {
        host.split(':').next().unwrap_or_default().to_string()
    };
    let host = host.trim_end_matches('.').to_lowercase();
    (!host.is_empty() && !host.contains(char::is_whitespace)).then_some(host)
}

/// Is the address's host one of `sites`, or a subdomain of one?
pub fn site_listed(address: &str, sites: &[String]) -> bool {
    let Some(host) = host_of(address) else { return false };
    sites.iter().any(|site| host == *site || host.strip_suffix(site.as_str()).is_some_and(|sub| sub.ends_with('.')))
}

/// A typed list entry, cleaned: trimmed, lower-cased, scheme and path removed. Err: a sentence.
pub fn clean_site(typed: &str) -> Result<String, String> {
    let shown = typed.trim();
    if shown.is_empty() { return Err("Type a site, such as youtube.com.".into()); }
    let bad = || format!("“{shown}” is not a site name. Type one such as youtube.com.");
    let host = host_of(shown).ok_or_else(bad)?;
    let host = host.strip_prefix("*.").unwrap_or(&host).to_string();
    let ok = if host.starts_with('[') { host.len() > 2 } else {
        host.split('.').all(|label| !label.is_empty() && !label.starts_with('-') && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    };
    if ok { Ok(host) } else { Err(bad()) }
}
impl Global for Settings {}

pub fn file_path() -> PathBuf { crate::paths::home().join("settings.json") }

/// The step nearest to `percent`.
pub fn nearest_step(percent: u32) -> u32 {
    ZOOM_STEPS.iter().copied().min_by_key(|s| s.abs_diff(percent)).unwrap_or(ZOOM_DEFAULT)
}

/// One step up (`dir > 0`) or down (`dir < 0`), stopping at the ends; `0` resets.
pub fn zoom_step(current: u32, dir: i32) -> u32 {
    if dir == 0 { return ZOOM_DEFAULT; }
    let at = ZOOM_STEPS.iter().position(|s| *s == nearest_step(current)).unwrap_or(2);
    let next = (at as i32 + dir.signum()).clamp(0, ZOOM_STEPS.len() as i32 - 1);
    ZOOM_STEPS[next as usize]
}

/// Read the file; a missing file is the defaults, a broken one is the defaults plus a plain sentence.
pub fn load() -> (Settings, Option<String>) {
    match std::fs::read_to_string(file_path()) {
        Ok(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => { s.zoom_percent = nearest_step(s.zoom_percent); (s, None) }
            Err(e) => (Settings::default(), Some(format!("settings.json is not valid, so the defaults are used: {e}"))),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
        Err(e) => (Settings::default(), Some(format!("Could not read settings.json: {e}"))),
    }
}

/// Install the global from disk. Returns a problem sentence to show, if any.
pub fn init(cx: &mut App) -> Option<String> {
    let (s, problem) = load();
    cx.set_global(s);
    problem
}

pub fn get(cx: &App) -> Settings { cx.try_global::<Settings>().cloned().unwrap_or_default() }

/// Change a setting and save it. The change stays in memory even when the save fails.
pub fn update(cx: &mut App, edit: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let mut s = get(cx);
    edit(&mut s);
    cx.set_global(s.clone());
    let text = serde_json::to_string_pretty(&s).map_err(|e| format!("Could not save settings: {e}"))?;
    let path = file_path();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("Could not save settings.json: {e}"))
}

#[cfg(test)]
mod tests {
    use super::{clean_site, host_of, nearest_step, site_listed, zoom_step, Settings};
    fn list(sites: &[&str]) -> Vec<String> { sites.iter().map(|s| s.to_string()).collect() }
    #[test]
    fn hosts_match_with_subdomains() {
        let sites = list(&["youtube.com", "youtu.be", "localhost"]);
        assert!(site_listed("https://www.youtube.com/watch?v=x", &sites));
        assert!(site_listed("https://youtube.com", &sites));
        assert!(site_listed("https://m.YouTube.com./", &sites));
        assert!(site_listed("http://youtu.be/abc", &sites));
        assert!(site_listed("http://localhost:8765/second.html", &sites));
        assert!(site_listed("https://user@music.youtube.com:443/", &sites));
        assert!(!site_listed("https://notyoutube.com/", &sites));
        assert!(!site_listed("https://youtube.com.evil.net/", &sites));
        assert!(!site_listed("https://example.com/?u=youtube.com", &sites));
        assert!(!site_listed("about:blank", &sites));
        assert!(!site_listed("", &sites));
        assert!(!site_listed("http://127.0.0.1:8765/", &sites));
        assert!(site_listed("http://127.0.0.1:8765/", &list(&["127.0.0.1"])));
        assert_eq!(host_of("http://[::1]:8080/x").as_deref(), Some("[::1]"));
    }
    #[test]
    fn entries_are_cleaned() {
        assert_eq!(clean_site("  YouTube.com ").unwrap(), "youtube.com");
        assert_eq!(clean_site("https://www.youtube.com/watch?v=1").unwrap(), "www.youtube.com");
        assert_eq!(clean_site("youtube.com/feed").unwrap(), "youtube.com");
        assert_eq!(clean_site("http://localhost:8765/").unwrap(), "localhost");
        assert_eq!(clean_site("*.example.org").unwrap(), "example.org");
        assert_eq!(clean_site("localhost:8765").unwrap(), "localhost");
        assert!(clean_site("").is_err());
        assert!(clean_site("two words").is_err());
        assert!(clean_site("bad_host!.com").is_err());
        assert!(clean_site("a..b").is_err());
        assert!(clean_site("about:blank").is_err());
    }
    #[test]
    fn old_settings_file_loads_with_defaults() {
        let s: Settings = serde_json::from_str(r#"{"zoom_percent":110,"startup_layout":null}"#).unwrap();
        assert_eq!(s.zoom_percent, 110);
        assert!(s.chromium_on_failure);
        assert_eq!(s.chromium_sites, list(&["youtube.com", "youtu.be", "youtube-nocookie.com"]));
    }
    #[test]
    fn steps_walk_and_stop() {
        assert_eq!(zoom_step(100, 1), 110);
        assert_eq!(zoom_step(100, -1), 90);
        assert_eq!(zoom_step(150, 1), 150);
        assert_eq!(zoom_step(75, -1), 75);
        assert_eq!(zoom_step(150, 0), 100);
        assert_eq!(zoom_step(104, 1), 110);
    }
    #[test]
    fn odd_values_snap() {
        assert_eq!(nearest_step(0), 75);
        assert_eq!(nearest_step(999), 150);
        assert_eq!(nearest_step(118), 125);
    }
}
