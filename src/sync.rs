use crate::{
    model::{Fault, Outcome},
    wire,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
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
    let _phase = crate::metrics::phase("hash");
    let mut r = fs::File::open(path).map_err(err)?;
    let mut h = blake3::Hasher::new();
    let mut b = [0; 65536];
    loop {
        let n = r.read(&mut b).map_err(err)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
        crate::metrics::add("bytes", "hashed", n as u64);
    }
    Ok(h.finalize().to_hex().to_string())
}
#[derive(Clone, PartialEq, Eq)]
struct Fingerprint {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
    mode: u32,
}
impl From<&fs::Metadata> for Fingerprint {
    fn from(m: &fs::Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            size: m.len(),
            mtime: m.mtime(),
            mtime_ns: m.mtime_nsec(),
            ctime: m.ctime(),
            ctime_ns: m.ctime_nsec(),
            mode: m.mode(),
        }
    }
}
type HashCache = BTreeMap<PathBuf, (Fingerprint, String, std::time::Instant)>;
static HASH_CACHE: std::sync::OnceLock<std::sync::Mutex<HashCache>> = std::sync::OnceLock::new();
/// ctime catches same-size edits with restored mtime. Periodic full verification
/// bounds stale entries on filesystems with unreliable metadata. Strict bypasses it.
pub fn cached_hash(path: &Path, strict: bool) -> Outcome<String> {
    let before = fs::metadata(path).map_err(err)?;
    let key = Fingerprint::from(&before);
    let cache = HASH_CACHE.get_or_init(Default::default);
    if !strict
        && let Some((old, hash, sampled)) = cache.lock().unwrap().get(path)
        && old == &key
        && sampled.elapsed() < std::time::Duration::from_secs(600)
    {
        crate::metrics::add("files", "hash_cache_hits", 1);
        return Ok(hash.clone());
    }
    crate::metrics::add("files", "hash_cache_misses", 1);
    let value = hash(path)?;
    if Fingerprint::from(&fs::metadata(path).map_err(err)?) != key {
        return Err(Fault::new("source_changed", path.display()));
    }
    let mut cache = cache.lock().unwrap();
    if cache.len() >= 200_000 {
        cache.clear();
    }
    cache.insert(path.into(), (key, value.clone(), std::time::Instant::now()));
    Ok(value)
}
pub fn scan(root: &Path, excludes: &[String]) -> Outcome<Manifest> {
    scan_with(root, excludes, true)
}
pub fn scan_with(root: &Path, excludes: &[String], strict: bool) -> Outcome<Manifest> {
    let _phase = crate::metrics::phase("scan");
    crate::metrics::add("files", "scanned", 0);
    crate::metrics::add("bytes", "scanned_logical", 0);
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
            crate::metrics::add("files", "scanned_dirs", 1);
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
                crate::metrics::add("files", "skipped_links", 1);
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
            crate::metrics::add("files", "scanned", 1);
            crate::metrics::add("bytes", "scanned_logical", meta.len());
            let h = cached_hash(path, strict)?;
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
pub fn read_manifest(path: &Path) -> Outcome<Manifest> {
    if !path.exists() {
        return Ok(Manifest::default());
    }
    serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)
}
pub struct Receiver {
    strict: bool,
    pub root: PathBuf,
    pub state: PathBuf,
    pub manifest: Manifest,
    pub needed: Vec<String>,
}
impl Receiver {
    pub fn prepare(root: &Path, state: &Path, manifest: Manifest) -> Outcome<Self> {
        Self::prepare_with(root, state, manifest, true)
    }
    pub fn prepare_with(
        root: &Path,
        state: &Path,
        manifest: Manifest,
        strict: bool,
    ) -> Outcome<Self> {
        fs::create_dir_all(root).map_err(err)?;
        crate::retention::incoming(root, crate::model::now().saturating_sub(86_400_000))
            .map_err(err)?;
        crate::workspace::invalidate(root).map_err(err)?;
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
                    crate::metrics::add("files", "deleted", 1);
                }
                Ok(_) => {
                    fs::remove_file(dest).map_err(err)?;
                    crate::metrics::add("files", "deleted", 1);
                }
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
                    if let Some(meta) = &meta
                        && cached_hash(&dest, strict)? == *expected
                    {
                        let mode = if *executable { 0o755 } else { 0o644 };
                        if meta.permissions().mode() & 0o777 != mode {
                            fs::set_permissions(&dest, fs::Permissions::from_mode(mode))
                                .map_err(err)?;
                        }
                    } else {
                        needed.push(p.clone());
                    }
                }
            }
        }
        Ok(Self {
            strict,
            root: root.into(),
            state: state.into(),
            manifest,
            needed,
        })
    }
    pub fn install(&self, path: &str, temp: &Path) -> Outcome<()> {
        let _phase = crate::metrics::phase("install");
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
        crate::metrics::add("files", "completed", 1);
        crate::metrics::add("bytes", "confirmed_logical", *size);
        Ok(())
    }
    pub fn commit(&self, confirmed: &Manifest) -> Outcome<()> {
        let _phase = crate::metrics::phase("commit");
        if confirmed != &self.manifest {
            return Err(Fault::new(
                "source_changed",
                "workspace changed during sync",
            ));
        }
        for (p, e) in &self.manifest.entries {
            if let Entry::File { hash: h, .. } = e
                && cached_hash(&self.root.join(p), self.strict)? != *h
            {
                return Err(err(format!("incomplete file {p}")));
            }
        }
        wire::atomic_json(&self.state.join("manifest.json"), &self.manifest).map_err(err)?;
        fs::remove_file(self.state.join("sync_incomplete.json")).map_err(err)?;
        fs::create_dir_all(self.root.join(".macrun")).map_err(err)?;
        let generation = blake3::hash(&serde_json::to_vec(&self.manifest).map_err(err)?)
            .to_hex()
            .to_string();
        wire::private_write(&self.root.join(".macrun/generation"), generation.as_bytes())
            .map_err(err)?;
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

/// Bounded frames for large file trees; binary file data follows on the same stream.
pub async fn send_manifest<W: tokio::io::AsyncWrite + Unpin>(
    out: &mut W,
    manifest: &Manifest,
) -> anyhow::Result<()> {
    let entries: Vec<_> = manifest.entries.iter().collect();
    for page in entries.chunks(256) {
        wire::send(
            out,
            &crate::model::Event::ManifestPage {
                entries: page
                    .iter()
                    .map(|(k, v)| ((*k).clone(), (*v).clone()))
                    .collect(),
                skipped: vec![],
                last: false,
            },
        )
        .await?;
    }
    for page in manifest.skipped.chunks(256) {
        wire::send(
            out,
            &crate::model::Event::ManifestPage {
                entries: BTreeMap::new(),
                skipped: page.to_vec(),
                last: false,
            },
        )
        .await?;
    }
    wire::send(
        out,
        &crate::model::Event::ManifestPage {
            entries: BTreeMap::new(),
            skipped: vec![],
            last: true,
        },
    )
    .await
}
pub async fn recv_manifest<R: tokio::io::AsyncRead + Unpin>(
    input: &mut R,
) -> anyhow::Result<Manifest> {
    let mut manifest = Manifest::default();
    let mut bytes = 0usize;
    loop {
        let page = wire::recv::<_, crate::model::Event>(input).await?;
        bytes += serde_json::to_vec(&page)?.len();
        anyhow::ensure!(
            bytes <= 128 * 1024 * 1024,
            "manifest exceeds 128 MiB aggregate limit"
        );
        let crate::model::Event::ManifestPage {
            entries,
            skipped,
            last,
        } = page
        else {
            anyhow::bail!("expected manifest page")
        };
        for (path, entry) in entries {
            relative(&path)?;
            anyhow::ensure!(
                manifest.entries.insert(path, entry).is_none(),
                "duplicate manifest path"
            );
        }
        manifest.skipped.extend(skipped);
        if last {
            return Ok(manifest);
        }
    }
}

pub fn digest(manifest: &Manifest) -> anyhow::Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(manifest)?)
        .to_hex()
        .to_string())
}
