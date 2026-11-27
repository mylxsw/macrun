//! A task-owned copy of the committed input generation; never a hard link.
use crate::{
    sync::{self, Entry},
    workspace,
};
use anyhow::{Result, ensure};
use std::{
    fs,
    io::ErrorKind,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

#[derive(serde::Serialize, serde::Deserialize)]
struct TaskWorkspace {
    work: PathBuf,
    profile: String,
}

/// Keep the snapshot next to the other work, and its ownership record with
/// the task. The profile namespace lets several connections share one folder.
pub(crate) fn prepare_task_workspace(
    work: &Path,
    task: &Path,
    profile_key: &[u8; 32],
) -> Result<PathBuf> {
    let marker = TaskWorkspace {
        work: work.to_path_buf(),
        profile: blake3::hash(profile_key).to_hex().to_string(),
    };
    let parent = task_workspace_parent(&marker, task, true)?;
    fs::create_dir(&parent)?;
    if let Err(error) = crate::wire::atomic_json(&task.join("snapshot-root.json"), &marker) {
        let _ = fs::remove_dir(&parent);
        return Err(error);
    }
    Ok(parent.join("workspace"))
}

pub(crate) fn task_workspace(task: &Path) -> Result<PathBuf> {
    let marker = task.join("snapshot-root.json");
    match fs::symlink_metadata(&marker) {
        Ok(meta) => ensure!(meta.is_file(), "invalid task snapshot record"),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(task.join("workspace")),
        Err(error) => return Err(error.into()),
    }
    let marker: TaskWorkspace = serde_json::from_slice(&fs::read(marker)?)?;
    let parent = task_workspace_parent(&marker, task, false)?;
    if let Ok(meta) = fs::symlink_metadata(&parent) {
        ensure!(meta.is_dir(), "invalid task snapshot directory");
    }
    Ok(parent.join("workspace"))
}

fn task_workspace_parent(marker: &TaskWorkspace, task: &Path, create: bool) -> Result<PathBuf> {
    let TaskWorkspace { work, profile } = marker;
    ensure!(work.is_absolute(), "invalid work directory");
    ensure!(
        profile.len() == 64 && profile.bytes().all(|value| value.is_ascii_hexdigit()),
        "invalid task snapshot profile"
    );
    let id = task
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    uuid::Uuid::parse_str(id)?;
    if work.try_exists()? {
        ensure!(fs::canonicalize(work)? == *work, "work directory changed");
    }
    let mut path = work.to_path_buf();
    for part in [".macrun", "snapshots", profile.as_str()] {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) => ensure!(meta.is_dir(), "invalid task snapshot directory"),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                if create {
                    match fs::create_dir(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                            ensure!(
                                fs::symlink_metadata(&path)?.is_dir(),
                                "invalid task snapshot directory"
                            );
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path.join(id))
}

pub fn create(root: &Path, state: &Path, destination: &Path, expected: &str) -> Result<()> {
    ensure!(workspace::generation(root)? == expected, "stale_workspace");
    let manifest = sync::read_manifest(&state.join("manifest.json"))?;
    ensure!(
        sync::digest(&manifest)? == expected,
        "snapshot manifest generation differs"
    );
    ensure!(!destination.exists(), "snapshot destination already exists");
    std::fs::create_dir_all(destination)?;
    let canonical_root = std::fs::canonicalize(root)?;
    let result = (|| -> Result<()> {
        for (path, entry) in &manifest.entries {
            let relative = sync::relative(path)?;
            let source = root.join(&relative);
            ensure!(
                std::fs::canonicalize(&source)?.starts_with(&canonical_root),
                "snapshot source escapes workspace"
            );
            let target = destination.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match entry {
                Entry::Directory => std::fs::create_dir_all(&target)?,
                Entry::File {
                    hash,
                    size,
                    executable,
                } => {
                    ensure!(
                        std::fs::symlink_metadata(&source)?.is_file(),
                        "snapshot source type changed"
                    );
                    ensure!(
                        std::fs::copy(&source, &target)? == *size && sync::hash(&target)? == *hash,
                        "source_changed during snapshot"
                    );
                    std::fs::set_permissions(
                        &target,
                        std::fs::Permissions::from_mode(if *executable { 0o755 } else { 0o644 }),
                    )?;
                }
                Entry::Link { target: link } => {
                    ensure!(
                        std::fs::read_link(&source)? == Path::new(link),
                        "snapshot link changed"
                    );
                    // Sync normalizes internal links to relative links. Keep the
                    // snapshot self-contained and reject untrusted absolute links.
                    ensure!(!Path::new(link).is_absolute(), "absolute snapshot link");
                    symlink(link, &target)?;
                }
            }
        }
        ensure!(
            workspace::generation(root)? == expected,
            "snapshot generation changed"
        );
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(destination);
    }
    result
}
