//! The capability gate aDOs owns. A panel's `panel.json` asks; aDOs decides.
//! Reading context and the panel's own storage are always allowed. Everything else
//! (running actions, processes, network, clipboard) needs the user's yes, and is denied
//! until then. Decisions live in `ADOS_HOME/state/script-approvals.json`, outside the
//! script folder, so a script cannot approve itself by editing a file next to it.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `ADOS_HOME/ui/<name>/panel.json`. Every field is optional.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Manifest {
    pub title: Option<String>,
    pub asks: Asks,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Asks {
    /// Run aDOs palette actions.
    pub actions: bool,
    /// Command names the panel may start.
    pub process: Vec<String>,
    /// Hosts the panel may reach.
    pub network: Vec<String>,
    /// Read and write the clipboard.
    pub clipboard: bool,
}

/// One thing that needs approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Actions,
    Process(Vec<String>),
    Network(Vec<String>),
    Clipboard,
}

impl Ask {
    /// Stable key for the saved decision. A changed command or host list is a new ask.
    pub fn key(&self) -> String {
        match self {
            Ask::Actions => "actions".into(),
            Ask::Process(c) => format!("process:{}", c.join(",")),
            Ask::Network(h) => format!("network:{}", h.join(",")),
            Ask::Clipboard => "clipboard".into(),
        }
    }
    /// The question, in plain words, finishing "<Panel> wants to …".
    pub fn question(&self) -> String {
        match self {
            Ask::Actions => "run aDOs actions".into(),
            Ask::Process(c) => format!("run programs on this computer: {}", c.join(", ")),
            Ask::Network(h) => format!("reach the internet: {}", h.join(", ")),
            Ask::Clipboard => "read and write the clipboard".into(),
        }
    }
}

impl Manifest {
    pub fn read(dir: &Path) -> Result<Self, String> {
        let path = dir.join("panel.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("panel.json could not be read: {e}.")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("panel.json could not be opened: {e}.")),
        }
    }

    pub fn asks(&self) -> Vec<Ask> {
        let a = &self.asks;
        let mut out = Vec::new();
        if a.actions { out.push(Ask::Actions); }
        if !a.process.is_empty() { out.push(Ask::Process(a.process.clone())); }
        if !a.network.is_empty() { out.push(Ask::Network(a.network.clone())); }
        if a.clipboard { out.push(Ask::Clipboard); }
        out
    }
}

/// Saved yes/no per panel per ask.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Decisions(BTreeMap<String, BTreeMap<String, bool>>);

impl Decisions {
    pub fn file() -> PathBuf { ados_core::paths::state_file("script-approvals") }

    /// A missing or unreadable file means nothing was approved: deny by default.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .map(Self).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.0).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)
            .and_then(|_| std::fs::rename(&tmp, path))
            .map_err(|e| format!("The permission choice could not be saved: {e}."))
    }

    pub fn get(&self, panel: &str, ask: &Ask) -> Option<bool> {
        self.0.get(panel).and_then(|m| m.get(&ask.key())).copied()
    }

    pub fn set(&mut self, panel: &str, ask: &Ask, allowed: bool) {
        self.0.entry(panel.into()).or_default().insert(ask.key(), allowed);
    }

    /// The first ask with no decision yet: the question to show.
    pub fn pending(&self, panel: &str, asks: &[Ask]) -> Option<Ask> {
        asks.iter().find(|a| self.get(panel, a).is_none()).cloned()
    }

    /// What the panel may do right now. Undecided and denied asks are both off.
    pub fn grants(&self, panel: &str, asks: &[Ask]) -> Grants {
        let mut g = Grants::default();
        for ask in asks.iter().filter(|a| self.get(panel, a) == Some(true)) {
            match ask {
                Ask::Actions => g.actions = true,
                Ask::Process(c) => g.process = c.clone(),
                Ask::Network(h) => g.network = h.clone(),
                Ask::Clipboard => g.clipboard = true,
            }
        }
        g
    }

    pub fn denied(&self, panel: &str, asks: &[Ask]) -> Vec<Ask> {
        asks.iter().filter(|a| self.get(panel, a) == Some(false)).cloned().collect()
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Grants {
    pub actions: bool,
    pub process: Vec<String>,
    pub network: Vec<String>,
    pub clipboard: bool,
}

impl Grants {
    pub fn capabilities(&self) -> gpui_shell::Capabilities {
        let mut c = gpui_shell::Capabilities::new();
        if !self.process.is_empty() {
            c = c.execute(gpui_shell::ExecuteGrant::Allowed(self.process.clone()));
        }
        if !self.network.is_empty() { c = c.network_hosts(self.network.clone()); }
        if self.clipboard { c = c.clipboard_read(true).clipboard_write(true); }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> Manifest { serde_json::from_str(json).unwrap() }

    #[test]
    fn nothing_asked_needs_no_question() {
        let m = manifest(r#"{"title":"Hello"}"#);
        assert!(m.asks().is_empty());
        assert_eq!(Decisions::default().pending("hello", &m.asks()), None);
    }

    #[test]
    fn asks_are_denied_until_answered() {
        let m = manifest(r#"{"asks":{"actions":true,"process":["git"]}}"#);
        let asks = m.asks();
        let mut d = Decisions::default();
        assert_eq!(d.pending("p", &asks), Some(Ask::Actions));
        assert_eq!(d.grants("p", &asks), Grants::default());
        d.set("p", &Ask::Actions, true);
        assert_eq!(d.pending("p", &asks), Some(Ask::Process(vec!["git".into()])));
        d.set("p", &asks[1], false);
        assert_eq!(d.pending("p", &asks), None);
        let g = d.grants("p", &asks);
        assert!(g.actions);
        assert!(g.process.is_empty());
        assert_eq!(d.denied("p", &asks), vec![asks[1].clone()]);
    }

    #[test]
    fn a_wider_ask_is_asked_again() {
        let mut d = Decisions::default();
        d.set("p", &Ask::Process(vec!["git".into()]), true);
        let wider = Ask::Process(vec!["git".into(), "rm".into()]);
        assert_eq!(d.pending("p", &[wider.clone()]), Some(wider));
    }

    #[test]
    fn decisions_are_per_panel() {
        let mut d = Decisions::default();
        d.set("a", &Ask::Clipboard, true);
        assert_eq!(d.get("b", &Ask::Clipboard), None);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(serde_json::from_str::<Manifest>(r#"{"asks":{"fs":true}}"#).is_err());
    }
}
