//! Private, same-user worker control. No control mutations are exposed over QUIC.
use crate::{engine::Engine, model::now, wire};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{UnixListener, UnixStream},
    sync::watch,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionState {
    pub state: String,
    pub server: String,
    pub since: u64,
    pub rtt_ms: Option<u64>,
    pub error: Option<String>,
    pub checks: Value,
}
impl ConnectionState {
    pub fn initial(server: String) -> Self {
        Self {
            state: "connecting".into(),
            server,
            since: now(),
            rtt_ms: None,
            error: None,
            checks: json!({"transport":false,"certificate":false,"authentication":false,"protocol":false}),
        }
    }
}
pub async fn request(socket: &Path, action: &str, args: Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut stream = UnixStream::connect(socket).await?;
        wire::send(&mut stream, &json!({"action":action,"args":args})).await?;
        let v: Value = wire::recv(&mut stream).await?;
        // Control failures use an error string; task records have a structured
        // error describing their completed execution, which is valid detail data.
        if let Some(error) = v.get("error").and_then(Value::as_str) {
            bail!("{error}");
        }
        Ok(v)
    })
    .await?
}
#[derive(Clone)]
pub struct LocalConnection {
    pub id: String,
    pub name: String,
    pub engine: Arc<Engine>,
    pub connection: watch::Receiver<ConnectionState>,
}
pub async fn serve(
    socket: PathBuf,
    engine: Arc<Engine>,
    connection: watch::Receiver<ConnectionState>,
    shutdown: CancellationToken,
) -> Result<()> {
    let name = connection.borrow().server.clone();
    serve_connections(
        socket,
        vec![LocalConnection {
            id: "primary".into(),
            name,
            engine,
            connection: connection.clone(),
        }],
        shutdown,
    )
    .await
}
pub async fn serve_connections(
    socket: PathBuf,
    connections: Vec<LocalConnection>,
    shutdown: CancellationToken,
) -> Result<()> {
    anyhow::ensure!(!connections.is_empty(), "no connections");
    let connections = Arc::new(connections);
    let parent = socket
        .parent()
        .ok_or_else(|| anyhow::anyhow!("socket needs parent directory"))?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let meta = std::fs::symlink_metadata(parent)?;
    if meta.file_type().is_symlink() {
        bail!("control directory cannot be a symlink");
    }
    if meta.uid() != unsafe { libc::geteuid() } || meta.permissions().mode() & 0o077 != 0 {
        bail!("control socket requires a dedicated, user-owned 0700 directory");
    }
    let _socket_lock = wire::lock(parent)?;
    if let Ok(meta) = std::fs::symlink_metadata(&socket) {
        if !meta.file_type().is_socket() {
            bail!("control path exists and is not a socket");
        }
        if UnixStream::connect(&socket).await.is_ok() {
            bail!("control socket is already in use");
        }
        std::fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    loop {
        tokio::select! {
            _=shutdown.cancelled()=>break,
            accepted=listener.accept()=>{
                let (stream,_) = accepted?;
                if stream.peer_cred()?.uid() != unsafe { libc::geteuid() } { continue; }
                let c=connections.clone(); let s=shutdown.clone();
                tokio::spawn(async move { let _=handle(stream,c,s).await; });
            }
        }
    }
    let _ = std::fs::remove_file(socket);
    Ok(())
}
async fn snapshot(connections: &[LocalConnection]) -> Result<Value> {
    let mut all = Vec::new();
    for c in connections {
        let mut v = c.engine.local_snapshot().await?;
        v["connection"] = json!(c.connection.borrow().clone());
        all.push(v);
    }
    let mut value = all[0].clone();
    value["connections"]=json!(connections.iter().zip(&all).map(|(c,v)|json!({"id":c.id,"name":c.name,"connection":v["connection"],"policy":v["policy"]})).collect::<Vec<_>>());
    if connections.len() == 1 {
        return Ok(value);
    }
    for field in ["tasks", "backends", "workspaces", "allow_rules"] {
        let mut rows = Vec::new();
        for (c, v) in connections.iter().zip(&all) {
            for row in v[field].as_array().into_iter().flatten() {
                let mut row = row.clone();
                row["connection_id"] = json!(c.id);
                row["connection_name"] = json!(c.name);
                if field == "backends" {
                    row["backend_name"] = row["name"].clone();
                    row["display_name"] = json!(format!(
                        "{} · {}",
                        row["name"].as_str().unwrap_or(""),
                        c.name
                    ));
                    row["name"] =
                        json!(format!("{}::{}", c.id, row["name"].as_str().unwrap_or("")));
                }
                if field == "allow_rules" {
                    row["id"] = json!(format!("{}::{}", c.id, row["id"].as_str().unwrap_or("")));
                }
                rows.push(row);
            }
        }
        if field == "tasks" {
            sort_tasks(&mut rows);
            rows = rows
                .into_iter()
                .enumerate()
                .filter(|(i, row)| *i < 200 || crate::history::active(row))
                .map(|(_, row)| row)
                .collect();
        }
        value[field] = json!(rows);
    }
    for field in ["total_tasks", "active_count"] {
        value[field] = json!(
            all.iter()
                .map(|v| v[field].as_u64().unwrap_or(0))
                .sum::<u64>()
        );
    }
    let mut summary = serde_json::Map::new();
    for v in &all {
        if let Some(counts) = v["today_summary"].as_object() {
            for (k, n) in counts {
                let count =
                    summary.get(k).and_then(Value::as_u64).unwrap_or(0) + n.as_u64().unwrap_or(0);
                summary.insert(k.clone(), json!(count));
            }
        }
    }
    value["today_summary"] = json!(summary);
    value["policy"]["paused"] = json!(all.iter().all(|v| v["policy"]["paused"] == true));
    value["policy"]["desktop_enabled"] =
        json!(all.iter().all(|v| v["policy"]["desktop_enabled"] == true));
    let connected = all
        .iter()
        .filter(|v| v["connection"]["state"] == "connected")
        .count();
    value["connection"] = all
        .iter()
        .find(|v| v["connection"]["state"] == "connected")
        .unwrap_or(&all[0])["connection"]
        .clone();
    value["connection"]["server"] = json!(format!("{connected}/{} 台服务器在线", all.len()));
    Ok(value)
}
fn sort_tasks(rows: &mut [Value]) {
    rows.sort_by(|a, b| {
        (
            b["started_at"].as_u64().unwrap_or(0),
            b["task_id"].as_str().unwrap_or(""),
        )
            .cmp(&(
                a["started_at"].as_u64().unwrap_or(0),
                a["task_id"].as_str().unwrap_or(""),
            ))
    });
}
async fn task_page(connections: &[LocalConnection], mut args: Value) -> Result<Value> {
    let filter = args
        .as_object_mut()
        .and_then(|v| v.remove("connection_id"))
        .and_then(|v| v.as_str().map(str::to_owned));
    let limit = args["limit"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
    let mut rows = Vec::new();
    let mut total = 0;
    let mut filtered = 0;
    let mut more = false;
    let mut counts = serde_json::Map::new();
    let mut matched = false;
    for c in connections {
        if filter.as_ref().is_some_and(|id| id != &c.id) {
            continue;
        }
        matched = true;
        let page = c.engine.local_task_list(args.clone()).await?;
        total += page["total"].as_u64().unwrap_or(0);
        filtered += page["filtered_total"].as_u64().unwrap_or(0);
        more |= !page["next_cursor"].is_null();
        if let Some(values) = page["counts"].as_object() {
            for (k, v) in values {
                let n =
                    counts.get(k).and_then(Value::as_u64).unwrap_or(0) + v.as_u64().unwrap_or(0);
                counts.insert(k.clone(), json!(n));
            }
        }
        for row in page["tasks"].as_array().into_iter().flatten() {
            let mut row = row.clone();
            row["connection_id"] = json!(c.id);
            row["connection_name"] = json!(c.name);
            rows.push(row);
        }
    }
    anyhow::ensure!(matched, "unknown connection");
    sort_tasks(&mut rows);
    more |= rows.len() > limit;
    rows.truncate(limit);
    let cursor = if more {
        rows.last()
            .map(|t| json!({"started_at":t["started_at"],"task_id":t["task_id"]}))
    } else {
        None
    };
    Ok(
        json!({"tasks":rows,"total":total,"filtered_total":filtered,"counts":counts,"next_cursor":cursor}),
    )
}
async fn handle(
    mut stream: UnixStream,
    connections: Arc<Vec<LocalConnection>>,
    shutdown: CancellationToken,
) -> Result<()> {
    let req: Value =
        tokio::time::timeout(Duration::from_secs(5), wire::recv(&mut stream)).await??;

    if req["action"] == "subscribe" {
        let mut changes = connections[0].engine.subscribe();
        let mut connection = connections[0].connection.clone();
        let mut tick = tokio::time::interval(Duration::from_millis(750));
        loop {
            tokio::select! {
                _=shutdown.cancelled()=>break,
                _=tick.tick()=>{},
                r=changes.changed()=>{r?;},
                r=connection.changed()=>{r?;},
            }
            let v = snapshot(&connections).await?;
            tokio::time::timeout(Duration::from_secs(3), wire::send(&mut stream, &v)).await??;
        }
        return Ok(());
    }
    let action = req["action"].as_str().unwrap_or("");
    let r = dispatch(&connections, action, req["args"].clone(), &shutdown).await;
    let reply = r.unwrap_or_else(|e| json!({"error":e.to_string()}));
    wire::send(&mut stream, &reply).await?;
    Ok(())
}

async fn dispatch(
    connections: &[LocalConnection],
    action: &str,
    mut args: Value,
    shutdown: &CancellationToken,
) -> Result<Value> {
    if action == "snapshot" {
        return snapshot(connections).await;
    }
    if action == "task_list" {
        return task_page(connections, args).await;
    }
    let mut selected = args
        .as_object_mut()
        .and_then(|v| v.remove("connection_id"))
        .and_then(|v| v.as_str().map(str::to_owned));
    let field = if action == "revoke_rule" {
        "rule_id"
    } else {
        "server"
    };
    if let Some(value) = args[field].as_str()
        && let Some((id, name)) = value.split_once("::")
        && connections.iter().any(|c| c.id == id)
    {
        selected = Some(id.to_owned());
        args[field] = json!(name);
    }
    if selected.is_none()
        && let Some(task) = args["task_id"].as_str()
    {
        selected = connections
            .iter()
            .find(|c| c.engine.owns_task(task))
            .map(|c| c.id.clone());
        anyhow::ensure!(selected.is_some(), "task not found");
    }
    let global = matches!(
        action,
        "pause" | "desktop" | "stop_all" | "safety" | "yield" | "shutdown" | "prune"
    );
    let targets: Vec<_> = connections
        .iter()
        .filter(|c| {
            selected
                .as_ref()
                .map_or(global || c.id == connections[0].id, |id| id == &c.id)
        })
        .collect();
    anyhow::ensure!(!targets.is_empty(), "unknown connection");
    let mut result = json!({});
    let mut errors = Vec::new();
    for c in targets {
        match dispatch_one(&c.engine, action, &args, shutdown).await {
            Ok(value) => {
                result = value;
                result["connection_id"] = json!(c.id);
                result["connection_name"] = json!(c.name);
            }
            Err(e) => errors.push(format!("{}: {e}", c.name)),
        }
    }
    anyhow::ensure!(errors.is_empty(), "{}", errors.join("; "));
    Ok(result)
}
async fn dispatch_one(
    engine: &Arc<Engine>,
    action: &str,
    a: &Value,
    shutdown: &CancellationToken,
) -> Result<Value> {
    match action {
        "self_test" => engine.self_test().await,
        "prune" => engine.prune_history().await,
        "observe" => engine.observe_locally(a.clone()).await,
        "safety" => engine.set_safety(serde_json::from_value(a.clone())?).await,
        "approve" => {
            engine
                .approve_scoped(
                    crate::files::string(a, "task_id")?,
                    a["allow"]
                        .as_bool()
                        .ok_or_else(|| anyhow::anyhow!("allow required"))?,
                    a["scope"].as_str().unwrap_or("once"),
                )
                .await
        }
        "revoke_rule" => {
            engine
                .revoke_rule(crate::files::string(a, "rule_id")?)
                .await
        }
        "yield" => engine.yield_desktop().await,
        "restart_backend" => {
            engine
                .restart_backend(crate::files::string(a, "server")?)
                .await
        }
        "task_detail" => engine.handle("task.get", a.clone()).await,
        "pause" => {
            engine
                .set_policy(
                    Some(
                        a["paused"]
                            .as_bool()
                            .ok_or_else(|| anyhow::anyhow!("paused must be boolean"))?,
                    ),
                    None,
                    false,
                )
                .await
        }
        "desktop" => {
            engine
                .set_policy(
                    None,
                    Some(
                        a["enabled"]
                            .as_bool()
                            .ok_or_else(|| anyhow::anyhow!("enabled must be boolean"))?,
                    ),
                    false,
                )
                .await
        }
        "stop_all" => engine.set_policy(None, None, true).await,
        "cancel" => engine.handle("task.cancel", a.clone()).await,
        "tools" => engine.handle("mcp.tools", a.clone()).await,
        "shutdown" => {
            engine.shutdown().await?;
            shutdown.cancel();
            Ok(json!({"stopped":true}))
        }
        _ => bail!("unsupported local action"),
    }
}

#[cfg(test)]
mod multi_tests {
    use super::*;
    #[tokio::test]
    async fn merged_history_paginates_equal_timestamps_without_gaps_and_filters_sources() {
        let root = tempfile::tempdir().unwrap();
        let mut connections = Vec::new();
        for source in ["primary", "secondary"] {
            let data = root.path().join(source);
            let engine =
                Engine::open(data.clone(), crate::config::WorkerConfig::default()).unwrap();
            for _ in 0..65 {
                let id = crate::model::id();
                crate::wire::atomic_json(&data.join("tasks").join(&id).join("result.json"),&json!({"task_id":id,"kind":"exec.start","status":"succeeded","started_at":100,"ended_at":101,"arguments":{"command":format!("echo {source}")}})).unwrap();
            }
            let (_, rx) = watch::channel(ConnectionState::initial(source.into()));
            connections.push(LocalConnection {
                id: source.into(),
                name: source.into(),
                engine,
                connection: rx,
            });
        }
        let mut cursor = Value::Null;
        let mut ids = std::collections::BTreeSet::new();
        loop {
            let page = task_page(&connections, json!({"limit":50,"cursor":cursor}))
                .await
                .unwrap();
            assert_eq!(page["total"], 130);
            for row in page["tasks"].as_array().unwrap() {
                assert!(ids.insert(row["task_id"].as_str().unwrap().to_owned()));
            }
            cursor = page["next_cursor"].clone();
            if cursor.is_null() {
                break;
            }
        }
        assert_eq!(ids.len(), 130);
        let selected = task_page(
            &connections,
            json!({"limit":100,"connection_id":"secondary"}),
        )
        .await
        .unwrap();
        assert_eq!(selected["total"], 65);
        assert!(
            selected["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["connection_id"] == "secondary")
        );
        assert!(
            task_page(&connections, json!({"connection_id":"missing"}))
                .await
                .is_err()
        );
    }
}
