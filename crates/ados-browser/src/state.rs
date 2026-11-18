use serde::{Deserialize, Serialize};

pub const DEFAULT_URL: &str = "https://duckduckgo.com";
const SERVO_START: &str = "https://search.brave.com/";

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Engine {
    #[cfg_attr(not(windows), default)]
    Servo,
    /// On Windows the one engine: WebView2 is Chromium.
    #[cfg_attr(windows, default)]
    Chromium,
}
impl Engine {
    pub fn start_page(self) -> &'static str {
        match self {
            Self::Servo => SERVO_START,
            Self::Chromium => DEFAULT_URL,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Servo => "Servo",
            Self::Chromium => "Chromium",
        }
    }
    pub fn other(self) -> Self {
        match self {
            Self::Servo => Self::Chromium,
            Self::Chromium => Self::Servo,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct State {
    pub url: String,
    pub engine: Engine,
    /// Where the keyboard was inside the Browser, page or address bar (focus.rs).
    #[serde(default)]
    pub focus: Option<crate::focus::Place>,
    /// Addresses of the open page tabs in order (`url` is the selected one's).
    #[serde(default)]
    pub tabs: Vec<String>,
    #[serde(default)]
    pub selected: usize,
    /// Tiled tabs, by index in `tabs` (`tiles.rs`).
    #[serde(default)]
    pub tile_groups: Vec<SavedGroup>,
    /// (follower, leader) by index in `tabs`.
    #[serde(default)]
    pub follows: Vec<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedGroup {
    pub tabs: Vec<usize>,
    pub layout: crate::tiles::Layout,
    #[serde(default)]
    pub sizes: Vec<f32>,
    #[serde(default)]
    pub cross: Vec<f32>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            url: Engine::default().start_page().into(),
            engine: Engine::default(),
            focus: None,
            tabs: vec![],
            selected: 0,
            tile_groups: vec![],
            follows: vec![],
        }
    }
}
impl State {
    pub fn load() -> Self {
        #[allow(unused_mut)]
        let mut state: Self = std::fs::read(ados_core::paths::state_file("browser"))
            .ok()
            .and_then(|s| serde_json::from_slice(&s).ok())
            .unwrap_or_default();
        #[cfg(windows)]
        { state.engine = Engine::Chromium; }
        state
    }
    pub fn save(&self) -> anyhow::Result<()> {
        let path = ados_core::paths::state_file("browser");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec(self)?)?;
        std::fs::rename(temp, path)?;
        Ok(())
    }
}

pub fn address(text: &str, engine: Engine) -> String {
    let text = text.trim();
    if text.is_empty() {
        return engine.start_page().into();
    }
    if text.starts_with("https://") || text.starts_with("http://") || text.starts_with("about:") {
        return text.into();
    }
    if !text.contains(char::is_whitespace)
        && (text.contains('.') || text.starts_with("localhost") || text.starts_with('['))
    {
        return format!("https://{text}");
    }
    let encoded: String = text
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    match engine {
        Engine::Servo => format!("{SERVO_START}search?q={encoded}"),
        Engine::Chromium => format!("https://duckduckgo.com/?q={encoded}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses() {
        assert_eq!(
            address("example.com", Engine::Chromium),
            "https://example.com"
        );
        assert_eq!(
            address("hello world", Engine::Chromium),
            "https://duckduckgo.com/?q=hello+world"
        );
        assert_eq!(
            address("rust", Engine::Chromium),
            "https://duckduckgo.com/?q=rust"
        );
        assert_eq!(
            address("a & café", Engine::Chromium),
            "https://duckduckgo.com/?q=a+%26+caf%C3%A9"
        );
        assert_eq!(
            address("http://localhost:8422/", Engine::Chromium),
            "http://localhost:8422/"
        );
        assert_eq!(address("", Engine::Chromium), DEFAULT_URL);
        assert_eq!(address("", Engine::Servo), SERVO_START);
        assert_eq!(State::default().url, SERVO_START);
        assert_eq!(
            address("a & café", Engine::Servo),
            "https://search.brave.com/search?q=a+%26+caf%C3%A9"
        );
        assert_eq!(
            address("https://example.com", Engine::Servo),
            "https://example.com"
        );
    }
}
