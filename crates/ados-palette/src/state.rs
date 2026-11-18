use ados_core::paths;
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct SavedState {
    pub recent: Vec<String>,
    // Do not migrate the legacy `draft`: it also contained action searches.
    pub capture_draft: String,
}

impl SavedState {
    pub fn load() -> anyhow::Result<Self> {
        match std::fs::read(paths::state_file("palette")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self) -> anyhow::Result<()> {
        let path = paths::state_file("palette");
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec(self)?)?;
        std::fs::rename(temp, path)?;
        Ok(())
    }
    pub fn ran(&mut self, id: &str) {
        self.recent.retain(|old| old != id);
        self.recent.insert(0, id.to_owned());
        self.recent.truncate(100);
    }
}
