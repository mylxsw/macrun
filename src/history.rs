//! Read-only task history. Cache summaries, never tool images or command output.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::SystemTime,
};

#[derive(Clone)]
pub struct TaskHistory {
    data: PathBuf,
    cache: Arc<Mutex<Index>>,
}
#[derive(Serialize, Deserialize)]
struct CachedTask {
    modified: Option<SystemTime>,
    length: u64,
    value: Value,
}

#[derive(Default)]
struct Index {
    rows: BTreeMap<String, CachedTask>,
    ordered: Option<Arc<Vec<Value>>>,
    loaded: bool,
    scanned: Option<std::time::Instant>,
    checkpointed: Option<std::time::Instant>,
    dirty: bool,
}
static INDEXES: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Mutex<Index>>>>> = OnceLock::new();
fn index(data: &Path) -> Arc<Mutex<Index>> {
    let mut indexes = INDEXES.get_or_init(Default::default).lock().unwrap();
    indexes.retain(|_, v| v.strong_count() > 0);
    if let Some(existing) = indexes.get(data).and_then(Weak::upgrade) {
        return existing;
    }
    let cache = Arc::new(Mutex::new(Index::default()));
    indexes.insert(data.into(), Arc::downgrade(&cache));
    cache
}
/// Called after a durable task record replacement; no image/output enters the index.
pub(crate) fn record_written(path: &Path, task: &Value) {
    if path.file_name().is_none_or(|n| n != "result.json") {
        return;
    }
    let Some(dir) = path.parent() else { return };
    let Some(tasks) = dir
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "tasks"))
    else {
        return;
    };
    let Some(data) = tasks.parent() else { return };
    let Some(cache) = INDEXES
        .get()
        .and_then(|m| m.lock().ok()?.get(data)?.upgrade())
    else {
        return;
    };
    let Some(ident) = dir.file_name().and_then(|v| v.to_str()) else {
        return;
    };
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut cache = cache.lock().unwrap();
        cache.rows.insert(
            ident.into(),
            CachedTask {
                modified: metadata.modified().ok(),
                length: metadata.len(),
                value: summary(task),
            },
        );
        cache.dirty = true;
        cache.ordered = None;
    }
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
            cache: index(&data),
            data,
        }
    }

    pub async fn records(&self) -> Result<Vec<Value>> {
        Ok(self.shared_records().await?.as_ref().clone())
    }

    /// Reuse the sorted snapshot until a durable task update invalidates it.
    pub async fn shared_records(&self) -> Result<Arc<Vec<Value>>> {
        let history = self.clone();
        crate::wire::blocking(move || {
            let mut cache = history
                .cache
                .lock()
                .map_err(|_| anyhow::anyhow!("task history unavailable"))?;
            if !cache.loaded {
                if let Ok(bytes) = std::fs::read(history.data.join("history-index.json"))
                    && let Ok(rows) = serde_json::from_slice::<BTreeMap<String, CachedTask>>(&bytes)
                {
                    // Updates made before the first query override checkpoint rows.
                    let fresh = std::mem::replace(&mut cache.rows, rows);
                    cache.rows.extend(fresh);
                }
                cache.loaded = true;
            }
            if cache
                .scanned
                .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(300))
            {
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
                    if cache.rows.get(&ident).is_some_and(|cached| {
                        cached.modified == metadata.modified().ok()
                            && cached.length == metadata.len()
                    }) {
                        continue;
                    }
                    if let Ok(bytes) = std::fs::read(&path)
                        && let Ok(task) = serde_json::from_slice::<Value>(&bytes)
                    {
                        cache.rows.insert(
                            ident,
                            CachedTask {
                                modified: metadata.modified().ok(),
                                length: metadata.len(),
                                value: summary(&task),
                            },
                        );
                    } else {
                        cache.rows.remove(&ident);
                    }
                }
                cache.rows.retain(|id, _| present.contains(id));
                cache.scanned = Some(std::time::Instant::now());
                cache.dirty = true;
                cache.ordered = None;
            }
            if cache.dirty
                && cache
                    .checkpointed
                    .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(30))
            {
                crate::wire::atomic_json(&history.data.join("history-index.json"), &cache.rows)?;
                cache.checkpointed = Some(std::time::Instant::now());
                cache.dirty = false;
            }
            if cache.ordered.is_none() {
                let mut records: Vec<_> = cache.rows.values().map(|c| c.value.clone()).collect();
                records.sort_by(|a, b| position(b).cmp(&position(a)));
                cache.ordered = Some(Arc::new(records));
            }
            Ok(cache.ordered.as_ref().unwrap().clone())
        })
        .await?
    }

    pub fn removed(&self, ident: &str) {
        let mut cache = self.cache.lock().unwrap();
        cache.rows.remove(ident);
        cache.dirty = true;
        cache.ordered = None;
    }
    pub async fn checkpoint(&self) -> Result<()> {
        let history = self.clone();
        crate::wire::blocking(move || {
            let cache = history.cache.lock().unwrap();
            crate::wire::atomic_json(&history.data.join("history-index.json"), &cache.rows)
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
        let records = self.shared_records().await?;
        let total = records.len();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut matches = Vec::new();
        for task in records.iter() {
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
            if exited_nonzero(task) {
                *counts.entry("exited".to_owned()).or_default() += 1;
            }
            if status_matches(&query.status, task) {
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
            .cloned()
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
