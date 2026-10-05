//! Read-only task history. Cache summaries, never tool images or command output.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::SystemTime,
};

#[derive(Clone)]
pub struct TaskHistory {
    data: PathBuf,
    cache: Arc<Mutex<BTreeMap<String, CachedTask>>>,
}
struct CachedTask {
    modified: Option<SystemTime>,
    length: u64,
    value: Value,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Cursor {
    started_at: u64,
    task_id: String,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Query {
    limit: Option<usize>,
    status: String,
    query: String,
    kind: Option<String>,
    cursor: Option<Cursor>,
}

/// A command that ran and exited non-zero. The agent reads the exit code and
/// decides what to do, so this is the agent's result rather than a Macrun fault.
pub fn exited_nonzero(task: &Value) -> bool {
    task["status"] == "failed"
        && task["error"].is_null()
        && task["result"]["exit_code"]
            .as_i64()
            .is_some_and(|code| code != 0)
}

/// Filter groups: "active" is still running or waiting; "attention" needs a
/// human look and leaves out commands that merely exited non-zero.
fn status_matches(filter: &str, task: &Value) -> bool {
    let status = task["status"].as_str().unwrap_or("unknown");
    match filter {
        "" | "all" => true,
        "active" => matches!(status, "accepted" | "running" | "awaiting_approval"),
        "attention" => {
            matches!(status, "failed" | "timed_out" | "unknown" | "denied") && !exited_nonzero(task)
        }
        _ => status == filter,
    }
}
pub fn active(task: &Value) -> bool {
    matches!(
        task["status"].as_str(),
        Some("accepted" | "running" | "awaiting_approval")
    )
}

fn summary(task: &Value) -> Value {
    let mut row = json!({});
    for field in [
        "task_id",
        "kind",
        "status",
        "arguments",
        "started_at",
        "ended_at",
        "error",
        "progress",
        "approval_deadline",
        "desktop_tier",
        "approved_by_rule",
    ] {
        if let Some(value) = task.get(field) {
            row[field] = value.clone();
        }
    }
    if !row["arguments"].is_object() {
        row["arguments"] = json!({});
    }
    if let Some(env) = row["arguments"]["env"].as_object_mut() {
        for value in env.values_mut() {
            *value = json!("••••");
        }
    }
    if let Some(code) = task["result"]["exit_code"].as_i64() {
        row["result"] = json!({"exit_code": code});
    }
    row
}

fn position(task: &Value) -> (u64, &str) {
    (
        task["started_at"].as_u64().unwrap_or(0),
        task["task_id"].as_str().unwrap_or(""),
    )
}

impl TaskHistory {
    pub fn new(data: PathBuf) -> Self {
        Self {
            data,
            cache: Default::default(),
        }
    }

    pub async fn records(&self) -> Result<Vec<Value>> {
        let history = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut cache = history
                .cache
                .lock()
                .map_err(|_| anyhow::anyhow!("task history unavailable"))?;
            let mut present = BTreeSet::new();
            for entry in std::fs::read_dir(history.data.join("tasks"))? {
                let entry = entry?;
                let ident = entry.file_name().to_string_lossy().into_owned();
                if uuid::Uuid::parse_str(&ident).is_err() {
                    continue;
                }
                let path = entry.path().join("result.json");
                let Ok(metadata) = std::fs::metadata(&path) else {
                    continue;
                };
                present.insert(ident.clone());
                if cache.get(&ident).is_some_and(|cached| {
                    cached.modified == metadata.modified().ok() && cached.length == metadata.len()
                }) {
                    continue;
                }
                if let Ok(bytes) = std::fs::read(&path)
                    && let Ok(task) = serde_json::from_slice::<Value>(&bytes)
                {
                    cache.insert(
                        ident,
                        CachedTask {
                            modified: metadata.modified().ok(),
                            length: metadata.len(),
                            value: summary(&task),
                        },
                    );
                } else {
                    cache.remove(&ident);
                }
            }
            cache.retain(|id, _| present.contains(id));
            let mut records: Vec<_> = cache.values().map(|cached| cached.value.clone()).collect();
            records.sort_by(|a, b| position(b).cmp(&position(a)));
            Ok(records)
        })
        .await?
    }

    pub async fn page(&self, args: Value) -> Result<Value> {
        let query: Query = serde_json::from_value(args)?;
        if ![
            "",
            "all",
            "active",
            "attention",
            "accepted",
            "running",
            "awaiting_approval",
            "succeeded",
            "failed",
            "cancelled",
            "timed_out",
            "unknown",
            "denied",
        ]
        .contains(&query.status.as_str())
        {
            bail!("unknown task status");
        }
        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let needle = query.query.trim().to_lowercase();
        let records = self.records().await?;
        let total = records.len();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut matches = Vec::new();
        for task in records {
            if query
                .kind
                .as_ref()
                .is_some_and(|kind| task["kind"] != *kind)
            {
                continue;
            }
            let args = &task["arguments"];
            let text = [
                task["task_id"].as_str(),
                task["kind"].as_str(),
                args["command"].as_str(),
                args["tool"].as_str(),
                args["cwd"].as_str(),
                args["remote_root"].as_str(),
                args["path"].as_str(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
            if !text.contains(&needle) {
                continue;
            }
            let status = task["status"].as_str().unwrap_or("unknown");
            *counts.entry(status.to_owned()).or_default() += 1;
            // "exited" is a subset of "failed", not another status: clients
            // subtract it from the attention group instead of adding it to totals.
            if exited_nonzero(&task) {
                *counts.entry("exited".to_owned()).or_default() += 1;
            }
            if status_matches(&query.status, &task) {
                matches.push(task);
            }
        }
        let filtered_total = matches.len();
        let mut rows: Vec<_> = matches
            .into_iter()
            .filter(|task| {
                query.cursor.as_ref().is_none_or(|cursor| {
                    position(task) < (cursor.started_at, cursor.task_id.as_str())
                })
            })
            .take(limit + 1)
            .collect();
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = if more {
            rows.last().map(|task| Cursor {
                started_at: position(task).0,
                task_id: position(task).1.to_owned(),
            })
        } else {
            None
        };
        Ok(
            json!({"tasks":rows,"total":total,"filtered_total":filtered_total,"counts":counts,"next_cursor":next_cursor}),
        )
    }
}
