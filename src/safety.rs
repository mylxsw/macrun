use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Safety {
    pub restrict_paths: bool,
    pub roots: Vec<PathBuf>,
    pub approval: String,
    pub retention_days: u32,
    pub yield_until: u64,
}
impl Default for Safety {
    fn default() -> Self {
        Self {
            restrict_paths: false,
            roots: vec![],
            approval: "direct".into(),
            retention_days: 30,
            yield_until: 0,
        }
    }
}
impl Safety {
    pub fn validate(&mut self) -> Result<()> {
        ensure!(
            ["direct", "risk", "all"].contains(&self.approval.as_str()),
            "invalid approval mode"
        );
        ensure!(
            [7, 30, 90].contains(&self.retention_days),
            "invalid retention period"
        );
        for root in &mut self.roots {
            ensure!(root.is_absolute(), "allowed directories must be absolute");
            *root = std::fs::canonicalize(&root)?;
            ensure!(root.is_dir(), "allowed path must be a directory");
        }
        Ok(())
    }
    pub fn check(&self, path: &Path) -> Result<()> {
        if !self.restrict_paths {
            return Ok(());
        }
        ensure!(path.is_absolute(), "path_denied: absolute path required");
        // Reject parent components even when the path does not exist yet.
        ensure!(
            !path.components().any(|c| matches!(c, Component::ParentDir)),
            "path_denied: parent traversal"
        );
        let mut ancestor = path;
        let mut suffix = Vec::new();
        while std::fs::symlink_metadata(ancestor).is_err() {
            suffix.push(
                ancestor
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("invalid path"))?,
            );
            ancestor = ancestor
                .parent()
                .ok_or_else(|| anyhow::anyhow!("invalid path"))?;
        }
        let mut resolved = std::fs::canonicalize(ancestor)?;
        for part in suffix.into_iter().rev() {
            resolved.push(part);
        }
        ensure!(
            self.roots.iter().any(|root| resolved.starts_with(root)),
            "path_denied: outside allowed directories"
        );
        Ok(())
    }
    pub fn approval_required(&self, command: &str) -> bool {
        self.approval == "all" || (self.approval == "risk" && risky(command))
    }
}
/// Risk mode is a conservative convenience filter, not a shell sandbox.
pub fn risky(command: &str) -> bool {
    let c = command.trim();
    if c.contains([';', '|', '&', '>', '<', '`', '$', '\n', '(', ')']) {
        return true;
    }
    let first = c.split_whitespace().next().unwrap_or("");
    // Unknown programs, interpreters and scripts require confirmation too.
    ![
        "pwd", "ls", "cat", "head", "tail", "wc", "stat", "file", "which", "whoami", "uname",
        "date", "echo", "printf",
    ]
    .contains(&first)
}
