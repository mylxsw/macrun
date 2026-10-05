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
