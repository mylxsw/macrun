//! Local performance observations. Task results are authoritative; SQLite is a repairable index.
//! Never store commands, environments, output, credentials or image contents here.
use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex, OnceLock, Weak},
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub name: String,
    pub wall_ms: f64,
    pub count: u64,
    pub max_ms: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub offset_ms: u64,
    pub payload_bytes: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub name: String,
    pub bytes: Option<u64>,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    pub schema_version: u32,
    pub origin: String,
    pub wall_ms: f64,
    pub complete: bool,
    pub phases: Vec<Phase>,
    pub bytes: BTreeMap<String, u64>,
    pub files: BTreeMap<String, u64>,
    pub samples: Vec<Sample>,
    pub outputs: Vec<Output>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub remote_job_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub transfer_id: Option<String>,
    pub direction: Option<String>,
    pub transport: Option<String>,
    pub optimized_sync: Option<bool>,
    pub strict: Option<bool>,
    #[serde(default)]
    pub first_payload_offset_ms: Option<f64>,
    #[serde(default)]
    pub last_payload_offset_ms: Option<f64>,
    #[serde(default)]
    pub rtt_ms: Option<f64>,
    #[serde(default)]
    pub dimensions: BTreeMap<String, String>,
}
impl Default for Metrics {
    fn default() -> Self {
        Self {
            schema_version: 1,
            origin: "worker".into(),
            wall_ms: 0.0,
            complete: false,
            phases: vec![],
            bytes: BTreeMap::new(),
            files: BTreeMap::new(),
            samples: vec![],
            outputs: vec![],
            project_id: None,
            project_name: None,
            remote_job_id: None,
            parent_task_id: None,
            transfer_id: None,
            direction: None,
            transport: None,
            optimized_sync: None,
            strict: None,
            first_payload_offset_ms: None,
            last_payload_offset_ms: None,
            rtt_ms: None,
            dimensions: BTreeMap::new(),
        }
    }
}
#[derive(Clone)]
pub struct Capture {
    start: Instant,
    record: Arc<Mutex<Metrics>>,
}
tokio::task_local! { static CURRENT: Capture; }
pub fn current() -> Option<Capture> {
    CURRENT.try_with(Clone::clone).ok()
}
impl Capture {
    pub fn new(origin: &str) -> Self {
        Self {
            start: Instant::now(),
            record: Arc::new(Mutex::new(Metrics {
                origin: origin.into(),
                ..Default::default()
            })),
        }
    }
    pub async fn scope<F: Future>(&self, future: F) -> F::Output {
        CURRENT.scope(self.clone(), future).await
    }
    pub fn sync_scope<F: FnOnce() -> T, T>(&self, work: F) -> T {
        CURRENT.sync_scope(self.clone(), work)
    }
    pub fn update(&self, f: impl FnOnce(&mut Metrics)) {
        if let Ok(mut m) = self.record.lock() {
            f(&mut m);
        }
    }
    pub fn snapshot(&self, complete: bool) -> Metrics {
        let mut m = self.record.lock().unwrap().clone();
        m.wall_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        m.complete = complete;
        if let (Some(last), Some(total)) =
            (m.last_payload_offset_ms, m.bytes.get("payload").copied())
        {
            let last = Sample {
                offset_ms: last as u64,
                payload_bytes: total,
            };
            if m.samples.last().is_none_or(|s| s.payload_bytes != total) {
                if m.samples.len() == 600 {
                    m.samples.pop();
                }
                m.samples.push(last);
            }
        }
        m
    }
    pub fn project(&self, root: &Path, key: &[u8; 32]) {
        self.update(|m| {
            m.project_id = Some(
                blake3::keyed_hash(key, root.as_os_str().as_encoded_bytes())
                    .to_hex()
                    .to_string(),
            );
            m.project_name = root
                .file_name()
                .map(|v| v.to_string_lossy().chars().take(80).collect());
        });
    }
    pub fn add(&self, group: &str, name: &str, n: u64) {
        self.update(|m| {
            let map = if group == "files" {
                &mut m.files
            } else {
                &mut m.bytes
            };
            let v = map.entry(name.into()).or_default();
            *v = v.saturating_add(n);
            if name == "payload" {
                let at = self.start.elapsed().as_secs_f64() * 1000.0;
                m.first_payload_offset_ms.get_or_insert(at);
                m.last_payload_offset_ms = Some(at);
                let offset_ms = self.start.elapsed().as_millis() as u64;
                if m.samples
                    .last()
                    .is_none_or(|s| offset_ms.saturating_sub(s.offset_ms) >= 1000)
                    && m.samples.len() < 600
                {
                    m.samples.push(Sample {
                        offset_ms,
                        payload_bytes: *v,
                    });
                }
            }
        });
    }
}
pub struct PhaseGuard {
    capture: Option<Capture>,
    start: Instant,
    name: &'static str,
}
pub fn phase(name: &'static str) -> PhaseGuard {
    PhaseGuard {
        capture: current(),
        start: Instant::now(),
        name,
    }
}
impl Drop for PhaseGuard {
    fn drop(&mut self) {
        if let Some(c) = &self.capture {
            let ms = self.start.elapsed().as_secs_f64() * 1000.0;
            c.update(|m| {
                if let Some(p) = m.phases.iter_mut().find(|p| p.name == self.name) {
                    p.wall_ms += ms;
                    p.count = p.count.saturating_add(1);
                    p.max_ms = p.max_ms.max(ms);
                } else if m.phases.len() < 64 {
                    m.phases.push(Phase {
                        name: self.name.into(),
                        wall_ms: ms,
                        count: 1,
                        max_ms: ms,
                    });
                }
            });
        }
    }
}
pub fn add(group: &str, name: &str, n: u64) {
    if let Some(c) = current() {
        c.add(group, name, n);
    }
}
pub fn set(group: &str, name: &str, n: u64) {
    if let Some(c) = current() {
        c.update(|m| {
            (if group == "files" {
                &mut m.files
            } else {
                &mut m.bytes
            })
            .insert(name.into(), n);
        });
    }
}
pub fn validate_report(m: &Metrics) -> Result<()> {
    ensure!(
        m.schema_version == 1 && m.origin == "server" && m.complete,
        "invalid performance report origin/version"
    );
    ensure!(
        m.wall_ms.is_finite() && (0.0..=86_400_000.0).contains(&m.wall_ms),
        "invalid performance duration"
    );
    ensure!(
        m.phases.len() <= 64
            && m.samples.len() <= 600
            && m.outputs.is_empty()
            && m.project_id.is_none()
            && m.project_name.is_none()
            && m.parent_task_id.is_none()
            && m.remote_job_id.is_none()
            && m.transfer_id.is_none()
            && m.direction.is_none()
            && m.transport.is_none(),
        "invalid remote performance fields"
    );
    ensure!(
        m.dimensions.is_empty() && m.rtt_ms.is_none(),
        "remote dimensions not accepted"
    );
    for p in &m.phases {
        ensure!(
            [
                "sync_total",
                "scan",
                "hash",
                "send_payload",
                "pack_encode",
                "confirm_source"
            ]
            .contains(&p.name.as_str())
                && p.name.len() <= 64
                && p.name.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
                && p.wall_ms.is_finite()
                && p.wall_ms >= 0.0
                && p.max_ms.is_finite()
                && p.max_ms >= 0.0,
            "invalid performance phase"
        );
    }
    ensure!(
        m.samples
            .windows(2)
            .all(|s| s[0].offset_ms <= s[1].offset_ms && s[0].payload_bytes <= s[1].payload_bytes)
            && m.samples.iter().all(|s| s.offset_ms <= 86_400_000)
            && [m.first_payload_offset_ms, m.last_payload_offset_ms]
                .into_iter()
                .flatten()
                .all(|ms| ms.is_finite() && (0.0..=86_400_000.0).contains(&ms)),
        "invalid performance samples"
    );
    for map in [&m.bytes, &m.files] {
        ensure!(
            map.len() <= 32
                && map.iter().all(|(k, v)| k.len() <= 64
                    && k.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
                    && *v <= i64::MAX as u64),
            "invalid performance counters"
        );
    }
    ensure!(
        m.bytes.keys().all(|k| [
            "hashed",
            "scanned_logical",
            "payload",
            "pack_raw",
            "pack_compressed",
            "control_rx",
            "control_tx"
        ]
        .contains(&k.as_str()))
            && m.files.keys().all(|k| [
                "scanned",
                "scanned_dirs",
                "skipped_links",
                "hash_cache_hits",
                "hash_cache_misses",
                "packs"
            ]
            .contains(&k.as_str())),
        "unknown remote performance counter"
    );
    ensure!(
        serde_json::to_vec(m)?.len() <= 65536,
        "performance report too large"
    );
    Ok(())
}
/// Count successful I/O, including partial bodies before errors. Never count planned lengths.
pub struct Metered<T> {
    inner: T,
    capture: Option<Capture>,
    counter: &'static str,
}
impl<T> Metered<T> {
    pub fn new(inner: T, counter: &'static str) -> Self {
        Self {
            inner,
            capture: current(),
            counter,
        }
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for Metered<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        let n = buf.filled().len() - before;
        if n > 0
            && let Some(c) = &this.capture
        {
            c.add("bytes", this.counter, n as u64);
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for Metered<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = r
            && let Some(c) = &this.capture
        {
            c.add("bytes", this.counter, n as u64);
        }
        r
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// A sanitized observation, never the original task arguments or result body.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub task_id: String,
    pub kind: String,
    pub status: String,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub metrics: Option<Metrics>,
    pub server_metrics: Option<Metrics>,
    pub version: String,
}
impl Record {
    fn from_task(task: &Value) -> Result<Self> {
        let task_id = task["task_id"].as_str().unwrap_or("").to_owned();
        uuid::Uuid::parse_str(&task_id)?;
        Ok(Self {
            task_id,
            kind: task["kind"].as_str().unwrap_or("unknown").into(),
            status: task["status"].as_str().unwrap_or("unknown").into(),
            started_at: task["started_at"].as_u64().unwrap_or(0),
            ended_at: task["ended_at"].as_u64(),
            metrics: task
                .get("metrics")
                .cloned()
                .map(serde_json::from_value)
                .transpose()?,
            server_metrics: task
                .get("server_metrics")
                .cloned()
                .map(serde_json::from_value)
                .transpose()?,
            version: task["metrics_version"].as_str().unwrap_or("legacy").into(),
        })
    }
}
struct Index {
    connection: Mutex<Connection>,
    states: Mutex<BTreeMap<String, String>>,
}
static INDEXES: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Index>>>> = OnceLock::new();
#[derive(Clone)]
pub struct Store {
    data: PathBuf,
    index: Arc<Index>,
}
impl Store {
    pub fn record(&self, value: &Value) -> Result<()> {
        let result = Record::from_task(value).and_then(|record| self.write(&record));
        if result.is_err() {
            use std::os::unix::fs::OpenOptionsExt;
            // Cross-process hint: the desktop must not silently present a stale index as complete.
            let _ = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(self.data.join("metrics.index-dirty"));
        }
        result
    }
    pub fn open(data: PathBuf) -> Result<Self> {
        let mut indexes = INDEXES.get_or_init(Default::default).lock().unwrap();
        indexes.retain(|_, index| index.strong_count() > 0);
        if let Some(index) = indexes.get(&data).and_then(Weak::upgrade) {
            return Ok(Self { data, index });
        }
        std::fs::create_dir_all(&data)?;
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700))?;
        let path = data.join("metrics.sqlite");
        ensure!(
            !std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()),
            "metrics index cannot be a symlink"
        );
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
            ensure!(
                !std::fs::symlink_metadata(sidecar).is_ok_and(|m| m.file_type().is_symlink()),
                "performance index sidecar cannot be a symlink"
            );
        }
        if !path.exists() {
            crate::wire::private_write(&path, &[])?;
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_millis(100))?;
        let configure = |db: &Connection| -> Result<()> {
            let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
            ensure!(
                version <= 1,
                "performance index was created by a newer version"
            );
            db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
          CREATE TABLE IF NOT EXISTS operations(task_id TEXT PRIMARY KEY, started_at INTEGER NOT NULL,
          ended_at INTEGER, kind TEXT NOT NULL, status TEXT NOT NULL, project_id TEXT, wall_ms REAL, summary TEXT NOT NULL);
          CREATE INDEX IF NOT EXISTS operation_time ON operations(started_at, task_id);
          CREATE INDEX IF NOT EXISTS operation_kind ON operations(kind, started_at);
          PRAGMA user_version=1;")?;
            Ok(())
        };
        let connection = match configure(&connection) {
            Ok(()) => connection,
            Err(e) if e.downcast_ref::<rusqlite::Error>().is_some_and(|e| matches!(e,
                rusqlite::Error::SqliteFailure(code, _) if matches!(code.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase))) => {
                drop(connection);
                // Results remain authoritative. Keep one private corrupt copy for diagnosis.
                std::fs::rename(&path, data.join("metrics.corrupt"))?;
                for suffix in ["-wal", "-shm"] { let p=PathBuf::from(format!("{}{suffix}",path.display())); if p.exists() { std::fs::remove_file(p)?; } }
                crate::wire::private_write(&path, &[])?;
                let db=Connection::open(&path)?;
                db.busy_timeout(Duration::from_millis(100))?;
                configure(&db)?;
                db
            }
            Err(e) => return Err(e),
        };
        for suffix in ["-wal", "-shm"] {
            let p = PathBuf::from(format!("{}{suffix}", path.display()));
            if p.exists() {
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600))?;
            }
        }
        let index = Arc::new(Index {
            connection: Mutex::new(connection),
            states: Mutex::new(BTreeMap::new()),
        });
        indexes.insert(data.clone(), Arc::downgrade(&index));
        drop(indexes);
        let store = Self { data, index };
        store.reconcile()?;
        Ok(store)
    }
    fn write(&self, record: &Record) -> Result<()> {
        let mut states = self.index.states.lock().unwrap();
        if record.ended_at.is_none() && states.get(&record.task_id) == Some(&record.status) {
            return Ok(());
        }
        let mut summary = record.clone();
        for m in [&mut summary.metrics, &mut summary.server_metrics]
            .into_iter()
            .flatten()
        {
            m.samples.clear();
            m.outputs.clear();
        }
        let started = i64::try_from(record.started_at)?;
        let ended = record.ended_at.map(i64::try_from).transpose()?;
        let db = self.index.connection.lock().unwrap();
        db.execute("INSERT INTO operations VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
            ON CONFLICT(task_id) DO UPDATE SET started_at=excluded.started_at,ended_at=excluded.ended_at,
            kind=excluded.kind,status=excluded.status,project_id=excluded.project_id,wall_ms=excluded.wall_ms,summary=excluded.summary",
            params![record.task_id, started, ended, record.kind, record.status,
                record.metrics.as_ref().and_then(|m|m.project_id.as_ref()), record.metrics.as_ref().map(|m|m.wall_ms), serde_json::to_string(&summary)?])?;
        states.insert(record.task_id.clone(), record.status.clone());
        Ok(())
    }
    pub fn reconcile(&self) -> Result<()> {
        let mut present = BTreeSet::new();
        for sub in ["tasks", "metrics-attempts"] {
            let root = self.data.join(sub);
            if !root.exists() {
                continue;
            }
            for e in std::fs::read_dir(root)? {
                let e = e?;
                if e.file_type()?.is_symlink()
                    || (sub == "metrics-attempts"
                        && e.path().extension().is_none_or(|x| x != "json"))
                {
                    continue;
                }
                let path = if sub == "tasks" {
                    e.path().join("result.json")
                } else {
                    e.path()
                };
                if let Ok(b) = std::fs::read(path)
                    && let Ok(v) = serde_json::from_slice::<Value>(&b)
                    && let Ok(r) = Record::from_task(&v)
                {
                    present.insert(r.task_id.clone());
                    self.write(&r)?;
                }
            }
        }
        let db = self.index.connection.lock().unwrap();
        let ids = db
            .prepare("SELECT task_id FROM operations")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in ids {
            if !present.contains(&id) {
                db.execute("DELETE FROM operations WHERE task_id=?1", [id])?;
            }
        }
        let _ = std::fs::remove_file(self.data.join("metrics.index-dirty"));
        Ok(())
    }
    pub fn remove(&self, id: &str) -> Result<()> {
        self.index.states.lock().unwrap().remove(id);
        self.index
            .connection
            .lock()
            .unwrap()
            .execute("DELETE FROM operations WHERE task_id=?1", [id])?;
        Ok(())
    }
}
/// Invoked on the same blocking thread as the task replacement. Index failures never change task outcomes.
pub(crate) fn record_written(path: &Path, value: &Value) {
    let Some(tasks) = path
        .parent()
        .and_then(Path::parent)
        .filter(|p| p.file_name().is_some_and(|n| n == "tasks"))
    else {
        return;
    };
    let Some(data) = tasks.parent() else { return };
    let index = INDEXES
        .get()
        .and_then(|i| i.lock().ok()?.get(data)?.upgrade());
    if let Some(index) = index {
        let store = Store {
            data: data.into(),
            index,
        };
        if store.record(value).is_err() {
            crate::logging::event("worker", "metrics_index_failed", json!({}));
        }
    }
}

