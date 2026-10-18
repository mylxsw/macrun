//! Process-wide leases include overlapping roots and existing symlink aliases.
//! They coordinate Macrun operations, not arbitrary external filesystem writers.
use crate::model::Fault;
use anyhow::Result;
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Exclusive,
    Pin,
    Cleanup,
}
type Held = Vec<(String, Vec<PathBuf>, Mode)>;
static LEASES: OnceLock<Mutex<Held>> = OnceLock::new();
pub struct Lease(String);

pub fn resolve(path: &Path) -> Result<PathBuf> {
    // Match filesystem APIs: relative paths are relative to this process.
    // Canonicalize before comparing so existing symlinks and `..` are respected.
    let absolute = std::path::absolute(path)?;
    let mut ancestor = absolute.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("invalid workspace"))?
                .to_owned(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid workspace"))?;
    }
    let mut root = std::fs::canonicalize(ancestor)?;
    for part in suffix.into_iter().rev() {
        root.push(part);
    }
    Ok(root)
}

impl Lease {
    pub fn acquire(paths: &[PathBuf]) -> Result<Self> {
        Self::acquire_as(paths, Mode::Exclusive)
    }
    /// Keep data alive for reads without serializing ordinary readers and writers.
    pub fn pin(paths: &[PathBuf]) -> Result<Self> {
        Self::acquire_as(paths, Mode::Pin)
    }
    pub(crate) fn for_cleanup(paths: &[PathBuf]) -> Result<Self> {
        Self::acquire_as(paths, Mode::Cleanup)
    }
    fn acquire_as(paths: &[PathBuf], mode: Mode) -> Result<Self> {
        let paths: Vec<_> = paths.iter().map(|p| resolve(p)).collect::<Result<_>>()?;
        let mut held = LEASES
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap();
        if held.iter().any(|(_, active, held_mode)| {
            let conflict = mode == Mode::Cleanup
                || *held_mode == Mode::Cleanup
                || (mode == Mode::Exclusive && *held_mode == Mode::Exclusive);
            conflict
                && active
                    .iter()
                    .any(|a| paths.iter().any(|b| a.starts_with(b) || b.starts_with(a)))
        }) {
            return Err(Fault::new(
                "busy",
                "workspace is in use; retry after the active operation completes",
            )
            .into());
        }
        let id = crate::model::id();
        held.push((id.clone(), paths, mode));
        Ok(Self(id))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        LEASES
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .retain(|(id, _, _)| id != &self.0);
    }
}

pub fn generation(root: &Path) -> Result<String> {
    Ok(std::fs::read_to_string(root.join(".macrun/generation"))?)
}
pub fn invalidate(root: &Path) -> Result<()> {
    match std::fs::remove_file(root.join(".macrun/generation")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_excludes_readers_and_writers_until_guard_is_dropped() {
        let root = tempfile::tempdir().unwrap();
        let paths = [root.path().to_path_buf()];
        let cleanup = Lease::for_cleanup(&paths).unwrap();
        assert!(Lease::pin(&paths).is_err());
        assert!(Lease::acquire(&paths).is_err());
        assert!(Lease::for_cleanup(&paths).is_err());
        drop(cleanup);
        assert!(Lease::pin(&paths).is_ok());
        assert!(Lease::acquire(&paths).is_ok());
    }
}
