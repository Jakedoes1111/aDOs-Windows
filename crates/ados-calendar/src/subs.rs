//! Read-only `.ics` subscriptions listed in `subscriptions.toml`. Fetched on open and on
//! Refresh only; the last good copy is cached and used when a fetch fails.
use crate::{ics, model::{local_zone, Series}, store};
use serde::Deserialize;
use std::path::PathBuf;

pub const TEMPLATE: &str = "\
# Read-only calendars for aDOs. Add one [[calendar]] block per calendar.
# source is a local .ics file path or an https:// (or webcal://) URL.
# They are fetched when the calendar opens and when you press Refresh.
#
# [[calendar]]
# name = \"Holidays\"
# source = \"/home/me/holidays.ics\"
";

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub name: Option<String>,
    pub source: String,
}

#[derive(Debug, Default, Deserialize)]
struct File {
    #[serde(default)]
    calendar: Vec<Entry>,
}

impl Entry {
    pub fn label(&self) -> String {
        self.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
            let s = self.source.trim_end_matches('/');
            s.rsplit('/').next().unwrap_or(s).trim_end_matches(".ics").to_string()
        })
    }
    fn is_url(&self) -> bool {
        let s = self.source.to_ascii_lowercase();
        s.starts_with("https://") || s.starts_with("http://") || s.starts_with("webcal://")
    }
    fn cache_path(&self) -> PathBuf {
        // FNV-1a: a stable file name per source, no extra dependency.
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in self.source.bytes() { hash = (hash ^ b as u64).wrapping_mul(0x100000001b3); }
        store::cache_dir().join(format!("{hash:016x}.ics"))
    }
}

/// The list, or a sentence saying why it could not be read. Creates a commented template once.
pub fn list() -> Result<Vec<Entry>, String> {
    let path = store::subscriptions_file();
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str::<File>(&text).map(|f| f.calendar)
            .map_err(|e| format!("subscriptions.toml has a mistake, so no subscriptions are shown: {}", e.message())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = store::atomic_write(&path, TEMPLATE);
            Ok(vec![])
        }
        Err(e) => Err(format!("Could not read subscriptions.toml: {e}")),
    }
}

/// One subscription after a fetch (or from cache only).
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub name: String,
    pub series: Vec<Series>,
    pub skipped: usize,
    /// A plain sentence when the latest fetch failed; the cached copy is still shown.
    pub error: Option<String>,
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(path),
    }
}

/// Fetch the source text. Blocking: call from a background task.
fn fetch(entry: &Entry) -> Result<String, String> {
    if !entry.is_url() {
        let path = expand_home(entry.source.trim_start_matches("file://"));
        return std::fs::read_to_string(&path).map_err(|e| format!("could not read {}: {e}", path.display()));
    }
    let url = match entry.source.strip_prefix("webcal://") {
        Some(rest) => format!("https://{rest}"),
        None => entry.source.clone(),
    };
    // curl is on every Linux desktop and keeps a TLS stack out of this crate.
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "20", "--", &url])
        .output()
        .map_err(|e| format!("could not run curl to fetch it: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("curl: ").to_string();
        return Err(if why.is_empty() { "the download failed".into() } else { why });
    }
    String::from_utf8(out.stdout).map_err(|_| "the download is not text".into())
}

fn from_text(name: String, text: &str) -> Loaded {
    let parsed = ics::parse(text, &local_zone());
    Loaded { name, series: parsed.series, skipped: parsed.skipped, error: None }
}

/// The cached copy only (used by `events_between`, never touches the network).
pub fn cached(entry: &Entry) -> Loaded {
    let text = std::fs::read_to_string(entry.cache_path()).unwrap_or_default();
    from_text(entry.label(), &text)
}

/// Fetch, parse and cache one subscription. Blocking.
pub fn refresh(entry: &Entry) -> Loaded {
    let name = entry.label();
    let failed = |why: String| {
        let mut loaded = cached(entry);
        let kept = if loaded.series.is_empty() { "Nothing cached yet." } else { "Showing the last good copy." };
        loaded.error = Some(format!("Could not update {name}: {why}. {kept}"));
        loaded
    };
    match fetch(entry) {
        Ok(text) => {
            let parsed = ics::parse(&text, &local_zone());
            if !parsed.is_calendar {
                return failed("it is not an .ics calendar".into());
            }
            let mut loaded = Loaded { name: name.clone(), series: parsed.series, skipped: parsed.skipped, error: None };
            if let Err(e) = store::atomic_write(&entry.cache_path(), &text) {
                loaded.error = Some(format!("{name} is shown but could not be cached: {e:#}"));
            }
            loaded
        }
        Err(why) => failed(why),
    }
}