#[derive(Clone)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub data: PathBuf,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Query {
    from_ms: Option<u64>,
    to_ms: Option<u64>,
    connection_id: Option<String>,
    project_id: Option<String>,
    kind: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
    cursor: Option<Cursor>,
    task_id: Option<String>,
}
/// Stream the full filter to disk; the WebView receives only a row count.
pub async fn export(
    profiles: Vec<Profile>,
    args: Value,
    format: String,
    path: PathBuf,
) -> Result<usize> {
    crate::wire::blocking(move || export_sync(&profiles, args, &format, &path)).await?
}
fn export_sync(profiles: &[Profile], args: Value, format: &str, path: &Path) -> Result<usize> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    ensure!(
        ["json", "csv"].contains(&format),
        "unsupported export format"
    );
    let mut q: Query = serde_json::from_value(args)?;
    q.cursor = None;
    q.limit = None;
    q.task_id = None;
    let to = q.to_ms.unwrap_or(crate::model::now().saturating_add(1));
    let from = q.from_ms.unwrap_or(to.saturating_sub(7 * 86_400_000));
    ensure!(
        from < to && to - from <= 90 * 86_400_000,
        "invalid export range"
    );
    let from = i64::try_from(from)?;
    let to = i64::try_from(to)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("export directory required"))?;
    let parent = std::fs::canonicalize(parent)?;
    let destination = parent.join(
        path.file_name()
            .ok_or_else(|| anyhow::anyhow!("export filename required"))?,
    );
    for profile in profiles {
        if let Ok(data) = std::fs::canonicalize(&profile.data) {
            ensure!(
                !parent.starts_with(data),
                "choose an export location outside runtime data"
            );
        }
    }
    struct Pending(PathBuf);
    impl Drop for Pending {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let pending = Pending(parent.join(format!(".macrun-export-{}", uuid::Uuid::new_v4())));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&pending.0)?;
    let mut writer = std::io::BufWriter::new(file);
    if format == "json" {
        let metadata = json!({"schema_version":1,"as_of":crate::model::now(),"filters":q,"scope":"accepted operations; worker and server phases are independent; payload includes failed attempts; counters above 2^53 are decimal strings"});
        write!(writer, "{{\"metadata\":{},\"operations\":[", metadata)?;
    } else {
        writeln!(
            writer,
            "connection_id,connection_name,task_id,kind,status,started_at,ended_at,project,wall_ms,payload_bytes,declared_output_bytes,artifact_bytes,metrics_version"
        )?;
    }
    let mut count = 0;
    let mut selected = false;
    for profile in profiles {
        if q.connection_id
            .as_ref()
            .is_some_and(|id| !id.is_empty() && id != &profile.id)
        {
            continue;
        }
        selected = true;
        if profile.data.join("metrics.index-dirty").exists() {
            anyhow::bail!(
                "performance index is incomplete; restart the worker to rebuild before exporting"
            );
        }
        if !profile.data.join("metrics.sqlite").exists() {
            let _ = Store::open(profile.data.clone());
        }
        let db = Connection::open_with_flags(
            profile.data.join("metrics.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::from_millis(100))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        db.progress_handler(1000, Some(move || Instant::now() >= deadline))?;
        let mut stmt=db.prepare("SELECT summary FROM operations WHERE started_at>=?1 AND started_at<?2 AND (?3 IS NULL OR kind=?3) AND (?4 IS NULL OR status=?4) AND (?5 IS NULL OR project_id=?5) ORDER BY started_at DESC,task_id DESC")?;
        let mut records = stmt.query(params![from, to, q.kind, q.status, q.project_id])?;
        while let Some(row) = records.next()? {
            let r: Record = serde_json::from_str(&row.get::<_, String>(0)?)?;
            if format == "json" {
                if count > 0 {
                    write!(writer, ",")?;
                }
                let mut value = decorated(&r, profile);
                safe_numbers(&mut value);
                serde_json::to_writer(&mut writer, &value)?;
            } else {
                let m = r.metrics.as_ref();
                let fields = vec![
                    profile.id.clone(),
                    profile.name.clone(),
                    r.task_id,
                    r.kind,
                    r.status,
                    r.started_at.to_string(),
                    r.ended_at.map(|v| v.to_string()).unwrap_or_default(),
                    m.and_then(|m| m.project_name.clone()).unwrap_or_default(),
                    m.map(|m| m.wall_ms.to_string()).unwrap_or_default(),
                    m.and_then(|m| m.bytes.get("payload"))
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                    m.and_then(|m| m.bytes.get("declared_outputs"))
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                    m.and_then(|m| m.bytes.get("artifacts"))
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                    r.version,
                ];
                writeln!(
                    writer,
                    "{}",
                    fields
                        .iter()
                        .map(|s| csv_field(s))
                        .collect::<Vec<_>>()
                        .join(",")
                )?;
            }
            count += 1;
        }
    }
    ensure!(selected, "unknown connection");
    if format == "json" {
        write!(writer, "]}}")?;
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    drop(writer);
    std::fs::rename(&pending.0, destination)?;
    Ok(count)
}
fn csv_field(value: &str) -> String {
    let prefix = if value
        .trim_start()
        .starts_with(['=', '+', '-', '@', '\t', '\r'])
    {
        "'"
    } else {
        ""
    };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Cursor {
    pub started_at: u64,
    pub connection_id: String,
    pub task_id: String,
}
pub fn percentile(values: &mut [f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[((p * values.len() as f64).ceil() as usize).clamp(1, values.len()) - 1])
}
fn counted(r: &Record) -> bool {
    r.kind != "artifact.download"
        && r.metrics
            .as_ref()
            .is_none_or(|m| m.parent_task_id.is_none())
}
fn decorated(record: &Record, profile: &Profile) -> Value {
    let mut v = serde_json::to_value(record).unwrap_or(Value::Null);
    v["connection_id"] = json!(profile.id);
    v["connection_name"] = json!(profile.name);
    v
}
/// Preserve exact counters across the JavaScript boundary.
fn safe_numbers(value: &mut Value) {
    match value {
        Value::Number(n) if n.as_u64().is_some_and(|n| n > 9_007_199_254_740_991) => {
            *value = Value::String(n.to_string());
        }
        Value::Array(values) => values.iter_mut().for_each(safe_numbers),
        Value::Object(values) => values.values_mut().for_each(safe_numbers),
        _ => {}
    }
}
pub fn for_web(mut value: Value) -> Value {
    safe_numbers(&mut value);
    value
}
pub fn prune_samples(value: &mut Value, cutoff: u64) -> bool {
    if value["ended_at"].as_u64().is_none_or(|at| at >= cutoff) {
        return false;
    }
    let mut changed = false;
    for key in ["metrics", "server_metrics"] {
        if value[key]["samples"]
            .as_array()
            .is_some_and(|s| !s.is_empty())
        {
            value[key]["samples"] = json!([]);
            changed = true;
        }
    }
    changed
}

/// Opens only known, same-user profile directories. Works when the worker and server are stopped.
pub async fn query(profiles: Vec<Profile>, action: &str, args: Value) -> Result<Value> {
    let action = action.to_owned();
    let mut result = crate::wire::blocking(move || query_sync(&profiles, &action, args)).await??;
    safe_numbers(&mut result);
    Ok(result)
}
fn query_sync(profiles: &[Profile], action: &str, args: Value) -> Result<Value> {
    ensure!(
        [
            "metrics_dashboard",
            "metrics_summary",
            "metrics_operations",
            "metrics_series",
            "metrics_detail"
        ]
        .contains(&action),
        "unknown metrics action"
    );
    let q: Query = serde_json::from_value(args)?;
    if action == "metrics_detail" {
        uuid::Uuid::parse_str(
            q.task_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("task_id required"))?,
        )?;
    }
    let now = crate::model::now();
    let to = q.to_ms.unwrap_or(now.saturating_add(1));
    let from = q.from_ms.unwrap_or(to.saturating_sub(7 * 86_400_000));
    ensure!(
        from < to && to - from <= 90 * 86_400_000,
        "performance range must be between 1 ms and 90 days"
    );
    let from = i64::try_from(from)?;
    let to = i64::try_from(to)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 100);
    let mut rows: Vec<Value> = vec![];
    let mut durations = vec![];
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut bytes: BTreeMap<String, u64> = BTreeMap::new();
    let mut phases: BTreeMap<String, f64> = BTreeMap::new();
    let mut buckets: BTreeMap<u64, (u64, Vec<f64>, u64)> = BTreeMap::new();
    let bucket_ms = ((to - from) as u64 / 120).max(60_000);
    let mut measured = 0u64;
    let mut total = 0u64;
    let mut errors = vec![];
    let mut projects = BTreeMap::new();
    let mut selected = false;
    for profile in profiles {
        if q.connection_id
            .as_ref()
            .is_some_and(|id| !id.is_empty() && id != &profile.id)
        {
            continue;
        }
        selected = true;
        if !profile.data.join("metrics.sqlite").exists() {
            if profile.data.join("tasks").exists() {
                let _ = Store::open(profile.data.clone());
            }
            if !profile.data.join("metrics.sqlite").exists() {
                continue;
            }
        }
        if profile.data.join("metrics.index-dirty").exists() {
            errors.push(json!({"connection_id":profile.id,"message":"部分性能数据尚未写入索引，汇总可能不完整；重启执行器可从任务结果重建"}));
        }
        let mut work = || -> Result<()> {
            let db = Connection::open_with_flags(
                profile.data.join("metrics.sqlite"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            db.busy_timeout(Duration::from_millis(100))?;
            let deadline = Instant::now() + Duration::from_secs(5);
            db.progress_handler(1000, Some(move || Instant::now() >= deadline))?;
            let mut statement = db.prepare("SELECT summary FROM operations WHERE (?1 IS NOT NULL OR (started_at>=?2 AND started_at<?3) OR (ended_at>=?2 AND ended_at<?3)) AND (?1 IS NULL OR task_id=?1) AND (?4 IS NULL OR kind=?4) AND (?5 IS NULL OR status=?5) AND (?6 IS NULL OR project_id=?6) ORDER BY started_at DESC,task_id DESC")?;
            let mut records =
                statement.query(params![q.task_id, from, to, q.kind, q.status, q.project_id])?;
            while let Some(row) = records.next()? {
                let r: Record = serde_json::from_str(&row.get::<_, String>(0)?)?;
                if action == "metrics_detail" {
                    let path = if r.kind == "artifact.download" {
                        profile
                            .data
                            .join("metrics-attempts")
                            .join(format!("{}.json", r.task_id))
                    } else {
                        profile
                            .data
                            .join("tasks")
                            .join(&r.task_id)
                            .join("result.json")
                    };
                    let full = Record::from_task(&serde_json::from_slice::<Value>(
                        &std::fs::read(path)?,
                    )?)?;
                    rows.push(decorated(&full, profile));
                    continue;
                }
                let accepted = r.started_at >= from as u64 && r.started_at < to as u64;
                let completed = r
                    .ended_at
                    .is_some_and(|at| at >= from as u64 && at < to as u64);
                if let Some(m) = &r.metrics {
                    if let (Some(id), Some(name)) = (&m.project_id, &m.project_name) {
                        projects.insert(id.clone(), format!("{name} · {}", profile.name));
                    }
                    if accepted {
                        for (k, n) in &m.bytes {
                            if k == "artifacts" && r.kind == "desktop.sequence" {
                                continue;
                            }
                            let v = bytes.entry(k.clone()).or_default();
                            *v = v.saturating_add(*n);
                        }
                    }
                }
                if counted(&r) {
                    if accepted {
                        total += 1;
                        *counts.entry(r.status.clone()).or_default() += 1;
                        buckets
                            .entry(r.started_at / bucket_ms * bucket_ms)
                            .or_default()
                            .0 += 1;
                    }
                    if let Some(m) = &r.metrics {
                        if accepted {
                            measured += 1;
                            for p in &m.phases {
                                *phases.entry(p.name.clone()).or_default() += p.wall_ms;
                            }
                        }
                        if completed && r.status == "succeeded" && m.complete {
                            durations.push(m.wall_ms);
                            buckets
                                .entry(r.ended_at.unwrap() / bucket_ms * bucket_ms)
                                .or_default()
                                .1
                                .push(m.wall_ms);
                        }
                    }
                }
                if accepted && let Some(m) = &r.metrics {
                    let bucket = buckets
                        .entry(r.started_at / bucket_ms * bucket_ms)
                        .or_default();
                    bucket.2 = bucket
                        .2
                        .saturating_add(*m.bytes.get("payload").unwrap_or(&0));
                }
                let position = (r.started_at, profile.id.as_str(), r.task_id.as_str());
                if accepted
                    && q.cursor.as_ref().is_none_or(|c| {
                        position < (c.started_at, c.connection_id.as_str(), c.task_id.as_str())
                    })
                    && (rows.len() < limit + 1
                        || rows.last().is_some_and(|r| {
                            position
                                > (
                                    r["started_at"].as_u64().unwrap_or(0),
                                    r["connection_id"].as_str().unwrap_or(""),
                                    r["task_id"].as_str().unwrap_or(""),
                                )
                        }))
                {
                    rows.push(decorated(&r, profile));
                    rows.sort_by(|a, b| {
                        (
                            b["started_at"].as_u64(),
                            b["connection_id"].as_str(),
                            b["task_id"].as_str(),
                        )
                            .cmp(&(
                                a["started_at"].as_u64(),
                                a["connection_id"].as_str(),
                                a["task_id"].as_str(),
                            ))
                    });
                    rows.truncate(limit + 1);
                }
            }
            Ok(())
        };
        if work().is_err() {
            errors.push(json!({"connection_id":profile.id,"message":"性能索引读取失败，可重启执行器重建索引"}));
        }
    }
    ensure!(selected, "unknown connection");
    if action == "metrics_detail" {
        ensure!(
            rows.len() == 1,
            "task not found or ambiguous; select a connection"
        );
        return Ok(rows.remove(0));
    }
    let p50 = percentile(&mut durations, 0.5);
    let p95 = percentile(&mut durations, 0.95);
    let p99 = percentile(&mut durations, 0.99);
    let more = rows.len() > limit;
    rows.truncate(limit);
    let cursor = if more {
        rows.last().map(|r|json!({"started_at":r["started_at"],"connection_id":r["connection_id"],"task_id":r["task_id"]}))
    } else {
        None
    };
    for time in ((from as u64 / bucket_ms * bucket_ms)..to as u64).step_by(bucket_ms as usize) {
        buckets.entry(time).or_default();
    }
    let series:Vec<Value>=buckets.into_iter().map(|(time,(count,mut samples,payload))|json!({"time":time,"count":count,"p50_ms":percentile(&mut samples,0.5),"p95_ms":percentile(&mut samples,0.95),"payload_bytes":payload})).collect();
    let denominator = counts.get("succeeded").unwrap_or(&0)
        + counts.get("failed").unwrap_or(&0)
        + counts.get("timed_out").unwrap_or(&0)
        + counts.get("unknown").unwrap_or(&0);
    Ok(
        json!({"as_of":now,"total":total,"measured":measured,"counts":counts,"success_denominator":denominator,
        "latency":{"n":durations.len(),"p50_ms":p50,"p95_ms":p95,"p99_ms":p99},"bytes":bytes,"phases_ms":phases,
        "operations":rows,"next_cursor":cursor,"series":series,"bucket_ms":bucket_ms,"projects":projects,"errors":errors}),
    )
}

/// Measure explicit outputs only; no contents are read and links never escape the declared workspace.
pub fn measure_outputs(root: &Path, declared: &[String]) -> Vec<Output> {
    measure_output_totals(root, declared).0
}
pub fn measure_output_totals(root: &Path, declared: &[String]) -> (Vec<Output>, Option<u64>, u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut out = vec![];
    let mut entries = 0usize;
    let mut all_seen = BTreeSet::new();
    let mut unique = 0u64;
    for name in declared.iter().take(32) {
        let path = Path::new(name);
        if path.is_absolute()
            || path.components().any(|c| {
                !matches!(
                    c,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            out.push(Output {
                name: name.chars().take(128).collect(),
                bytes: None,
                status: "outside_workspace".into(),
            });
            continue;
        }
        let mut size = 0u64;
        let mut seen = BTreeSet::new();
        let mut status = "complete";
        let pattern = if name.contains(['*', '?', '[', '{']) {
            match globset::GlobBuilder::new(name)
                .literal_separator(true)
                .build()
            {
                Ok(glob) => Some(glob.compile_matcher()),
                Err(_) => {
                    out.push(Output {
                        name: name.clone(),
                        bytes: None,
                        status: "invalid_pattern".into(),
                    });
                    continue;
                }
            }
        } else {
            None
        };
        let path = root.join(path);
        // Reject intermediate symlinks too, rather than trusting a joined path.
        if std::fs::canonicalize(&path)
            .ok()
            .zip(std::fs::canonicalize(root).ok())
            .is_some_and(|(p, r)| !p.starts_with(r))
        {
            out.push(Output {
                name: name.clone(),
                bytes: None,
                status: "outside_workspace".into(),
            });
            continue;
        }
        if pattern.is_none() && !path.exists() {
            status = "missing";
        } else {
            let scan = if pattern.is_some() { root } else { &path };
            let mut matched = pattern.is_none();
            for e in walkdir::WalkDir::new(scan).follow_links(false) {
                entries += 1;
                if entries > 100_000 || Instant::now() > deadline {
                    status = "incomplete";
                    break;
                }
                let e = match e {
                    Ok(e) => e,
                    Err(_) => {
                        status = "unreadable";
                        break;
                    }
                };
                if let Some(pattern) = &pattern {
                    let Ok(relative) = e.path().strip_prefix(root) else {
                        continue;
                    };
                    if !relative.ancestors().any(|p| pattern.is_match(p)) {
                        continue;
                    }
                    matched = true;
                }
                match e.metadata().map(|m| (e, m)) {
                    Ok((e, m)) if m.is_file() && !e.file_type().is_symlink() => {
                        if seen.insert((m.dev(), m.ino())) {
                            size = size.saturating_add(m.len());
                        }
                        if all_seen.insert((m.dev(), m.ino())) {
                            unique = unique.saturating_add(m.len());
                        }
                    }
                    Ok(_) => {}
                    Err(_) => {
                        status = "unreadable";
                        break;
                    }
                }
            }
            if !matched && status == "complete" {
                status = "missing";
            }
        }
        out.push(Output {
            name: name.clone(),
            bytes: (status == "complete").then_some(size),
            status: status.into(),
        });
    }
    let complete = !out.is_empty() && out.iter().all(|o| o.status == "complete");
    (out, complete.then_some(unique), all_seen.len() as u64)
}
