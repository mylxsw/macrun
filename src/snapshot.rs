//! A task-owned copy of the committed input generation; never a hard link.
use crate::{
    sync::{self, Entry},
    workspace,
};
use anyhow::{Result, ensure};
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
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
