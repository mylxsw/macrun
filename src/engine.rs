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
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;
type Slot = Arc<Mutex<Option<Client>>>;
pub struct Engine {
    data: PathBuf,
    config: WorkerConfig,
    submissions: Mutex<()>,
    tasks: Mutex<BTreeMap<String, CancellationToken>>,
    backends: BTreeMap<String, Slot>,
}
impl Engine {
    pub fn open(data: PathBuf, config: WorkerConfig) -> Result<Arc<Self>> {
        std::fs::create_dir_all(data.join("tasks"))?;
        for entry in std::fs::read_dir(data.join("tasks"))? {
            let dir = entry?.path();
            let p = dir.join("result.json");
            if !p.exists() {
                continue;
            }
            let mut v: Value = serde_json::from_slice(&std::fs::read(&p)?)?;
            if ["accepted", "running"].contains(&v["status"].as_str().unwrap_or("")) {
                v["status"] = json!("unknown");
                v["ended_at"] = json!(now());
                v["error"] = json!({"code":"worker_restarted","message":"Execution interrupted; effects may have occurred. Never automatically replay."});
                let journal = dir.join("process.json");
                if journal.exists() {
                    v["recovery"] = json!(process::recover(&journal).err());
                }
                wire::atomic_json(&p, &v)?;
            }
        }
        let backends = config
            .mcp
            .keys()
            .map(|n| (n.clone(), Arc::new(Mutex::new(None))))
            .collect();
        Ok(Arc::new(Self {
            data,
            config,
            submissions: Mutex::new(()),
            tasks: Mutex::new(BTreeMap::new()),
            backends,
        }))
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
            _ if kind.starts_with("file.") => crate::files::handle(kind, &a).await,
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
            if v["kind"] != kind || v["arguments"] != a {
                bail!("request_id already belongs to a different operation");
            }
            return Ok(json!({"task_id":task_id,"status":v["status"],"duplicate":true}));
        }
        let initial = json!({"task_id":task_id,"kind":kind,"arguments":a,"status":"accepted","started_at":now(),"result_path":path,"output_path":dir.join("output.log")});
        wire::atomic_json(&path, &initial)?;
        let cancel = CancellationToken::new();
        self.tasks
            .lock()
            .await
            .insert(task_id.clone(), cancel.clone());
        let engine = self.clone();
        let ident = task_id.clone();
        let kind = kind.to_owned();
        tokio::spawn(async move {
            let mut result = initial;
            result["status"] = json!("running");
            let outcome = async {
                wire::atomic_json(&path, &result)?;
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
            if let Err(e) = wire::atomic_json(&path, &result) {
                eprintln!("persist task result: {e}");
            }
            engine.tasks.lock().await.remove(&ident);
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
