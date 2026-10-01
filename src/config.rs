use crate::model::{Fault, Outcome};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Project {
    pub remote_root: String,
    pub derived_data: String,
    pub project: Option<String>,
    pub workspace: Option<String>,
    pub package: bool,
    pub scheme: String,
    pub configuration: String,
    pub app_relative_path: Option<String>,
    pub exclude: Vec<String>,
    pub screenshot_max_edge: u32,
    pub allow_foreground_fallback: bool,
    pub sync_timeout_seconds: u64,
    pub build_timeout_seconds: u64,
    pub test_timeout_seconds: u64,
    pub launch_timeout_seconds: u64,
    pub ui_timeout_seconds: u64,
}
impl Default for Project {
    fn default() -> Self {
        Self {
            remote_root: String::new(),
            derived_data: String::new(),
            project: None,
            workspace: None,
            package: false,
            scheme: String::new(),
            configuration: "Debug".into(),
            app_relative_path: None,
            exclude: vec![],
            screenshot_max_edge: 1600,
            allow_foreground_fallback: false,
            sync_timeout_seconds: 120,
            build_timeout_seconds: 900,
            test_timeout_seconds: 900,
            launch_timeout_seconds: 60,
            ui_timeout_seconds: 30,
        }
    }
}
impl Project {
    pub fn load(root: &Path) -> Outcome<Self> {
        let s = std::fs::read_to_string(root.join("macrun.toml"))
            .map_err(|e| Fault::new("invalid_config", e))?;
        let p: Self = toml::from_str(&s).map_err(|e| Fault::new("invalid_config", e))?;
        if p.remote_root.is_empty()
            || p.derived_data.is_empty()
            || p.screenshot_max_edge == 0
            || usize::from(p.project.is_some())
                + usize::from(p.workspace.is_some())
                + usize::from(p.package)
                != 1
            || (!p.package && p.scheme.is_empty())
            || [
                p.sync_timeout_seconds,
                p.build_timeout_seconds,
                p.test_timeout_seconds,
                p.launch_timeout_seconds,
                p.ui_timeout_seconds,
            ]
            .contains(&0)
        {
            return Err(Fault::new(
                "invalid_config",
                "Set remote_root, derived_data and exactly one of project/workspace/package; Xcode requires scheme; limits must be positive",
            ));
        }
        Ok(p)
    }
    pub fn root(&self) -> PathBuf {
        expand(&self.remote_root)
    }
    pub fn cache(&self) -> PathBuf {
        expand(&self.derived_data)
    }
}
pub fn expand(s: &str) -> PathBuf {
    if let Some(s) = s.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(s)
    } else {
        s.into()
    }
}
