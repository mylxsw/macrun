use crate::model::{Fault, Outcome};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Project {
    pub remote_root: String,
    pub exclude: Vec<String>,
    pub sync_timeout_seconds: u64,
}
impl Default for Project {
    fn default() -> Self {
        Self {
            remote_root: String::new(),
            exclude: vec![],
            sync_timeout_seconds: 120,
        }
    }
}
impl Project {
    pub fn load(root: &Path) -> Outcome<Self> {
        let s = std::fs::read_to_string(root.join("macrun.toml"))
            .map_err(|e| Fault::new("invalid_config", e))?;
        let p: Self = toml::from_str(&s).map_err(|e| {
            Fault::new(
                "invalid_config",
                format!(
                    "{e}; v0.2 config is sync-only: remote_root, exclude, sync_timeout_seconds"
                ),
            )
        })?;
        if p.remote_root.is_empty() || p.sync_timeout_seconds == 0 {
            return Err(Fault::new(
                "invalid_config",
                "remote_root and positive sync_timeout_seconds required",
            ));
        }
        Ok(p)
    }
    pub fn root(&self) -> PathBuf {
        expand(&self.remote_root)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backend {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkerConfig {
    pub mcp: BTreeMap<String, Backend>,
}
pub fn expand(s: &str) -> PathBuf {
    if let Some(s) = s.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(s)
    } else {
        s.into()
    }
}
