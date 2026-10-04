use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
/// Desktop tool tiers. Phase one ships permissive defaults; users tighten them locally.
pub const TIERS: [&str; 3] = ["observe", "control", "high"];
const TIER_POLICIES: [&str; 3] = ["allow", "confirm", "deny"];
/// "similar" rules expire after 15 minutes; "session" rules last until the worker restarts.
pub const SIMILAR_RULE_MS: u64 = 15 * 60 * 1000;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DesktopTiers {
    pub observe: String,
    pub control: String,
    pub high: String,
}
impl Default for DesktopTiers {
    fn default() -> Self {
        Self {
            observe: "allow".into(),
            control: "allow".into(),
            high: "allow".into(),
        }
    }
}
impl DesktopTiers {
    pub fn policy(&self, tier: &str) -> &str {
        match tier {
            "observe" => &self.observe,
            "high" => &self.high,
            _ => &self.control,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Safety {
    pub restrict_paths: bool,
    pub roots: Vec<PathBuf>,
    pub approval: String,
    pub retention_days: u32,
    pub yield_until: u64,
    pub desktop: DesktopTiers,
}
impl Default for Safety {
    fn default() -> Self {
        Self {
            restrict_paths: false,
            roots: vec![],
            approval: "direct".into(),
            retention_days: 30,
            yield_until: 0,
            desktop: DesktopTiers::default(),
        }
    }
}
impl Safety {
    pub fn validate(&mut self) -> Result<()> {
        ensure!(
            ["direct", "risk", "all"].contains(&self.approval.as_str()),
            "invalid approval mode"
        );
        for tier in TIERS {
            ensure!(
                TIER_POLICIES.contains(&self.desktop.policy(tier)),
                "invalid desktop policy for {tier}"
            );
        }
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
/// Classify a discovered MCP tool from its declared metadata.
/// Read-only tools only observe; non-read-only tools that the backend rates
/// as its highest risk class (`r3`) are high risk; everything else controls input.
pub fn classify_tool(tool: &Value) -> &'static str {
    if tool.pointer("/annotations/readOnlyHint") == Some(&Value::Bool(true)) {
        "observe"
    } else if tool.pointer("/risk/class").and_then(Value::as_str) == Some("r3") {
        "high"
    } else {
        "control"
    }
}
/// The program a shell command starts with, without its directory.
pub fn program(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or("");
    first.rsplit('/').next().unwrap_or(first).to_owned()
}
fn plain_dir(cwd: &str) -> Option<PathBuf> {
    let p = crate::config::expand(cwd);
    // Lexical prefix checks are only meaningful without parent components.
    (p.is_absolute() && !p.components().any(|c| matches!(c, Component::ParentDir))).then_some(p)
}
/// A temporary approval granted from an approval prompt. It is held in memory
/// only, so every rule disappears when the worker restarts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AllowRule {
    pub id: String,
    pub kind: String,
    pub scope: String,
    pub program: Option<String>,
    pub cwd: Option<PathBuf>,
    pub server: Option<String>,
    pub tool: Option<String>,
    pub tier: Option<String>,
    pub created_at: u64,
    pub expires_at: Option<u64>,
}
impl AllowRule {
    /// Build a rule from the request a user approved.
    /// Commands: "similar" = same program inside the same directory tree for 15 minutes;
    /// "session" = any command inside the directory tree until restart.
    /// Desktop calls: "similar" = same tool on the same backend for 15 minutes;
    /// "session" = the same tier on the same backend until restart.
    pub fn from_request(
        kind: &str,
        scope: &str,
        args: &Value,
        tier: Option<&str>,
        now: u64,
    ) -> Result<Self> {
        let text = |k: &str| args[k].as_str().map(str::to_owned);
        let mut rule = Self {
            id: crate::model::id(),
            kind: kind.into(),
            scope: scope.into(),
            program: None,
            cwd: None,
            server: None,
            tool: None,
            tier: None,
            created_at: now,
            expires_at: match scope {
                "similar" => Some(now + SIMILAR_RULE_MS),
                "session" => None,
                _ => bail!("invalid approval scope"),
            },
        };
        match kind {
            "exec.start" => {
                let cwd = text("cwd").as_deref().and_then(plain_dir);
                ensure!(
                    cwd.is_some(),
                    "approval rule requires an absolute directory"
                );
                rule.cwd = cwd;
                if scope == "similar" {
                    let program = program(args["command"].as_str().unwrap_or(""));
                    ensure!(!program.is_empty(), "approval rule requires a program");
                    rule.program = Some(program);
                }
            }
            "mcp.call" => {
                rule.server = text("server");
                ensure!(rule.server.is_some(), "approval rule requires a backend");
                if scope == "similar" {
                    rule.tool = text("tool");
                } else {
                    rule.tier = Some(tier.unwrap_or("control").into());
                }
            }
            _ => bail!("approval rules only cover commands and desktop calls"),
        }
        Ok(rule)
    }
    pub fn active(&self, now: u64) -> bool {
        self.expires_at.is_none_or(|end| end > now)
    }
    pub fn matches(&self, kind: &str, args: &Value, tier: Option<&str>, now: u64) -> bool {
        if self.kind != kind || !self.active(now) {
            return false;
        }
        match kind {
            "exec.start" => {
                let inside = match (&self.cwd, args["cwd"].as_str().and_then(plain_dir)) {
                    (Some(root), Some(cwd)) => cwd.starts_with(root),
                    _ => false,
                };
                inside
                    && self
                        .program
                        .as_ref()
                        .is_none_or(|p| *p == program(args["command"].as_str().unwrap_or("")))
            }
            _ => {
                self.server.as_deref() == args["server"].as_str()
                    && self
                        .tool
                        .as_ref()
                        .is_none_or(|t| Some(t.as_str()) == args["tool"].as_str())
                    && self.tier.as_ref().is_none_or(|t| Some(t.as_str()) == tier)
            }
        }
    }
}
