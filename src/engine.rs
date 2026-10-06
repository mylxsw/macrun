//! Worker-owned tasks survive transport disconnections. Only worker restart loses execution ownership.
use crate::{
    backend::Client,
    config::{WorkerConfig, expand},
    files::string,
    model::{Event, id, now},
    process, wire,
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, mpsc, watch},
};
use tokio_util::sync::CancellationToken;
type Slot = Arc<Mutex<Option<Client>>>;
pub struct Engine {
    data: PathBuf,
    history: crate::history::TaskHistory,
    metrics_store: Option<crate::metrics::Store>,
    config: WorkerConfig,
    submissions: Mutex<()>,
    tasks: Mutex<BTreeMap<String, CancellationToken>>,
    backends: BTreeMap<String, Slot>,
    policy: Mutex<Policy>,
    safety: Mutex<crate::safety::Safety>,
    approvals: Mutex<BTreeMap<String, Pending>>,
    rules: Mutex<Vec<crate::safety::AllowRule>>,
    backend_stop: Mutex<CancellationToken>,
    tool_counts: Mutex<BTreeMap<String, DiscoveredTools>>,
    changes: watch::Sender<u64>,
    fingerprint_key: [u8; 32],
}
/// Holds admission closed while all profiles are checked before an idle-only shutdown.
pub(crate) struct IdleShutdownGuard<'a> {
    engine: &'a Engine,
    _admission: tokio::sync::MutexGuard<'a, ()>,
}
impl IdleShutdownGuard<'_> {
    pub(crate) async fn pause(&self) {
        self.engine.policy.lock().await.paused = true;
        self.engine.changed();
    }
}
#[derive(Default)]
struct DiscoveredTools {
    session: String,
    names: BTreeSet<String>,
    tiers: BTreeMap<String, &'static str>,
}
/// A request waiting for a local decision, kept so an approval can grant a rule.
struct Pending {
    sender: tokio::sync::oneshot::Sender<bool>,
    kind: String,
    args: Value,
    tier: Option<&'static str>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Policy {
    pub paused: bool,
    pub desktop_enabled: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            paused: false,
            desktop_enabled: true,
        }
    }
}
fn denial_code(text: &str) -> Option<&'static str> {
    if text.starts_with("denied: approval expired") {
        Some("approval_expired")
    } else if text.starts_with("denied: approval rejected") {
        Some("approval_rejected")
    } else {
        None
    }
}
fn redact_arguments(a: &Value) -> Value {
    let mut a = a.clone();
    if let Some(env) = a.get_mut("env").and_then(Value::as_object_mut) {
        for value in env.values_mut() {
            *value = json!("••••");
        }
    }
    a
}
impl Engine {
    pub fn open(data: PathBuf, config: WorkerConfig) -> Result<Arc<Self>> {
        for (backend, group) in &config.resource_groups {
            anyhow::ensure!(
                config.mcp.contains_key(backend) && !group.is_empty() && group.len() <= 128,
                "invalid backend resource group"
            );
        }
        std::fs::create_dir_all(data.join("tasks"))?;
        let key_path = data.join("request-key");
        let key = if key_path.exists() {
            std::fs::read(&key_path)?
        } else {
            let mut bytes = uuid::Uuid::new_v4().as_bytes().to_vec();
            bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
            wire::private_write(&key_path, &bytes)?;
            bytes
        };
        let fingerprint_key: [u8; 32] = key
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid request key"))?;
        for entry in std::fs::read_dir(data.join("tasks"))? {
            let dir = entry?.path();
            let p = dir.join("result.json");
            if !p.exists() {
                continue;
            }
            let mut v: Value = serde_json::from_slice(&std::fs::read(&p)?)?;
            if v.get("arguments").is_some() && v.get("request_fingerprint").is_none() {
                v["request_fingerprint"] = json!(
                    blake3::keyed_hash(&fingerprint_key, &serde_json::to_vec(&v["arguments"])?)
                        .to_hex()
                        .to_string()
                );
                v["arguments"] = redact_arguments(&v["arguments"]);
                wire::atomic_json(&p, &v)?;
            }
            if v["status"] == "awaiting_approval" {
                v["status"] = json!("denied");
                v["ended_at"] = json!(now());
                v["error"] = json!({"code":"approval_interrupted","message":"Approval interrupted by restart; command was not dispatched."});
                wire::atomic_json(&p, &v)?;
            }
            if ["accepted", "running"].contains(&v["status"].as_str().unwrap_or("")) {
                v["status"] = json!("unknown");
                v["ended_at"] = json!(now());
                v["error"] = json!({"code":"worker_restarted","message":"Execution interrupted; effects may have occurred. Never automatically replay."});
                let journal = dir.join("process.json");
                if journal.exists() {
                    v["recovery"] = json!(process::recover(&journal).err());
                }
                wire::atomic_json(&p, &v)?;
                crate::logging::event(
                    "worker",
                    "task_recovered",
                    json!({"task_id":v["task_id"],"status":"unknown","reason":"worker_restarted"}),
                );
            }
        }
        let backends = config
            .mcp
            .keys()
            .map(|n| (n.clone(), Arc::new(Mutex::new(None))))
            .collect();
        let policy = if data.join("desktop-policy.json").exists() {
            serde_json::from_slice(&std::fs::read(data.join("desktop-policy.json"))?)?
        } else {
            Policy::default()
        };
        let safety = if data.join("safety.json").exists() {
            serde_json::from_slice(&std::fs::read(data.join("safety.json"))?)?
        } else {
            crate::safety::Safety::default()
        };
        let (changes, _) = watch::channel(0);
        Ok(Arc::new(Self {
            history: crate::history::TaskHistory::new(data.clone()),
            metrics_store: crate::metrics::Store::open(data.clone())
                .inspect_err(|_| {
                    crate::logging::event("worker", "metrics_index_failed", json!({}));
                })
                .ok(),
            data,
            config,
            submissions: Mutex::new(()),
            tasks: Mutex::new(BTreeMap::new()),
            backends,
            policy: Mutex::new(policy),
            safety: Mutex::new(safety),
            approvals: Mutex::new(BTreeMap::new()),
            rules: Mutex::new(Vec::new()),
            backend_stop: Mutex::new(CancellationToken::new()),
            tool_counts: Mutex::new(BTreeMap::new()),
            changes,
            fingerprint_key,
        }))
    }
    fn fingerprint(&self, a: &Value) -> Result<String> {
        Ok(
            blake3::keyed_hash(&self.fingerprint_key, &serde_json::to_vec(a)?)
                .to_hex()
                .to_string(),
        )
    }
    pub fn changed(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    pub async fn policy(&self) -> Policy {
        self.policy.lock().await.clone()
    }
    async fn check_admission(&self, kind: &str) -> Result<()> {
        let p = self.policy.lock().await;
        if p.paused {
            return Err(crate::model::Fault::new("busy", "worker paused by local user").into());
        }
        if matches!(kind, "mcp.call" | "desktop.sequence") && !p.desktop_enabled {
            return Err(crate::model::Fault::new(
                "desktop_disabled",
                "desktop control disabled by local user",
            )
            .into());
        }
        drop(p);
        if matches!(kind, "mcp.call" | "desktop.sequence")
            && self.safety.lock().await.yield_until > now()
        {
            return Err(crate::model::Fault::new(
                "busy",
                "desktop temporarily yielded to local input",
            )
            .into());
        }
        Ok(())
    }
    /// Only the local control endpoint can change policy. Admission and stopping are serialized.
    pub async fn set_policy(
        &self,
        paused: Option<bool>,
        desktop: Option<bool>,
        stop: bool,
    ) -> Result<Value> {
        let _admission = self.submissions.lock().await;
        let mut p = self.policy.lock().await;
        let mut next = p.clone();
        if let Some(v) = paused {
            next.paused = v;
        }
        if let Some(v) = desktop {
            next.desktop_enabled = v;
        }
        if stop {
            next.paused = true;
            next.desktop_enabled = false;
        }
        // Fail closed in memory even if persisting an emergency stop fails.
        if stop {
            let mut backend_stop = self.backend_stop.lock().await;
            backend_stop.cancel();
            *backend_stop = CancellationToken::new();
            *p = next.clone();
            for cancel in self.tasks.lock().await.values() {
                cancel.cancel();
            }
            self.changed();
        }
        wire::persist(&self.data.join("desktop-policy.json"), &next).await?;
        *p = next;
        self.changed();
        Ok(json!({"policy":*p,"cancel_requested":stop}))
    }
    pub async fn set_safety(&self, mut safety: crate::safety::Safety) -> Result<Value> {
        let _guard = self.submissions.lock().await;
        safety.validate()?;
        wire::persist(&self.data.join("safety.json"), &safety).await?;
        *self.safety.lock().await = safety.clone();
        self.changed();
        Ok(json!(safety))
    }
    pub async fn approve(&self, ident: &str, allow: bool) -> Result<Value> {
        self.approve_scoped(ident, allow, "once").await
    }
    /// `scope` is "once", or "similar"/"session" to also grant a temporary rule.
    pub async fn approve_scoped(&self, ident: &str, allow: bool, scope: &str) -> Result<Value> {
        anyhow::ensure!(
            ["once", "similar", "session"].contains(&scope),
            "invalid approval scope"
        );
        let mut approvals = self.approvals.lock().await;
        let pending = approvals
            .get(ident)
            .ok_or_else(|| anyhow::anyhow!("approval expired or already handled"))?;
        let rule = if allow && scope != "once" {
            Some(crate::safety::AllowRule::from_request(
                &pending.kind,
                scope,
                &pending.args,
                pending.tier,
                now(),
            )?)
        } else {
            None
        };
        let pending = approvals.remove(ident).expect("checked above");
        drop(approvals);
        pending
            .sender
            .send(allow)
            .map_err(|_| anyhow::anyhow!("approval no longer active"))?;
        let rule_id = rule.as_ref().map(|r| r.id.clone());
        if let Some(rule) = rule {
            self.rules.lock().await.push(rule);
            self.changed();
        }
        Ok(json!({"handled":true,"rule_id":rule_id}))
    }
    pub async fn revoke_rule(&self, ident: &str) -> Result<Value> {
        let mut rules = self.rules.lock().await;
        let before = rules.len();
        rules.retain(|r| r.id != ident);
        let removed = before != rules.len();
        drop(rules);
        self.changed();
        Ok(json!({"removed":removed}))
    }
    pub async fn allow_rules(&self) -> Vec<crate::safety::AllowRule> {
        let now = now();
        let mut rules = self.rules.lock().await;
        rules.retain(|r| r.active(now));
        rules.clone()
    }
    async fn tool_tier(&self, server: &str, tool: &str) -> &'static str {
        self.tool_counts
            .lock()
            .await
            .get(server)
            .and_then(|known| known.tiers.get(tool).copied())
            .unwrap_or("control")
    }
    pub async fn yield_desktop(&self) -> Result<Value> {
        let mut s = self.safety.lock().await;
        s.yield_until = now() + 30_000;
        self.changed();
        Ok(json!({"until":s.yield_until}))
    }
    async fn check_paths(&self, kind: &str, args: &Value) -> Result<()> {
        let safety = self.safety.lock().await;
        for field in match kind {
            "exec.start" => vec!["cwd"],
            "sync" => vec!["remote_root"],
            "file.move" => vec!["path", "destination"],
            k if k.starts_with("file.") => vec!["path"],
            _ => vec![],
        } {
            safety.check(&expand(string(args, field)?))?;
        }
        Ok(())
    }
    pub async fn restart_backend(&self, name: &str) -> Result<Value> {
        let slot = self
            .backends
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown backend"))?;
        let mut c = tokio::time::timeout(Duration::from_secs(2), slot.lock())
            .await
            .map_err(|_| anyhow::anyhow!("backend busy; stop active calls first"))?;
        if let Some(client) = c.as_mut() {
            client.stop().await;
        }
        *c = None;
        self.tool_counts.lock().await.remove(name);
        self.changed();
        Ok(json!({"session_invalidated":true}))
    }
    pub async fn begin_external(
        &self,
        kind: &str,
        args: Value,
    ) -> Result<(String, CancellationToken)> {
        let _guard = self.submissions.lock().await;
        self.check_admission(kind).await?;
        self.check_paths(kind, &args).await?;
        if self.tasks.lock().await.len() >= 128 {
            return Err(crate::model::Fault::new("busy", "task capacity reached").into());
        }
        let ident = id();
        let cancel = CancellationToken::new();
        let args = if kind.starts_with("file.") {
            // Never record upload bytes or arbitrary file contents.
            json!({"path":args["path"],"destination":args["destination"]})
        } else {
            args
        };
        wire::persist(
            &self.directory(&ident)?.join("result.json"),
            &json!({"task_id":ident,"kind":kind,"arguments":args,"status":"running","started_at":now()}),
        ).await?;
        self.tasks
            .lock()
            .await
            .insert(ident.clone(), cancel.clone());
        self.changed();
        Ok((ident, cancel))
    }
    pub async fn external_progress(&self, ident: &str, progress: Value) -> Result<()> {
        let p = self.directory(ident)?.join("result.json");
        let mut v: Value = serde_json::from_slice(&tokio::fs::read(&p).await?)?;
        v["progress"] = progress;
        wire::persist(&p, &v).await?;
        self.changed();
        Ok(())
    }
    pub async fn finish_external(
        &self,
        ident: &str,
        error: Option<String>,
        code: Option<&str>,
    ) -> Result<()> {
        self.finish_external_metrics(ident, error, code, None).await
    }
    pub async fn finish_external_metrics(
        &self,
        ident: &str,
        error: Option<String>,
        code: Option<&str>,
        metrics: Option<crate::metrics::Metrics>,
    ) -> Result<()> {
        let p = self.directory(ident)?.join("result.json");
        let mut v: Value = serde_json::from_slice(&tokio::fs::read(&p).await?)?;
        let cancelled = self
            .tasks
            .lock()
            .await
            .get(ident)
            .is_some_and(|c| c.is_cancelled());
        v["status"] = json!(if cancelled {
            "cancelled"
        } else if code == Some("timed_out") {
            "timed_out"
        } else if error.is_some() {
            "failed"
        } else {
            "succeeded"
        });
        v["ended_at"] = json!(now());
        if let Some(metrics) = metrics {
            v["metrics"] = json!(metrics);
            v["metrics_version"] = json!(env!("CARGO_PKG_VERSION"));
        }
        if let Some(message) = error {
            v["error"] = json!({"message":message});
        }
        if v["kind"] == "sync" {
            let index = self.data.join("workspaces.json");
            let mut workspaces: BTreeMap<String, Value> = std::fs::read(&index)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            if let Some(root) = v["arguments"]["remote_root"].as_str() {
                workspaces.insert(root.into(),json!({"root":root,"time":v["started_at"],"status":v["status"],"error":v["error"]}));
                if wire::persist(&index, &workspaces).await.is_err() {
                    crate::logging::event(
                        "worker",
                        "workspace_index_failed",
                        json!({"task_id":ident}),
                    );
                }
            }
        }
        let result = wire::persist(&p, &v).await;
        self.tasks.lock().await.remove(ident);
        self.changed();
        result
    }
    pub async fn active_count(&self) -> usize {
        self.tasks.lock().await.len()
    }
    pub(crate) async fn reserve_idle_shutdown(&self) -> Result<IdleShutdownGuard<'_>> {
        let admission = self.submissions.lock().await;
        anyhow::ensure!(
            self.tasks.lock().await.is_empty(),
            "仍有任务运行，请等待任务结束后再添加服务器"
        );
        Ok(IdleShutdownGuard {
            engine: self,
            _admission: admission,
        })
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.backend_stop.lock().await.cancel();
        {
            let _admission = self.submissions.lock().await;
            self.policy.lock().await.paused = true;
            for cancel in self.tasks.lock().await.values() {
                cancel.cancel();
            }
            self.changed();
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.active_count().await > 0 {
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("tasks did not stop in time"))?;
        for slot in self.backends.values() {
            let mut client = slot.lock().await;
            if let Some(c) = client.as_mut() {
                c.stop().await;
            }
            *client = None;
        }
        self.history.checkpoint().await?;
        Ok(())
    }
    pub async fn self_test(self: &Arc<Self>) -> Result<Value> {
        let root = self
            .safety
            .lock()
            .await
            .roots
            .first()
            .cloned()
            .unwrap_or_else(|| self.data.join("self-test"));
        std::fs::create_dir_all(&root)?;
        self.handle(
            "exec.start",
            json!({"command":"printf macrun-self-test","cwd":root,"timeout_seconds":10}),
        )
        .await
    }
    pub async fn prune_history(&self) -> Result<Value> {
        let _guard = self.submissions.lock().await;
        let days = self.safety.lock().await.retention_days as u64;
        let cutoff = now().saturating_sub(days * 86_400_000);
        let mut removed = 0;
        let mut transfers_removed = 0;
        for record in self.history.records().await? {
            let Some(ident) = record["task_id"].as_str() else {
                continue;
            };
            let dir = self.directory(ident)?;
            if record["kind"] == "file.upload" {
                let path = dir.clone();
                transfers_removed += crate::wire::blocking(move || {
                    crate::retention::transfers(&path, now().saturating_sub(86_400_000))
                })
                .await?? as usize;
            }
            if record["ended_at"].as_u64().is_some_and(|end| end < cutoff) {
                let v: Value =
                    serde_json::from_slice(&tokio::fs::read(dir.join("result.json")).await?)?;
                let tombstone = json!({"task_id":ident,"kind":v["kind"],"request_fingerprint":v["request_fingerprint"],"status":v["status"],"started_at":v["started_at"],"ended_at":v["ended_at"],"history_expired":true});
                wire::persist(
                    &self.data.join("dedup").join(format!("{ident}.json")),
                    &tombstone,
                )
                .await?;
                tokio::fs::remove_dir_all(dir).await?;
                self.history.removed(ident);
                if let Some(store) = &self.metrics_store {
                    let _ = store.remove(ident);
                }
                removed += 1;
            } else if record["ended_at"]
                .as_u64()
                .is_some_and(|at| at < now().saturating_sub(7 * 86_400_000))
            {
                let path = dir.join("result.json");
                let mut value: Value = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
                if crate::metrics::prune_samples(&mut value, now().saturating_sub(7 * 86_400_000)) {
                    wire::persist(&path, &value).await?;
                }
            }
        }
        let attempts = self.data.join("metrics-attempts");
        let store = self.metrics_store.clone();
        let attempts_removed = wire::blocking(move || -> Result<usize> {
            let mut removed = 0;
            if !attempts.exists() {
                return Ok(0);
            }
            for entry in std::fs::read_dir(attempts)? {
                let entry = entry?;
                let path = entry.path();
                if !entry.file_type()?.is_file() || path.extension().is_none_or(|x| x != "json") {
                    continue;
                }
                let Some(mut record) = std::fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                else {
                    continue;
                };
                if record["ended_at"].as_u64().is_some_and(|at| at < cutoff) {
                    std::fs::remove_file(path)?;
                    if let (Some(store), Some(id)) = (&store, record["task_id"].as_str()) {
                        store.remove(id)?;
                    }
                    removed += 1;
                } else if crate::metrics::prune_samples(
                    &mut record,
                    now().saturating_sub(7 * 86_400_000),
                ) {
                    crate::wire::atomic_json(&path, &record)?;
                }
            }
            Ok(removed)
        })
        .await??;
        self.changed();
        Ok(
            json!({"removed":removed,"transfers_removed":transfers_removed,"metrics_attempts_removed":attempts_removed,"dedup_preserved":true}),
        )
    }
    pub async fn local_snapshot(&self) -> Result<Value> {
        let tasks = self.history.shared_records().await?;
        let total = tasks.len();
        let mut task_counts: BTreeMap<String, usize> = BTreeMap::new();
        for task in tasks.iter() {
            *task_counts
                .entry(task["status"].as_str().unwrap_or("unknown").into())
                .or_default() += 1;
        }
        let day = |ms: u64| {
            let t = (ms / 1000) as libc::time_t;
            let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
            unsafe {
                libc::localtime_r(&t, tm.as_mut_ptr());
                let tm = tm.assume_init();
                (tm.tm_year, tm.tm_yday)
            }
        };
        let today = day(now());
        let mut summary: BTreeMap<String, u64> = BTreeMap::from([("total".into(), 0)]);
        for t in tasks.iter() {
            if day(t["started_at"].as_u64().unwrap_or(0)) == today {
                *summary.entry("total".into()).or_default() += 1;
                *summary
                    .entry(t["status"].as_str().unwrap_or("unknown").into())
                    .or_default() += 1;
                if crate::history::exited_nonzero(t) {
                    *summary.entry("exited".into()).or_default() += 1;
                }
            }
        }

        let mut workspaces: BTreeMap<String, Value> =
            std::fs::read(self.data.join("workspaces.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
        for t in tasks.iter() {
            if t["kind"] == "sync"
                && let Some(root) = t["arguments"]["remote_root"].as_str()
            {
                workspaces.entry(root.to_owned()).or_insert(json!({"root":root,"time":t["started_at"],"status":t["status"],"error":t["error"]}));
            }
        }
        // Include all active tasks even when more than 200 newer tasks exist.
        let mut tasks: Vec<Value> = tasks
            .iter()
            .enumerate()
            .filter(|(i, v)| *i < 200 || crate::history::active(v))
            .map(|(_, v)| v.clone())
            .collect();
        for v in &mut tasks {
            // Historical output is loaded with task_detail on demand. Only live
            // activity cards need output in the frequent subscription snapshot.
            if !crate::history::active(v) {
                continue;
            }
            let ident = v["task_id"].as_str().unwrap_or("");
            let p = self.directory(ident)?.join("output.log");
            if let Ok(meta) = tokio::fs::metadata(&p).await {
                let offset = meta.len().saturating_sub(8192);
                if let Ok(out) = crate::files::handle(
                    "file.read",
                    &json!({"path":p,"offset":offset,"length":8192,"text":true}),
                )
                .await
                {
                    v["output_tail"] = out["text"].clone();
                }
            }
        }
        let mut backends = Vec::new();
        for (name, slot) in &self.backends {
            let (state, session) = match slot.try_lock() {
                Ok(c) => (
                    if c.is_some() { "ready" } else { "not_started" },
                    c.as_ref().map(|c| c.generation.clone()),
                ),
                Err(_) => ("busy", None),
            };
            let tool_count = self
                .tool_counts
                .lock()
                .await
                .get(name)
                .filter(|known| {
                    state == "busy" || session.as_deref() == Some(known.session.as_str())
                })
                .map(|known| known.names.len());
            let tiers = self
                .tool_counts
                .lock()
                .await
                .get(name)
                .filter(|known| tool_count.is_some() && !known.tiers.is_empty())
                .map(|known| known.tiers.clone());
            backends.push(json!({"name":name,"state":state,"session":session,"command":self.config.mcp[name].command,"tool_count":tool_count,"tiers":tiers}));
        }
        Ok(
            json!({"today_summary":summary,"task_counts":task_counts,"workspaces":workspaces.values().collect::<Vec<_>>(),"safety":self.safety.lock().await.clone(),"policy":self.policy().await,"tasks":tasks,"total_tasks":total,"active_count":self.active_count().await,"backends":backends,"allow_rules":self.allow_rules().await,"version":env!("CARGO_PKG_VERSION"),"protocol":crate::model::PROTOCOL}),
        )
    }
    pub async fn local_task_list(&self, args: Value) -> Result<Value> {
        self.history.page(args).await
    }
    pub fn metrics_data(&self) -> PathBuf {
        self.data.clone()
    }
    pub async fn record_metrics_attempt(
        &self,
        kind: &str,
        metrics: crate::metrics::Metrics,
        succeeded: bool,
    ) -> Result<()> {
        let ident = id();
        let value = json!({"task_id":ident,"kind":kind,"status":if succeeded {"succeeded"} else {"failed"},"started_at":now().saturating_sub(metrics.wall_ms as u64),"ended_at":now(),"metrics":metrics,"metrics_version":env!("CARGO_PKG_VERSION")});
        let dir = self.data.join("metrics-attempts");
        tokio::fs::create_dir_all(&dir).await?;
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).await?;
        wire::persist(&dir.join(format!("{ident}.json")), &value).await?;
        if let Some(store) = self.metrics_store.clone()
            && wire::blocking(move || store.record(&value)).await?.is_err()
        {
            crate::logging::event("worker", "metrics_index_failed", json!({}));
        }
        Ok(())
    }
    pub fn metrics_capture(&self, a: &Value) -> crate::metrics::Capture {
        let capture = crate::metrics::Capture::new("worker");
        capture.update(|m| {
            for key in ["server", "tool", "resource_group", "desktop_tier"] {
                if let Some(value) = a[key].as_str() {
                    m.dimensions.insert(
                        key.into(),
                        value.chars().filter(|c| !c.is_control()).take(80).collect(),
                    );
                }
            }
            if let Some(value) = a["snapshot"].as_bool() {
                m.dimensions.insert("snapshot".into(), value.to_string());
            }
        });
        if let Some(root) = a["workspace_root"]
            .as_str()
            .or(a["remote_root"].as_str())
            .or(a["cwd"].as_str())
        {
            capture.project(&expand(root), &self.fingerprint_key);
        }
        capture
    }
    pub(crate) fn directory(&self, id: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(id)?;
        Ok(self.data.join("tasks").join(id))
    }
    pub fn owns_task(&self, ident: &str) -> bool {
        uuid::Uuid::parse_str(ident).is_ok()
            && (self
                .data
                .join("tasks")
                .join(ident)
                .join("result.json")
                .is_file()
                || self
                    .data
                    .join("dedup")
                    .join(format!("{ident}.json"))
                    .is_file())
    }
    pub async fn handle(self: &Arc<Self>, kind: &str, a: Value) -> Result<Value> {
        match kind {
            "exec.start" | "mcp.call" | "desktop.sequence" => {
                let wait = a["wait_ms"].as_u64().unwrap_or(0).min(1000);
                let refs = a["artifact_refs"] == true;
                let mut args = a;
                if let Some(object) = args.as_object_mut() {
                    object.remove("wait_ms");
                    object.remove("artifact_refs");
                }
                let value = self.submit(kind, args, false, None).await?;
                if wait == 0 {
                    return Ok(value);
                }
                self.wait_task(
                    json!({"task_id":value["task_id"],"wait_ms":wait,"artifact_refs":refs}),
                )
                .await
            }
            "task.wait" => self.wait_task(a).await,
            "task.get" => {
                // Keep the record and its output alive for the whole read while
                // retention cleanup moves completed tasks into dedup records.
                let _guard = self.submissions.lock().await;
                let dir = self.directory(string(&a, "task_id")?)?;
                let mut v: Value = serde_json::from_slice(
                    &tokio::fs::read(if dir.join("result.json").exists() {
                        dir.join("result.json")
                    } else {
                        self.data
                            .join("dedup")
                            .join(format!("{}.json", string(&a, "task_id")?))
                    })
                    .await?,
                )?;
                let log = dir.join("output.log");
                if log.exists() {
                    let tail = a["tail_bytes"].as_u64().map(|n| n.clamp(1, 65536));
                    let offset = if let Some(length) = tail {
                        tokio::fs::metadata(&log)
                            .await?
                            .len()
                            .saturating_sub(length)
                    } else {
                        a["offset"].as_u64().unwrap_or(0)
                    };
                    let args = json!({"path":log,"offset":offset,"length":tail.unwrap_or(65536),"text":true});
                    let mut output = crate::files::handle("file.read", &args).await?;
                    output.as_object_mut().unwrap().remove("data");
                    v["output"] = output;
                }
                if a["include_result"] == false {
                    v.as_object_mut().unwrap().remove("result");
                } else if a["artifact_refs"] != true {
                    v = crate::wire::blocking(move || {
                        crate::artifact::hydrate(&dir, &mut v)?;
                        Ok::<_, anyhow::Error>(v)
                    })
                    .await??;
                }
                Ok(v)
            }
            "task.cancel" => {
                let _guard = self.submissions.lock().await;
                let id = string(&a, "task_id")?;
                let mut p = self.directory(id)?.join("result.json");
                if let Some(c) = self.tasks.lock().await.get(id) {
                    c.cancel();
                    return Ok(json!({"task_id":id,"cancel_requested":true}));
                }
                if !p.exists() {
                    p = self.data.join("dedup").join(format!("{id}.json"));
                }
                let v: Value = serde_json::from_slice(&tokio::fs::read(p).await?)?;
                Ok(json!({"task_id":id,"cancel_requested":false,"status":v["status"]}))
            }
            "mcp.servers" => Ok(json!({"servers":self.config.mcp.keys().collect::<Vec<_>>()})),
            "mcp.tools" => {
                let v = self
                    .backend_request(&a, "tools/list", &CancellationToken::new(), None)
                    .await?;
                self.changed();
                Ok(v)
            }
            "performance.report" => {
                let job = string(&a, "job_id")?;
                uuid::Uuid::parse_str(job)?;
                let metrics: crate::metrics::Metrics =
                    serde_json::from_value(a["metrics"].clone())?;
                crate::metrics::validate_report(&metrics)?;
                let _guard = self.submissions.lock().await;
                let task = self
                    .history
                    .records()
                    .await?
                    .into_iter()
                    .find(|r| r["arguments"]["job_id"] == job)
                    .ok_or_else(|| anyhow::anyhow!("unknown performance operation"))?;
                let p = self
                    .directory(string(&task, "task_id")?)?
                    .join("result.json");
                let mut record: Value = serde_json::from_slice(&tokio::fs::read(&p).await?)?;
                anyhow::ensure!(
                    record["ended_at"].as_u64().is_some(),
                    "performance operation still running; retry later"
                );
                record["server_metrics"] = json!(metrics);
                wire::persist(&p, &record).await?;
                Ok(json!({"recorded":true}))
            }
            _ if kind.starts_with("file.") => {
                let (ident, cancel) = self.begin_external(kind, a.clone()).await?;
                let lease = if matches!(kind, "file.write" | "file.move") {
                    let mut paths = vec![expand(string(&a, "path")?)];
                    if kind == "file.move" {
                        paths.push(expand(string(&a, "destination")?));
                    }
                    crate::workspace::Lease::acquire(&paths).map(Some)
                } else {
                    Ok(None)
                };
                let r = if let Err(e) = lease {
                    Err(e)
                } else {
                    let _lease = lease?;
                    tokio::select! { r=crate::files::handle(kind,&a)=>r, _=cancel.cancelled()=>Err(anyhow::anyhow!("cancelled")) }
                };

                self.finish_external(&ident, r.as_ref().err().map(ToString::to_string), None)
                    .await?;
                r
            }
            _ => bail!("unknown operation: {kind}"),
        }
    }
    async fn wait_task(self: &Arc<Self>, mut args: Value) -> Result<Value> {
        let wait = args["wait_ms"].as_u64().unwrap_or(1000).min(25_000);
        args.as_object_mut().map(|o| o.remove("wait_ms"));
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
        let mut changed = self.subscribe();
        loop {
            let value = Box::pin(self.handle("task.get", args.clone())).await?;
            if value.get("ended_at").is_some() || value["history_expired"] == true {
                return Ok(value);
            }
            if tokio::time::timeout_at(deadline, changed.changed())
                .await
                .is_err()
            {
                return Ok(value);
            }
        }
    }
    /// The desktop app's own observation, sent over the private control socket.
    /// Only this path may carry `local_observation`; it never waits for approval.
    pub async fn observe_locally(self: &Arc<Self>, mut a: Value) -> Result<Value> {
        a["local_observation"] = json!(true);
        self.submit("mcp.call", a, true, None).await
    }
    async fn submit(
        self: &Arc<Self>,
        kind: &str,
        mut a: Value,
        local: bool,
        reservation: Option<Arc<crate::scheduler::Reservation>>,
    ) -> Result<Value> {
        // Remote requests must not impersonate the local app: the flag skips
        // approval and hides the on-screen overlay.
        if !local && a.get("local_observation").is_some() {
            bail!("local_observation is reserved for the local desktop app");
        }
        let task_id = a["request_id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(id);
        let dir = self.directory(&task_id)?;
        a.as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("arguments must be an object"))?
            .remove("request_id");
        if kind == "exec.start" {
            string(&a, "command")?;
            string(&a, "cwd")?;
            if let Some(outputs) = a.get("outputs") {
                let outputs: Vec<String> = serde_json::from_value(outputs.clone())?;
                anyhow::ensure!(
                    outputs.len() <= 32 && outputs.iter().all(|p| !p.is_empty() && p.len() <= 1024),
                    "outputs must contain at most 32 bounded relative paths"
                );
                if outputs.is_empty() {
                    a.as_object_mut().unwrap().remove("outputs");
                }
            }
            if let Some(env) = a.get("env") {
                let _: BTreeMap<String, String> = serde_json::from_value(env.clone())?;
            }
        } else {
            let server = string(&a, "server")?;
            if kind == "desktop.sequence" {
                let steps = a["steps"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("steps required"))?;
                anyhow::ensure!(
                    !steps.is_empty() && steps.len() <= 16,
                    "one to sixteen steps required"
                );
                string(&a, "session")?;
                for step in steps {
                    string(step, "tool")?;
                    if let Some(expect) = step.get("expect") {
                        string(expect, "pointer")?;
                        anyhow::ensure!(expect.get("equals").is_some(), "expect.equals required");
                    }
                }
            } else {
                string(&a, "tool")?;
            }
            if !self.backends.contains_key(server) {
                bail!("unknown MCP server");
            }
        }
        if a.get("timeout_seconds").is_some()
            && a["timeout_seconds"].as_u64().is_none_or(|v| v == 0)
        {
            bail!("timeout_seconds must be positive");
        }
        let _guard = self.submissions.lock().await;
        let path = dir.join("result.json");
        let tombstone = self.data.join("dedup").join(format!("{task_id}.json"));
        if path.exists() || tombstone.exists() {
            let v: Value = serde_json::from_slice(&std::fs::read(if path.exists() {
                &path
            } else {
                &tombstone
            })?)?;
            if v["kind"] != kind || v["request_fingerprint"] != json!(self.fingerprint(&a)?) {
                bail!("request_id already belongs to a different operation");
            }
            return Ok(json!({"task_id":task_id,"status":v["status"],"duplicate":true}));
        }
        self.check_admission(kind).await?;
        if self.tasks.lock().await.len() >= 128 {
            return Err(crate::model::Fault::new("busy", "task capacity reached").into());
        }
        self.check_paths(kind, &a).await?;
        let tier = if kind == "mcp.call" {
            Some(
                self.tool_tier(string(&a, "server")?, string(&a, "tool")?)
                    .await,
            )
        } else {
            None
        };
        let needs_approval = {
            let safety = self.safety.lock().await;
            match tier {
                None if kind == "desktop.sequence" => false,
                None => safety.approval_required(string(&a, "command")?),
                Some(tier) => {
                    if safety.desktop.policy(tier) == "deny" {
                        return Err(crate::model::Fault::new(
                            "desktop_denied",
                            format!("{tier} desktop tools are disabled by local user"),
                        )
                        .into());
                    }
                    // The local app's own observation is a direct user action.
                    safety.desktop.policy(tier) == "confirm" && !local
                }
            }
        };
        let rule = if needs_approval {
            let now = now();
            self.rules
                .lock()
                .await
                .iter()
                .find(|r| r.matches(kind, &a, tier, now))
                .map(|r| r.id.clone())
        } else {
            None
        };
        let needs_approval = needs_approval && rule.is_none();
        let capture = self.metrics_capture(&a);
        let mut initial = json!({"task_id":task_id,"kind":kind,"arguments":redact_arguments(&a),"request_fingerprint":self.fingerprint(&a)?,"status":"accepted","started_at":now(),"result_path":path,"output_path":dir.join("output.log")});
        if let Some(tier) = tier {
            initial["desktop_tier"] = json!(tier);
        }
        if let Some(rule) = rule {
            initial["approved_by_rule"] = json!(rule);
        }
        wire::persist(&path, &initial).await?;
        let cancel = CancellationToken::new();
        let approval = if needs_approval {
            let (sender, rx) = tokio::sync::oneshot::channel();
            self.approvals.lock().await.insert(
                task_id.clone(),
                Pending {
                    sender,
                    kind: kind.to_owned(),
                    args: a.clone(),
                    tier,
                },
            );
            Some(rx)
        } else {
            None
        };
        self.tasks
            .lock()
            .await
            .insert(task_id.clone(), cancel.clone());
        self.changed();
        let engine = self.clone();
        let ident = task_id.clone();
        let kind = kind.to_owned();
        tokio::spawn(async move {
            let mut result = initial;
            if let Some(reserved) = &reservation {
                capture.update(|m| m.parent_task_id = Some(reserved.parent_task_id.clone()));
            }
            let outcome = capture.scope(async {
                if let Some(approval)=approval {
                    let _phase=crate::metrics::phase("approval_wait");
                    result["status"]=json!("awaiting_approval");result["approval_deadline"]=json!(now()+60_000);
                    wire::persist(&path,&result).await?;engine.changed();
                    let decision=tokio::select! {
                        v=tokio::time::timeout(Duration::from_secs(60),approval)=>v,
                        _=cancel.cancelled()=>{bail!("cancelled before approval");}
                    };
                    engine.approvals.lock().await.remove(&ident);
                    match decision {
                        Ok(Ok(true)) => {}
                        Ok(_) => bail!("denied: approval rejected by local user"),
                        Err(_) => bail!("denied: approval expired after 60 seconds without a local decision; the user may be away, retry later or ask them"),
                    }
                }
                engine.check_paths(&kind,&a).await?;
                result["status"] = json!("running");
                wire::persist(&path, &result).await?;
                engine.changed();
                if cancel.is_cancelled() {
                    bail!("cancelled before dispatch");
                }
                if kind == "exec.start" {
                    engine.command(&a, &dir, &cancel).await
                } else if kind=="desktop.sequence" {
                    engine.clone().run_sequence(a.clone(),ident.clone(),cancel.clone()).await
                } else {
                    engine.backend_request(&a, "tools/call", &cancel, reservation.as_ref()).await
                }
            }).await;
            match outcome {
                Ok(v) => {
                    result["status"] = json!(if v["exit_code"].as_i64().is_some_and(|n| n != 0)
                        || v.pointer("/result/isError") == Some(&json!(true))
                    {
                        "failed"
                    } else {
                        "succeeded"
                    });
                    let dir = dir.clone();
                    let stored = capture
                        .scope(crate::wire::blocking(move || {
                            let _phase = crate::metrics::phase("artifact_store");
                            let mut value = v;
                            let artifacts = crate::artifact::store(&dir, &mut value)?;
                            Ok::<_, anyhow::Error>((value, artifacts))
                        }))
                        .await;
                    match stored {
                        Ok(Ok((value, artifacts))) => {
                            result["result"] = value;
                            result["artifacts"] = json!(artifacts);
                        }
                        other => {
                            result["status"] = json!("unknown");
                            result["error"] = json!({"message":format!("could not persist result artifacts: {other:?}")});
                        }
                    }
                }
                Err(e) => {
                    let text = e.to_string();
                    result["status"] = json!(if text.starts_with("denied:") {
                        "denied"
                    } else if matches!(kind.as_str(), "mcp.call" | "desktop.sequence")
                        && !text.starts_with("stale_session")
                        && !text.starts_with("desktop_disabled")
                        && !text.starts_with("cancelled before")
                        && !text.starts_with("timed_out waiting")
                    {
                        "unknown"
                    } else if cancel.is_cancelled() {
                        "cancelled"
                    } else if text.contains("timed_out") {
                        "timed_out"
                    } else {
                        "failed"
                    });
                    result["error"] = json!({"message":text});
                    if let Some(fault) = e.downcast_ref::<crate::model::Fault>() {
                        result["error"]["code"] = json!(fault.code);
                    }
                    if let Some(code) = denial_code(&text) {
                        result["error"]["code"] = json!(code);
                    }
                }
            }
            result["ended_at"] = json!(now());
            if let Some(artifacts) = result["artifacts"].as_array() {
                capture.add("files", "artifacts", artifacts.len() as u64);
                for a in artifacts {
                    capture.add("bytes", "artifacts", a["bytes"].as_u64().unwrap_or(0));
                }
            }
            result["metrics"] = json!(capture.snapshot(true));
            result["metrics_version"] = json!(env!("CARGO_PKG_VERSION"));
            if wire::persist(&path, &result).await.is_err() {
                crate::logging::event("worker", "persist_failed", json!({"task_id":ident}));
            }
            crate::logging::event(
                "worker",
                "task_finished",
                json!({
                    "operation":kind,"task_id":ident,"status":result["status"],
                    "duration_ms":now().saturating_sub(result["started_at"].as_u64().unwrap_or(now())),
                    "exit_code":result.pointer("/result/exit_code")
                }),
            );
            engine.approvals.lock().await.remove(&ident);
            engine.tasks.lock().await.remove(&ident);
            engine.changed();
        });
        Ok(
            json!({"task_id":task_id,"status":"accepted","result_path":self.directory(&task_id)?.join("result.json")}),
        )
    }
    async fn command(
        &self,
        a: &Value,
        dir: &std::path::Path,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        static EXEC_SLOTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> =
            std::sync::OnceLock::new();
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(a["timeout_seconds"].as_u64().unwrap_or(3600));
        let queue = crate::metrics::phase("exec_slot_wait");
        let _slot = tokio::select! {
            slot=tokio::time::timeout_at(deadline,EXEC_SLOTS.get_or_init(||Arc::new(tokio::sync::Semaphore::new(4))).clone().acquire_owned())=>slot.map_err(|_|anyhow::anyhow!("timed_out waiting for command capacity"))??,
            _=cancel.cancelled()=>bail!("cancelled before command dispatch"),
        };
        drop(queue);
        let cwd = expand(string(a, "cwd")?);
        let root = a["workspace_root"]
            .as_str()
            .map(expand)
            .unwrap_or_else(|| cwd.clone());
        self.safety.lock().await.check(&root)?;
        anyhow::ensure!(
            crate::workspace::resolve(&cwd)?.starts_with(crate::workspace::resolve(&root)?),
            "cwd is outside workspace_root"
        );
        let mut lease = Some(Arc::new(crate::workspace::Lease::acquire(
            std::slice::from_ref(&root),
        )?));
        if let Some(expected) = a["generation"].as_str() {
            anyhow::ensure!(
                crate::workspace::generation(&root).ok().as_deref() == Some(expected),
                "stale_workspace: synchronize and use the returned generation"
            );
        }
        let mut run_cwd = cwd.clone();
        if a["snapshot"] == true {
            let _phase = crate::metrics::phase("snapshot_copy");
            let generation = crate::workspace::generation(&root)?;
            let state = self.data.join("mirrors").join(
                blake3::hash(root.to_string_lossy().as_bytes())
                    .to_hex()
                    .as_str(),
            );
            let source = root.clone();
            let destination = dir.join("workspace");
            let output = destination.clone();
            let guard = lease.as_ref().unwrap().clone();
            let relative = crate::workspace::resolve(&cwd)?
                .strip_prefix(crate::workspace::resolve(&root)?)?
                .to_path_buf();
            crate::wire::blocking(move || {
                let _guard = guard;
                crate::snapshot::create(&source, &state, &destination, &generation)
            })
            .await??;
            lease.take();
            run_cwd = output.join(relative);
            lease = Some(Arc::new(crate::workspace::Lease::acquire(
                std::slice::from_ref(&output),
            )?));
        }
        let _lease = lease;
        let env: BTreeMap<String, String> =
            serde_json::from_value(a.get("env").cloned().unwrap_or(json!({})))?;
        let (tx, mut rx) = mpsc::channel(64);
        let p = dir.join("output.log");
        let writer = tokio::spawn(async move {
            let mut f = tokio::fs::File::create(p).await?;
            while let Some(e) = rx.recv().await {
                if let Event::Log { text, .. } = e {
                    f.write_all(text.as_bytes()).await?;
                }
            }
            f.sync_all().await?;
            Ok::<_, anyhow::Error>(())
        });
        let execution = crate::metrics::phase("process_run");
        let r = process::run_command_until(
            "/bin/sh",
            &["-c".into(), string(a, "command")?.into()],
            &run_cwd,
            &env,
            deadline,
            cancel,
            &tx,
            "output.log",
            &dir.join("process.json"),
        )
        .await;
        drop(execution);
        drop(tx);
        let drain = crate::metrics::phase("log_drain");
        writer.await??;
        drop(drain);
        if let Ok(meta) = tokio::fs::metadata(dir.join("output.log")).await {
            crate::metrics::set("bytes", "stored_log", meta.len());
        }
        if let Some(declared) = a["outputs"].as_array() {
            let outputs = declared
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let root = run_cwd.clone();
            let measured = wire::blocking(move || {
                let _phase = crate::metrics::phase("output_measure");
                crate::metrics::measure_output_totals(&root, &outputs)
            })
            .await?;
            if let Some(c) = crate::metrics::current() {
                c.update(|m| {
                    m.outputs = measured.0;
                    if let Some(bytes) = measured.1 {
                        m.bytes.insert("declared_outputs".into(), bytes);
                        m.files.insert("declared_outputs".into(), measured.2);
                    }
                });
            }
        }
        Ok(json!({"exit_code":r?,"workspace":run_cwd,"snapshot":a["snapshot"]==true}))
    }
    fn run_sequence(
        self: Arc<Self>,
        args: Value,
        parent: String,
        cancel: CancellationToken,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>> {
        Box::pin(async move {
            let server = string(&args, "server")?;
            let group = self
                .config
                .resource_groups
                .get(server)
                .map(String::as_str)
                .unwrap_or("desktop");
            let deadline = tokio::time::Instant::now()
                + Duration::from_secs(args["timeout_seconds"].as_u64().unwrap_or(120).min(600));
            let waiting = crate::metrics::phase("resource_wait");
            if let Some(c) = crate::metrics::current() {
                c.update(|m| {
                    m.dimensions.insert("resource_group".into(), group.into());
                });
            }
            let guard = tokio::select! {
                r=tokio::time::timeout_at(deadline,crate::scheduler::resource(group).lock_owned())=>r.map_err(|_|anyhow::anyhow!("timed_out waiting for sequence resource"))?,
                _=cancel.cancelled()=>bail!("cancelled before sequence dispatch"),
            };
            drop(waiting);
            let reserved = Arc::new(crate::scheduler::Reservation {
                parent_task_id: parent.clone(),
                group: group.into(),
                guard,
            });
            let mut results = Vec::new();
            let mut last = json!({});
            for (index, step) in args["steps"].as_array().unwrap().iter().enumerate() {
                if cancel.is_cancelled() || tokio::time::Instant::now() >= deadline {
                    bail!("sequence cancelled or timed_out before next step")
                }
                let hash = blake3::hash(format!("sequence:{parent}:{index}").as_bytes());
                let request = uuid::Uuid::from_slice(&hash.as_bytes()[..16])?.to_string();
                let child_args = json!({"server":server,"session":args["session"],"tool":step["tool"],"arguments":step.get("arguments").cloned().unwrap_or(json!({})),"request_id":request,"timeout_seconds":deadline.saturating_duration_since(tokio::time::Instant::now()).as_secs().max(1)});
                let child = self
                    .submit("mcp.call", child_args, false, Some(reserved.clone()))
                    .await?;
                let task = child["task_id"].clone();
                let outcome = async {
                    loop {
                        let value = self
                            .wait_task(json!({"task_id":task,"wait_ms":25000}))
                            .await?;
                        if value.get("ended_at").is_some() {
                            return Ok::<_, anyhow::Error>(value);
                        }
                    }
                };
                let done = tokio::select! {
                    value=tokio::time::timeout_at(deadline,outcome)=>value.unwrap_or_else(|_|Err(anyhow::anyhow!("sequence timed_out"))),
                    _=cancel.cancelled()=>Err(anyhow::anyhow!("sequence cancelled")),
                };
                let done = match done {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = self.handle("task.cancel", json!({"task_id":task})).await;
                        return Err(error);
                    }
                };
                results.push(json!({"index":index,"task_id":task,"status":done["status"]}));
                self.external_progress(&parent, json!({"steps":results}))
                    .await?;
                if done["status"] == "unknown" {
                    bail!("sequence stopped: step {index} has unknown effects; task_id={task}");
                }
                if done["status"] != "succeeded" {
                    return Ok(
                        json!({"steps":results,"result":{"isError":true,"content":[{"type":"text","text":"sequence stopped after unsuccessful step"}]}}),
                    );
                }
                last = done["result"]["result"].clone();
                if let Some(expect) = step.get("expect") {
                    let pointer = string(expect, "pointer")?;
                    if last.pointer(pointer) != Some(&expect["equals"]) {
                        return Ok(
                            json!({"steps":results,"result":{"isError":true,"content":[{"type":"text","text":"sequence condition failed; remaining steps were not dispatched"}]}}),
                        );
                    }
                }
            }
            Ok(json!({"steps":results,"result":last}))
        })
    }
    async fn backend_request(
        &self,
        a: &Value,
        method: &str,
        cancel: &CancellationToken,
        reservation: Option<&Arc<crate::scheduler::Reservation>>,
    ) -> Result<Value> {
        let stop_all = self.backend_stop.lock().await.clone();
        let name = string(a, "server")?;
        let slot = self
            .backends
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown MCP server: {name}"))?;
        let timeout = Duration::from_secs(
            a["timeout_seconds"]
                .as_u64()
                .unwrap_or(if method == "tools/list" { 12 } else { 300 }),
        );
        let deadline = tokio::time::Instant::now() + timeout;
        let queued_at = std::time::Instant::now();
        let resource_wait = crate::metrics::phase("resource_wait");
        let group = self
            .config
            .resource_groups
            .get(name)
            .map(String::as_str)
            .unwrap_or("desktop");
        let resource = crate::scheduler::resource(group);
        if let Some(reserved) = reservation {
            anyhow::ensure!(reserved.group == group, "wrong reserved resource");
        }
        let _desktop = if method == "tools/call" && reservation.is_none() {
            Some(
                tokio::select! {v=tokio::time::timeout_at(deadline,resource.lock_owned())=>v.map_err(|_|anyhow::anyhow!("timed_out waiting for desktop"))?,_=cancel.cancelled()=>bail!("cancelled before desktop dispatch"),_=stop_all.cancelled()=>bail!("cancelled before desktop dispatch")},
            )
        } else {
            None
        };
        drop(resource_wait);
        let backend_wait = crate::metrics::phase("backend_lock_wait");
        // Waiting for the per-backend lock is cancellable, but never drops somebody else's client.
        let mut guard = tokio::select! {v=tokio::time::timeout_at(deadline,slot.lock())=>v.map_err(|_|anyhow::anyhow!("timed_out waiting for backend"))?,_=cancel.cancelled()=>bail!("cancelled before backend dispatch"),_=stop_all.cancelled()=>bail!("cancelled before backend dispatch")};
        drop(backend_wait);
        if let Some(expected) = a["session"].as_str() {
            if guard.as_ref().is_none_or(|c| c.generation != expected) {
                bail!("stale_session: rediscover tools and observe the target again");
            }
        } else if method == "tools/call" {
            bail!("stale_session: mcp.call requires session from mcp.tools");
        }
        if cancel.is_cancelled() || stop_all.is_cancelled() {
            bail!("cancelled before backend dispatch");
        }
        if method == "tools/call" && self.safety.lock().await.yield_until > now() {
            bail!("desktop_disabled: local input yield active");
        }
        if method == "tools/call" && !self.policy.lock().await.desktop_enabled {
            bail!("desktop_disabled before backend dispatch");
        }
        let queue_ms = queued_at.elapsed().as_millis();
        let operation = async {
            if guard.is_none() {
                let _phase = crate::metrics::phase("backend_start");
                *guard = Some(Client::connect(&self.config.mcp[name]).await?);
            }
            let c = guard.as_mut().unwrap();
            let params = if method == "tools/call" {
                json!({"name":string(a,"tool")?,"arguments":a.get("arguments").cloned().unwrap_or(json!({}))})
            } else {
                json!({"cursor":a.get("cursor").cloned().unwrap_or(Value::Null)})
            };
            let params = if params == json!({"cursor":null}) {
                json!({})
            } else {
                params
            };
            let call = crate::metrics::phase("backend_call");
            let result = c.rpc(method, params).await?;
            drop(call);
            if method == "tools/list" {
                // Keep counting under the backend lock so a late page cannot
                // overwrite a restarted session or a newer first-page refresh.
                let mut discovered = self.tool_counts.lock().await;
                let known = discovered.entry(name.to_owned()).or_default();
                if known.session != c.generation || a["cursor"].as_str().is_none_or(str::is_empty) {
                    known.names.clear();
                    known.tiers.clear();
                }
                known.session.clone_from(&c.generation);
                if let Some(tools) = result["tools"].as_array() {
                    for tool in tools {
                        if let Some(name) = tool["name"].as_str() {
                            known.names.insert(name.to_owned());
                            known
                                .tiers
                                .insert(name.to_owned(), crate::safety::classify_tool(tool));
                        }
                    }
                }
            }
            Ok::<Value, anyhow::Error>(
                json!({"server":name,"session":c.generation,"result":result,"queue_ms":queue_ms,"resource_group":group}),
            )
        };
        let r = tokio::select! {v=tokio::time::timeout_at(deadline,operation)=>v.unwrap_or_else(|_|Err(anyhow::anyhow!("timed_out; backend session invalidated; effect may be unknown"))),_=cancel.cancelled()=>Err(anyhow::anyhow!("cancelled; backend session invalidated; effect may be unknown")),_=stop_all.cancelled()=>Err(anyhow::anyhow!("cancelled; backend session invalidated; effect may be unknown"))};
        if r.is_err() {
            if let Some(c) = guard.as_mut() {
                c.stop().await;
            }
            *guard = None;
            self.tool_counts.lock().await.remove(name);
        }
        r
    }
}
