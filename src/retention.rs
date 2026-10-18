use anyhow::{Result, ensure};
use serde_json::Value;
use std::path::Path;
fn old(path: &Path, cutoff: u64) -> bool {
    std::fs::symlink_metadata(path).ok().is_some_and(|m| {
        m.is_file()
            && m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .is_some_and(|d| d.as_millis() < cutoff as u128)
    })
}
pub fn transfers(task: &Path, cutoff: u64) -> Result<bool> {
    let registry = task.join("transfer.json");
    if !registry.exists() {
        return Ok(false);
    }
    ensure!(
        std::fs::symlink_metadata(&registry)?.is_file(),
        "invalid transfer registry"
    );
    let record: Value = serde_json::from_slice(&std::fs::read(&registry)?)?;
    let destination = Path::new(crate::files::string(&record, "destination")?);
    let id = crate::files::string(&record, "transfer_id")?;
    uuid::Uuid::parse_str(id)?;
    let part = destination.with_file_name(format!(
        "{}.{}.macrun-part",
        destination
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("invalid transfer path"))?
            .to_string_lossy(),
        id
    ));
    let meta = part.with_extension("macrun-meta");
    let Ok(_lease) = crate::workspace::Lease::acquire(&[destination.into()]) else {
        return Ok(false);
    };
    if !part.try_exists()? && !meta.try_exists()? {
        std::fs::remove_file(registry)?;
        return Ok(false);
    }
    if !old(&meta, cutoff) || (part.exists() && !old(&part, cutoff)) {
        return Ok(false);
    }
    let identity: Value = serde_json::from_slice(&std::fs::read(&meta)?)?;
    ensure!(
        identity["path"] == record["destination"],
        "transfer cleanup identity mismatch"
    );
    if part.exists() {
        std::fs::remove_file(part)?;
    }
    std::fs::remove_file(meta)?;
    std::fs::remove_file(registry)?;
    Ok(true)
}
pub fn sync_jobs(data: &Path, cutoff: u64) -> Result<usize> {
    let root = data.join("sync-jobs");
    if !root.exists() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir()
            || uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err()
        {
            continue;
        }
        let path = entry.path();
        if let Ok(bytes) = std::fs::read(path.join("result.json"))
            && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
            && v["ended_at"].as_u64().is_some_and(|t| t < cutoff)
        {
            std::fs::remove_dir_all(path)?;
            removed += 1;
        }
        if removed >= 1000 {
            break;
        }
    }
    Ok(removed)
}
pub fn incoming(root: &Path, cutoff: u64) -> Result<()> {
    let dir = root.join(".macrun/incoming");
    for path in [root.join(".macrun"), dir.clone()] {
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            ensure!(
                metadata.is_dir(),
                "reserved sync directory is not a real directory"
            );
        }
    }
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_ok()
            && old(&entry.path(), cutoff)
        {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Usage is logical file bytes, excludes symlink targets, and is sampled at cleanup time.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TaskStorage {
    pub scanned_at: u64,
    pub task_bytes: u64,
    pub workspace_bytes: u64,
    pub limit_bytes: u64,
    pub over_limit: bool,
    pub removed: usize,
    pub workspaces_removed: usize,
    pub transfers_removed: usize,
    pub reclaimed_bytes: u64,
    pub errors: usize,
    #[serde(skip)]
    pub removed_ids: Vec<String>,
}

fn directory_bytes(path: &Path) -> Result<u64> {
    let mut bytes = 0u64;
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file() {
            bytes = bytes.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(bytes)
}

struct StoredTask {
    path: std::path::PathBuf,
    record: Value,
    bytes: u64,
    workspace_bytes: u64,
}

fn terminal(record: &Value) -> bool {
    matches!(
        record["status"].as_str(),
        Some("succeeded" | "failed" | "cancelled" | "timed_out" | "denied" | "unknown")
    )
}

/// Only real, UUID-named task directories with a matching terminal record are eligible.
/// Incomplete process recovery, active operations and malformed records fail closed.
pub fn task_storage(
    data: &Path,
    safety: &crate::safety::Safety,
    active: &std::collections::BTreeSet<String>,
    timestamp: u64,
) -> Result<TaskStorage> {
    let root = data.join("tasks");
    ensure!(
        std::fs::symlink_metadata(&root)?.is_dir(),
        "task storage must be a real directory"
    );
    let mut report = TaskStorage {
        scanned_at: timestamp,
        limit_bytes: u64::from(safety.storage_limit_gb) * 1024 * 1024 * 1024,
        ..Default::default()
    };
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        // Never follow a top-level symlink, even if it looks like a task ID.
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        let bytes = match directory_bytes(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };
        report.task_bytes = report.task_bytes.saturating_add(bytes);
        let ident = entry.file_name().to_string_lossy().into_owned();
        let workspace = path.join("workspace");
        let workspace_bytes = if std::fs::symlink_metadata(&workspace).is_ok_and(|m| m.is_dir()) {
            match directory_bytes(&workspace) {
                Ok(bytes) => bytes,
                Err(_) => {
                    report.errors += 1;
                    continue;
                }
            }
        } else {
            0
        };
        report.workspace_bytes = report.workspace_bytes.saturating_add(workspace_bytes);
        if uuid::Uuid::parse_str(&ident).is_err() || active.contains(&ident) {
            continue;
        }
        let eligible = (|| -> Result<Value> {
            let record_path = match std::fs::symlink_metadata(path.join("result.json")) {
                Ok(meta) => {
                    ensure!(meta.is_file(), "invalid task record");
                    path.join("result.json")
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let dedup = data.join("dedup");
                    ensure!(
                        std::fs::symlink_metadata(&dedup)?.is_dir(),
                        "invalid dedup directory"
                    );
                    let tombstone = dedup.join(format!("{ident}.json"));
                    ensure!(
                        std::fs::symlink_metadata(&tombstone)?.is_file(),
                        "invalid dedup record"
                    );
                    tombstone
                }
                Err(e) => return Err(e.into()),
            };
            let record: Value = serde_json::from_slice(&std::fs::read(record_path)?)?;
            ensure!(record["task_id"] == ident, "task identity mismatch");
            Ok(record)
        })();
        let record = match eligible {
            Ok(record) => record,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };
        if !terminal(&record)
            || !record["ended_at"]
                .as_u64()
                .is_some_and(|end| end <= timestamp)
            || path.join("process.json").try_exists()?
        {
            continue;
        }
        // Hold cleanup access only while mutating this task, so other downloads
        // remain available while a large history is scanned.
        let Ok(_lease) = crate::workspace::Lease::for_cleanup(std::slice::from_ref(&path)) else {
            continue;
        };
        if record["kind"] == "file.upload" {
            match transfers(&path, timestamp.saturating_sub(86_400_000)) {
                Ok(removed) => report.transfers_removed += usize::from(removed),
                Err(_) => report.errors += 1,
            }
            // Keep ownership information while a resumable upload is still recent or busy.
            if path.join("transfer.json").try_exists()? {
                continue;
            }
        }
        // Keep only dedup metadata in memory, never all task outputs/images.
        let record = serde_json::json!({"task_id":record["task_id"],"kind":record["kind"],"request_fingerprint":record["request_fingerprint"],"status":record["status"],"started_at":record["started_at"],"ended_at":record["ended_at"]});
        candidates.push(StoredTask {
            path,
            record,
            bytes,
            workspace_bytes,
        });
    }
    candidates.sort_by_key(|task| (task.record["ended_at"].as_u64().unwrap(), task.path.clone()));
    let history_cutoff = timestamp.saturating_sub(u64::from(safety.retention_days) * 86_400_000);
    let snapshot_cutoff =
        timestamp.saturating_sub(u64::from(safety.snapshot_retention_days) * 86_400_000);
    // Preserve results and fresh snapshots for one hour so clients can collect them.
    let budget_cutoff = timestamp.saturating_sub(3_600_000);
    // Reclaim bulky execution copies before sacrificing any task history.
    for task in &mut candidates {
        let end = task.record["ended_at"].as_u64().unwrap();
        let expired = end < snapshot_cutoff;
        let pressure = report.task_bytes > report.limit_bytes && end < budget_cutoff;
        let workspace = task.path.join("workspace");
        // A symlink named workspace is unlinked only with its whole expired task.
        if (expired || pressure) && std::fs::symlink_metadata(&workspace).is_ok_and(|m| m.is_dir())
        {
            let Ok(_lease) = crate::workspace::Lease::for_cleanup(std::slice::from_ref(&task.path))
            else {
                continue;
            };
            let result = (|| -> Result<()> {
                let path = task.path.join("result.json");
                let mut record: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
                // Expire the reference first. Failed deletion is retried on the next sweep.
                record["workspace_expired"] = serde_json::json!(true);
                crate::wire::atomic_json(&path, &record)?;
                std::fs::remove_dir_all(&workspace)?;
                Ok(())
            })();
            if result.is_err() {
                report.errors += 1;
            }
            if !workspace.exists() {
                report.workspaces_removed += 1;
            }
            update_usage(task, &mut report);
        }
    }
    for task in &mut candidates {
        let end = task.record["ended_at"].as_u64().unwrap();
        if end >= history_cutoff && !(report.task_bytes > report.limit_bytes && end < budget_cutoff)
        {
            continue;
        }
        let Ok(_lease) = crate::workspace::Lease::for_cleanup(std::slice::from_ref(&task.path))
        else {
            continue;
        };
        let had_workspace =
            std::fs::symlink_metadata(task.path.join("workspace")).is_ok_and(|m| m.is_dir());
        let result = (|| -> Result<()> {
            let record = &task.record;
            let ident = record["task_id"].as_str().unwrap();
            let dedup = data.join("dedup");
            if let Ok(meta) = std::fs::symlink_metadata(&dedup) {
                ensure!(meta.is_dir(), "dedup storage must be a real directory");
            }
            let tombstone = serde_json::json!({"task_id":ident,"kind":record["kind"],"request_fingerprint":record["request_fingerprint"],"status":record["status"],"started_at":record["started_at"],"ended_at":record["ended_at"],"history_expired":true});
            // Persist dedup before removing data. A retry can never re-execute an expired request.
            crate::wire::atomic_json(&dedup.join(format!("{ident}.json")), &tombstone)?;
            std::fs::remove_dir_all(&task.path)?;
            Ok(())
        })();
        if result.is_err() {
            report.errors += 1;
        }
        if !task.path.exists() {
            report.workspaces_removed += usize::from(had_workspace);
            report.removed += 1;
            report
                .removed_ids
                .push(task.record["task_id"].as_str().unwrap().into());
        }
        update_usage(task, &mut report);
    }
    for task in &mut candidates {
        if task.record["ended_at"].as_u64().unwrap() >= timestamp.saturating_sub(7 * 86_400_000) {
            continue;
        }
        let path = task.path.join("result.json");
        if !path.exists() {
            continue;
        }
        let Ok(_lease) = crate::workspace::Lease::for_cleanup(std::slice::from_ref(&task.path))
        else {
            continue;
        };
        let result = (|| -> Result<()> {
            let mut record: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            if crate::metrics::prune_samples(&mut record, timestamp.saturating_sub(7 * 86_400_000))
            {
                crate::wire::atomic_json(&path, &record)?;
            }
            Ok(())
        })();
        if result.is_err() {
            report.errors += 1;
        }
        update_usage(task, &mut report);
    }
    report.over_limit = report.task_bytes > report.limit_bytes;
    Ok(report)
}

fn update_usage(task: &mut StoredTask, report: &mut TaskStorage) {
    let measured = (|| -> Result<(u64, u64)> {
        let bytes = if task.path.exists() {
            directory_bytes(&task.path)?
        } else {
            0
        };
        let workspace = task.path.join("workspace");
        let workspace_bytes = if std::fs::symlink_metadata(&workspace).is_ok_and(|m| m.is_dir()) {
            directory_bytes(&workspace)?
        } else {
            0
        };
        Ok((bytes, workspace_bytes))
    })();
    let (bytes, workspace_bytes) = match measured {
        Ok(usage) => usage,
        Err(_) => {
            report.errors += 1;
            return;
        }
    };
    report.reclaimed_bytes = report
        .reclaimed_bytes
        .saturating_add(task.bytes.saturating_sub(bytes));
    report.task_bytes = report
        .task_bytes
        .saturating_sub(task.bytes)
        .saturating_add(bytes);
    report.workspace_bytes = report
        .workspace_bytes
        .saturating_sub(task.workspace_bytes)
        .saturating_add(workspace_bytes);
    task.workspace_bytes = workspace_bytes;
    task.bytes = bytes;
}
