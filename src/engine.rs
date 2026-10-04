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
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, mpsc, watch},
};
use tokio_util::sync::CancellationToken;
type Slot = Arc<Mutex<Option<Client>>>;
pub struct Engine {
    data: PathBuf,
    config: WorkerConfig,
    submissions: Mutex<()>,
    tasks: Mutex<BTreeMap<String, CancellationToken>>,
    backends: BTreeMap<String, Slot>,
    policy: Mutex<Policy>,
    changes: watch::Sender<u64>,
    fingerprint_key: [u8; 32],
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
        let (changes, _) = watch::channel(0);
        Ok(Arc::new(Self {
            data,
            config,
            submissions: Mutex::new(()),
            tasks: Mutex::new(BTreeMap::new()),
            backends,
            policy: Mutex::new(policy),
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
        if kind == "mcp.call" && !p.desktop_enabled {
            return Err(crate::model::Fault::new(
                "desktop_disabled",
                "desktop control disabled by local user",
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
            *p = next.clone();
            for cancel in self.tasks.lock().await.values() {
                cancel.cancel();
            }
            self.changed();
        }
        wire::atomic_json(&self.data.join("desktop-policy.json"), &next)?;
        *p = next;
        self.changed();
        Ok(json!({"policy":*p,"cancel_requested":stop}))
    }
    pub async fn begin_external(
        &self,
        kind: &str,
        args: Value,
    ) -> Result<(String, CancellationToken)> {
        let _guard = self.submissions.lock().await;
        self.check_admission(kind).await?;
        let ident = id();
        let cancel = CancellationToken::new();
        let args = if kind.starts_with("file.") {
            // Never record upload bytes or arbitrary file contents.
            json!({"path":args["path"],"destination":args["destination"]})
        } else {
            args
        };
        wire::atomic_json(
            &self.directory(&ident)?.join("result.json"),
            &json!({"task_id":ident,"kind":kind,"arguments":args,"status":"running","started_at":now()}),
        )?;
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
        wire::atomic_json(&p, &v)?;
        self.changed();
        Ok(())
    }
    pub async fn finish_external(
        &self,
        ident: &str,
        error: Option<String>,
        code: Option<&str>,
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
        if let Some(message) = error {
            v["error"] = json!({"message":message});
        }
        let result = wire::atomic_json(&p, &v);
        self.tasks.lock().await.remove(ident);
        self.changed();
        result
    }
    pub async fn active_count(&self) -> usize {
        self.tasks.lock().await.len()
    }
    pub async fn shutdown(&self) -> Result<()> {
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
        Ok(())
    }
    pub async fn local_snapshot(&self) -> Result<Value> {
        let mut tasks = Vec::new();
        for entry in std::fs::read_dir(self.data.join("tasks"))? {
            let path = entry?.path().join("result.json");
            if let Ok(bytes) = std::fs::read(path)
                && let Ok(mut v) = serde_json::from_slice::<Value>(&bytes)
            {
                v.as_object_mut().map(|m| m.remove("request_fingerprint"));
                v["arguments"] = redact_arguments(&v["arguments"]);
                // Tool results can contain images and secrets; retrieve only on explicit request.
                if v["kind"] == "mcp.call" {
                    v.as_object_mut().map(|m| m.remove("result"));
                }
                tasks.push(v);
            }
        }
        tasks.sort_by_key(|v| std::cmp::Reverse(v["started_at"].as_u64().unwrap_or(0)));
        let total = tasks.len();
        tasks.truncate(200);
        for v in &mut tasks {
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
            backends.push(json!({"name":name,"state":state,"session":session,"command":self.config.mcp[name].command}));
        }
        Ok(
            json!({"policy":self.policy().await,"tasks":tasks,"total_tasks":total,"active_count":self.active_count().await,"backends":backends,"version":env!("CARGO_PKG_VERSION"),"protocol":crate::model::PROTOCOL}),
        )
    }
    fn directory(&self, id: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(id)?;
        Ok(self.data.join("tasks").join(id))
    }
    pub async fn handle(self: &Arc<Self>, kind: &str, a: Value) -> Result<Value> {
        match kind {
            "exec.start" | "mcp.call" => self.submit(kind, a).await,
            "task.get" => {
                let dir = self.directory(string(&a, "task_id")?)?;
                let mut v: Value =
                    serde_json::from_slice(&tokio::fs::read(dir.join("result.json")).await?)?;
                let log = dir.join("output.log");
                if log.exists() {
                    let args = json!({"path":log,"offset":a["offset"].as_u64().unwrap_or(0),"length":65536,"text":true});
                    let mut output = crate::files::handle("file.read", &args).await?;
                    output.as_object_mut().unwrap().remove("data");
                    v["output"] = output;
                }
                Ok(v)
            }
            "task.cancel" => {
                let id = string(&a, "task_id")?;
                let p = self.directory(id)?.join("result.json");
                if let Some(c) = self.tasks.lock().await.get(id) {
                    c.cancel();
                    return Ok(json!({"task_id":id,"cancel_requested":true}));
                }
                let v: Value = serde_json::from_slice(&tokio::fs::read(p).await?)?;
                Ok(json!({"task_id":id,"cancel_requested":false,"status":v["status"]}))
            }
            "mcp.servers" => Ok(json!({"servers":self.config.mcp.keys().collect::<Vec<_>>()})),
            "mcp.tools" => {
                self.backend_request(&a, "tools/list", &CancellationToken::new())
                    .await
            }
            _ if kind.starts_with("file.") => {
                let (ident, cancel) = self.begin_external(kind, a.clone()).await?;
                let r = tokio::select! { r=crate::files::handle(kind,&a)=>r, _=cancel.cancelled()=>Err(anyhow::anyhow!("cancelled")) };
                self.finish_external(&ident, r.as_ref().err().map(ToString::to_string), None)
                    .await?;
                r
            }
            _ => bail!("unknown operation: {kind}"),
        }
    }
    async fn submit(self: &Arc<Self>, kind: &str, mut a: Value) -> Result<Value> {
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
            if let Some(env) = a.get("env") {
                let _: BTreeMap<String, String> = serde_json::from_value(env.clone())?;
            }
        } else {
            let server = string(&a, "server")?;
            string(&a, "tool")?;
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
        if path.exists() {
            let v: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            if v["kind"] != kind || v["request_fingerprint"] != json!(self.fingerprint(&a)?) {
                bail!("request_id already belongs to a different operation");
            }
            return Ok(json!({"task_id":task_id,"status":v["status"],"duplicate":true}));
        }
        self.check_admission(kind).await?;
        let initial = json!({"task_id":task_id,"kind":kind,"arguments":redact_arguments(&a),"request_fingerprint":self.fingerprint(&a)?,"status":"accepted","started_at":now(),"result_path":path,"output_path":dir.join("output.log")});
        wire::atomic_json(&path, &initial)?;
        let cancel = CancellationToken::new();
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
            result["status"] = json!("running");
            let outcome = async {
                wire::atomic_json(&path, &result)?;
                engine.changed();
                if cancel.is_cancelled() {
                    bail!("cancelled before dispatch");
                }
                if kind == "exec.start" {
                    engine.command(&a, &dir, &cancel).await
                } else {
                    engine.backend_request(&a, "tools/call", &cancel).await
                }
            }
            .await;
            match outcome {
                Ok(v) => {
                    result["status"] = json!(if v["exit_code"].as_i64().is_some_and(|n| n != 0)
                        || v.pointer("/result/isError") == Some(&json!(true))
                    {
                        "failed"
                    } else {
                        "succeeded"
                    });
                    result["result"] = v;
                }
                Err(e) => {
                    let text = e.to_string();
                    result["status"] = json!(if kind == "mcp.call"
                        && !text.starts_with("stale_session")
                        && !text.starts_with("desktop_disabled")
                        && !text.starts_with("cancelled before")
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
                }
            }
            result["ended_at"] = json!(now());
            if wire::atomic_json(&path, &result).is_err() {
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
        let r = process::run_command(
            "/bin/sh",
            &["-c".into(), string(a, "command")?.into()],
            &expand(string(a, "cwd")?),
            &env,
            a["timeout_seconds"].as_u64().unwrap_or(3600),
            cancel,
            &tx,
            "output.log",
            &dir.join("process.json"),
        )
        .await;
        drop(tx);
        writer.await??;
        Ok(json!({"exit_code":r?}))
    }
    async fn backend_request(
        &self,
        a: &Value,
        method: &str,
        cancel: &CancellationToken,
    ) -> Result<Value> {
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
        // Waiting for the per-backend lock is cancellable, but never drops somebody else's client.
        let mut guard = tokio::select! {v=tokio::time::timeout(timeout,slot.lock())=>v.map_err(|_|anyhow::anyhow!("timed_out waiting for backend"))?,_=cancel.cancelled()=>bail!("cancelled before backend dispatch")};
        if let Some(expected) = a["session"].as_str() {
            if guard.as_ref().is_none_or(|c| c.generation != expected) {
                bail!("stale_session: rediscover tools and observe the target again");
            }
        } else if method == "tools/call" {
            bail!("stale_session: mcp.call requires session from mcp.tools");
        }
        if cancel.is_cancelled() {
            bail!("cancelled before backend dispatch");
        }
        if method == "tools/call" && !self.policy.lock().await.desktop_enabled {
            bail!("desktop_disabled before backend dispatch");
        }
        let operation = async {
            if guard.is_none() {
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
            let result = c.rpc(method, params).await?;
            Ok::<Value, anyhow::Error>(
                json!({"server":name,"session":c.generation,"result":result}),
            )
        };
        let r = tokio::select! {v=tokio::time::timeout(timeout,operation)=>v.unwrap_or_else(|_|Err(anyhow::anyhow!("timed_out; backend session invalidated; effect may be unknown"))),_=cancel.cancelled()=>Err(anyhow::anyhow!("cancelled; backend session invalidated; effect may be unknown"))};
        if r.is_err() {
            if let Some(c) = guard.as_mut() {
                c.stop().await;
            }
            *guard = None;
        }
        r
    }
}
