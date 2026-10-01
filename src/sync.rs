use crate::{
    model::{Fault, Outcome},
    wire,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Component, Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    File {
        size: u64,
        hash: String,
        executable: bool,
    },
    Directory,
    Link {
        target: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Manifest {
    pub entries: BTreeMap<String, Entry>,
    pub skipped: Vec<String>,
}
fn err(e: impl ToString) -> Fault {
    Fault::new("sync_failed", e)
}
pub fn relative(path: &str) -> Outcome<PathBuf> {
    let p = Path::new(path);
    if path.is_empty() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(Fault::new("path_conflict", path));
    }
    Ok(p.into())
}
pub fn hash(path: &Path) -> Outcome<String> {
    let mut r = fs::File::open(path).map_err(err)?;
    let mut h = blake3::Hasher::new();
    let mut b = [0; 65536];
    loop {
        let n = r.read(&mut b).map_err(err)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(h.finalize().to_hex().to_string())
}
pub fn scan(root: &Path, excludes: &[String]) -> Outcome<Manifest> {
    let root = fs::canonicalize(root).map_err(err)?;
    let mut patterns = globset::GlobSetBuilder::new();
    for p in excludes {
        patterns.add(globset::Glob::new(p).map_err(|e| Fault::new("invalid_config", e))?);
    }
    let patterns = patterns.build().map_err(err)?;
    let default = [".git", ".build", "DerivedData", ".macrun", "target"];
    let mut out = Manifest::default();
    let mut lower = BTreeSet::new();
    let iter = walkdir::WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let rel = e.path().strip_prefix(&root).unwrap();
            !default.iter().any(|n| e.file_name() == *n) && !patterns.is_match(rel)
        });
    for item in iter {
        let item = item.map_err(err)?;
        if item.depth() == 0 {
            continue;
        }
        let path = item.path();
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_str()
            .ok_or_else(|| Fault::new("path_conflict", "non-UTF8 path"))?
            .to_string();
        if !lower.insert(rel.to_lowercase()) {
            return Err(Fault::new(
                "path_conflict",
                format!("case collision: {rel}"),
            ));
        }
        let meta = fs::symlink_metadata(path).map_err(err)?;
        let entry = if meta.is_dir() {
            Entry::Directory
        } else if meta.file_type().is_symlink() {
            let resolved = match fs::canonicalize(path) {
                Ok(p) => p,
                Err(_) => {
                    out.skipped.push(rel);
                    continue;
                }
            };
            if !resolved.starts_with(&root) {
                out.skipped.push(rel);
                continue;
            }
            let original = fs::read_link(path).map_err(err)?;
            let target = if original.is_absolute() {
                pathdiff::diff_paths(&resolved, path.parent().unwrap()).unwrap()
            } else {
                original
            };
            Entry::Link {
                target: target.to_str().ok_or_else(|| err("non-UTF8 link"))?.into(),
            }
        } else if meta.is_file() {
            let h = hash(path)?;
            let after = fs::metadata(path).map_err(err)?;
            if meta.len() != after.len() || meta.modified().ok() != after.modified().ok() {
                return Err(Fault::new("source_changed", &rel));
            }
            Entry::File {
                size: meta.len(),
                hash: h,
                executable: meta.permissions().mode() & 0o111 != 0,
            }
        } else {
            out.skipped.push(rel);
            continue;
        };
        out.entries.insert(rel, entry);
    }
    Ok(out)
}
fn read_manifest(path: &Path) -> Outcome<Manifest> {
    if !path.exists() {
        return Ok(Manifest::default());
    }
    serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)
}
pub struct Receiver {
    pub root: PathBuf,
    pub state: PathBuf,
    pub manifest: Manifest,
    pub needed: Vec<String>,
}
impl Receiver {
    pub fn prepare(root: &Path, state: &Path, manifest: Manifest) -> Outcome<Self> {
        fs::create_dir_all(root).map_err(err)?;
        fs::create_dir_all(state).map_err(err)?;
        let old = read_manifest(&state.join("manifest.json"))?;
        let pending = read_manifest(&state.join("sync_incomplete.json"))?;
        let mut managed = old.entries.clone();
        managed.extend(pending.entries);
        let mut plan = Manifest {
            entries: managed.clone(),
            skipped: vec![],
        };
        plan.entries.extend(manifest.entries.clone());
        // Persist ownership before any mutation, including new files in interrupted first syncs.
        wire::atomic_json(&state.join("sync_incomplete.json"), &plan).map_err(err)?;
        let mut obsolete: Vec<_> = managed
            .keys()
            .filter(|p| match manifest.entries.get(*p) {
                None => true,
                Some(expected) => std::fs::symlink_metadata(root.join(p)).ok().is_some_and(
                    |meta| match expected {
                        Entry::Directory => !meta.is_dir(),
                        Entry::File { .. } => !meta.is_file(),
                        Entry::Link { .. } => !meta.file_type().is_symlink(),
                    },
                ),
            })
            .cloned()
            .collect();
        obsolete.sort_by_key(|p| std::cmp::Reverse(p.matches('/').count()));
        for p in obsolete {
            let dest = root.join(relative(&p)?);
            ensure_parents(root, &dest)?;
            match fs::symlink_metadata(&dest) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(err(e)),
                Ok(m) if m.is_dir() => {
                    fs::remove_dir(&dest).map_err(|e| {
                        Fault::new(
                            "path_conflict",
                            format!("preserving unmanaged content at {p}: {e}"),
                        )
                    })?;
                }
                Ok(_) => fs::remove_file(dest).map_err(err)?,
            }
        }
        let mut needed = vec![];
        for (p, e) in &manifest.entries {
            let dest = root.join(relative(p)?);
            ensure_parents(root, &dest)?;
            match e {
                Entry::Directory => {
                    fs::create_dir_all(&dest).map_err(|e| Fault::new("path_conflict", e))?;
                }
                Entry::Link { target } => {
                    if dest.exists() || fs::symlink_metadata(&dest).is_ok() {
                        if fs::read_link(&dest).ok() == Some(PathBuf::from(target)) {
                            continue;
                        }
                        if !managed.contains_key(p) {
                            return Err(Fault::new("path_conflict", p));
                        }
                        fs::remove_file(&dest).map_err(err)?;
                    }
                    symlink(target, dest).map_err(err)?;
                }
                Entry::File {
                    hash: expected,
                    executable,
                    ..
                } => {
                    let meta = fs::symlink_metadata(&dest).ok();
                    if let Some(m) = &meta
                        && !m.is_file()
                    {
                        return Err(Fault::new("path_conflict", p));
                    }
                    if meta.is_some() && hash(&dest)? == *expected {
                        fs::set_permissions(
                            &dest,
                            fs::Permissions::from_mode(if *executable { 0o755 } else { 0o644 }),
                        )
                        .map_err(err)?;
                    } else {
                        needed.push(p.clone());
                    }
                }
            }
        }
        Ok(Self {
            root: root.into(),
            state: state.into(),
            manifest,
            needed,
        })
    }
    pub fn install(&self, path: &str, temp: &Path) -> Outcome<()> {
        let Some(Entry::File {
            size,
            hash: expected,
            executable,
        }) = self.manifest.entries.get(path)
        else {
            return Err(err("unexpected file"));
        };
        if fs::metadata(temp).map_err(err)?.len() != *size || hash(temp)? != *expected {
            return Err(Fault::new("source_changed", path));
        }
        let dest = self.root.join(relative(path)?);
        ensure_parents(&self.root, &dest)?;
        fs::set_permissions(
            temp,
            fs::Permissions::from_mode(if *executable { 0o755 } else { 0o644 }),
        )
        .map_err(err)?;
        // Temporary files live in the mirror directory so rename stays on one filesystem.
        fs::rename(temp, dest).map_err(err)?;
        Ok(())
    }
    pub fn commit(&self, confirmed: &Manifest) -> Outcome<()> {
        if confirmed != &self.manifest {
            return Err(Fault::new(
                "source_changed",
                "workspace changed during sync",
            ));
        }
        for (p, e) in &self.manifest.entries {
            if let Entry::File { hash: h, .. } = e
                && hash(&self.root.join(p))? != *h
            {
                return Err(err(format!("incomplete file {p}")));
            }
        }
        wire::atomic_json(&self.state.join("manifest.json"), &self.manifest).map_err(err)?;
        fs::remove_file(self.state.join("sync_incomplete.json")).map_err(err)?;
        Ok(())
    }
}
fn ensure_parents(root: &Path, dest: &Path) -> Outcome<()> {
    let rel = dest.strip_prefix(root).map_err(err)?;
    let mut p = root.to_path_buf();
    let parts: Vec<_> = rel.components().collect();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        p.push(part.as_os_str());
        if let Ok(m) = fs::symlink_metadata(&p)
            && !m.is_dir()
        {
            return Err(Fault::new(
                "path_conflict",
                format!("non-directory parent: {}", p.display()),
            ));
        }
    }
    Ok(())
}
