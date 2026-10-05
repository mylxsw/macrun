use crate::{
    config::Project,
    model::*,
    sync::{self, Entry},
    wire,
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::AsyncReadExt,
    net::{UnixListener, UnixStream},
    sync::{Mutex, broadcast},
};
#[derive(Clone)]
struct Peer {
    connection: quinn::Connection,
    instance: String,
    status: Value,
    seen: u64,
}
#[derive(Clone)]
struct Active {
    job_id: String,
    directory: PathBuf,
    reply: broadcast::Sender<Reply>,
}
#[derive(Default)]
struct State {
    peers: BTreeMap<String, Peer>,
    active: BTreeMap<String, Active>,
    last: Option<Value>,
}
type Shared = Arc<Mutex<State>>;
pub async fn serve(listen: std::net::SocketAddr, socket: PathBuf, data: PathBuf) -> Result<()> {
    let _lock = wire::lock(&data)?;
    let token = std::fs::read_to_string(data.join("token"))?
        .trim()
        .to_string();
    if token.is_empty() {
        anyhow::bail!("empty connection token");
    }
    let mut journals = vec![data.join("active.json")];
    if data.join("active").exists() {
        for entry in std::fs::read_dir(data.join("active"))? {
            journals.push(entry?.path());
        }
    }
    for journal in journals {
        if !journal.is_file() {
            continue;
        }
        let mut v: Value = serde_json::from_slice(&std::fs::read(&journal)?)?;
        if let Some(p) = v["directory"].as_str().map(PathBuf::from) {
            finish(
                &mut v,
                Some(Fault::new(
                    "server_restarted",
                    "server restarted during job",
                )),
            );
            wire::atomic_json(&p.join("result.json"), &v)?;
        }
        std::fs::remove_file(journal)?;
    }
    if socket.exists() {
        std::fs::remove_file(&socket)?;
    }
    if let Some(p) = socket.parent() {
        std::fs::create_dir_all(p)?;
    }
    let local = UnixListener::bind(&socket)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    let endpoint = wire::server(listen, &data)?;
    let state = Arc::new(Mutex::new(State::default()));
    crate::logging::event(
        "server",
        "listening",
        json!({"address":endpoint.local_addr()?.to_string(),"socket":socket}),
    );
    loop {
        tokio::select! {
            incoming=endpoint.accept()=>{if let Some(incoming)=incoming {let s=state.clone();let t=token.clone();let d=data.clone();tokio::spawn(async move {if register(incoming,s,t,d).await.is_err() {crate::logging::event("server", "connection_error", json!({"status":"failed"}));}});}},
            accepted=local.accept()=>{let (stream,_)=accepted?;let s=state.clone();let d=data.clone();tokio::spawn(async move {if handle_cli(stream,s,d).await.is_err(){crate::logging::event("server", "cli_error", json!({"status":"failed","reason":"request_decode_or_transport_failed"}));}});},
            _=tokio::signal::ctrl_c()=>break,
        }
    }
    endpoint.close(0u32.into(), b"server shutdown");
    Ok(())
}
async fn register(
    incoming: quinn::Incoming,
    state: Shared,
    token: String,
    data: PathBuf,
) -> Result<()> {
    let conn = tokio::time::timeout(Duration::from_secs(5), incoming).await??;
    let (mut send, mut recv) =
        tokio::time::timeout(Duration::from_secs(5), conn.accept_bi()).await??;
    let hello: Control =
        tokio::time::timeout(Duration::from_secs(5), wire::recv(&mut recv)).await??;
    if let Control::Pair { protocol, code } = &hello {
        let response = if *protocol != PROTOCOL {
            Control::Reject {
                error: Fault::new("protocol_mismatch", "update server and desktop"),
            }
        } else {
            match crate::pairing::redeem(&data, code) {
                Ok(token) => Control::Paired {
                    token,
                    protocol: PROTOCOL,
                },
                Err(e) => Control::Reject {
                    error: Fault::new("pairing_failed", e),
                },
            }
        };
        wire::send(&mut send, &response).await?;
        send.finish()?;
        let _ = tokio::time::timeout(Duration::from_secs(3), send.stopped()).await;
        return Ok(());
    }
    let Control::Hello {
        protocol,
        token: provided,
        instance,
        status,
    } = hello
    else {
        anyhow::bail!("expected hello");
    };
    if protocol != PROTOCOL || (provided != token && !crate::pairing::authorized(&data, &provided))
    {
        wire::send(
            &mut send,
            &Control::Reject {
                error: if protocol != PROTOCOL {
                    Fault::new("protocol_mismatch", "server and worker protocol differ")
                } else {
                    Fault::new("connection_auth_failed", "invalid worker credential")
                },
            },
        )
        .await?;
        send.finish()?;
        return Ok(());
    }
    let client_id = status["client_id"].as_str().unwrap_or(&instance).to_owned();
    anyhow::ensure!(
        uuid::Uuid::parse_str(&client_id).is_ok(),
        "invalid client identity"
    );
    {
        let mut s = state.lock().await;
        if s.peers.contains_key(&client_id) {
            wire::send(
                &mut send,
                &Control::Reject {
                    error: Fault::new("busy", "this client is already connected"),
                },
            )
            .await?;
            return Ok(());
        }
        wire::send(&mut send, &Control::Welcome { protocol: PROTOCOL }).await?;
        s.peers.insert(
            client_id.clone(),
            Peer {
                connection: conn.clone(),
                instance: instance.clone(),
                status,
                seen: now(),
            },
        );
    }
    crate::logging::event("server", "worker_connected", json!({"instance":instance}));
    let mut control_rx = wire::read_channel::<_, Control>(recv);
    let result:Result<()>=async {
        let mut heartbeat=tokio::time::interval(Duration::from_secs(5));
        loop {tokio::select! {
            message=control_rx.recv()=>{match message.ok_or_else(||anyhow::anyhow!("control stream closed"))?? {Control::Heartbeat{status}=>{
                let mut s=state.lock().await;
                if let Some(p)=s.peers.get_mut(&client_id){p.status=status;p.seen=now();}
            },_=>anyhow::bail!("unexpected control")}},
            _=heartbeat.tick()=>{if state.lock().await.peers.get(&client_id).is_none_or(|p|now()-p.seen>15000){anyhow::bail!("heartbeat timeout");}wire::send(&mut send,&Control::Heartbeat{status:json!({})}).await?;},
            _=conn.closed()=>break,
        }}Ok(())
    }.await;
    conn.close(1u32.into(), b"control disconnected");
    let mut s = state.lock().await;
    if s.peers
        .get(&client_id)
        .is_some_and(|p| p.instance == instance)
    {
        s.peers.remove(&client_id);
    }
    drop(s);
    crate::logging::event(
        "server",
        "worker_disconnected",
        json!({"instance":instance}),
    );
    result
}
async fn handle_cli(mut socket: UnixStream, state: Shared, data: PathBuf) -> Result<()> {
    let mut req: Request = wire::recv(&mut socket).await?;
    let target_value = req.args.as_object_mut().and_then(|a| a.remove("client_id"));
    if target_value.as_ref().is_some_and(|v| !v.is_string()) {
        wire::send(
            &mut socket,
            &Reply::Error {
                error: Fault::new("invalid_argument", "client_id must be a string"),
            },
        )
        .await?;
        return Ok(());
    }
    let target = target_value.and_then(|v| v.as_str().map(str::to_owned));
    let request_id = id();
    let mut log = crate::logging::Operation::new("server", &req.kind, &request_id);
    log.context(&req.args);
    if req.kind == "status" {
        let s = state.lock().await;
        let workers: Vec<Value> = s.peers.iter().map(|(id,p)| json!({"client_id":id,"instance":p.instance,"status":p.status,"sampled_at":p.seen,"active":s.active.get(id).map(|a|json!({"job_id":a.job_id,"directory":a.directory}))})).collect();
        let selected = target.as_ref().or_else(|| {
            if s.peers.len() == 1 {
                s.peers.keys().next()
            } else {
                None
            }
        });
        let peer = selected.and_then(|id| s.peers.get(id));
        let active = selected
            .and_then(|id| s.active.get(id))
            .map(|a| json!({"job_id":a.job_id,"directory":a.directory}));
        let value = json!({"connected":!s.peers.is_empty(),"workers":workers,"worker":peer.map(|p|&p.status),"sampled_at":peer.map(|p|p.seen),"active":active,"last":s.last});
        log.send(&mut socket, &Reply::Status { value }).await?;
        return Ok(());
    }
    if req.kind == "sync.get" {
        let ident = req.args["job_id"].as_str().unwrap_or("");
        if uuid::Uuid::parse_str(ident).is_err() {
            log.send(
                &mut socket,
                &Reply::Error {
                    error: Fault::new("invalid_argument", "job_id must be a UUID returned by sync"),
                },
            )
            .await?;
            return Ok(());
        }
        let path = data.join("sync-jobs").join(ident).join("result.json");
        let reply = match std::fs::read(path)
            .and_then(|b| serde_json::from_slice::<Value>(&b).map_err(std::io::Error::other))
        {
            Ok(value) => Reply::Status { value },
            Err(e) => Reply::Error {
                error: Fault::new("not_found", e),
            },
        };
        log.send(&mut socket, &reply).await?;
        return Ok(());
    }
    let client_id = {
        let s = state.lock().await;
        match select_client(&s, target.as_deref()) {
            Ok(id) => id,
            Err(error) => {
                drop(s);
                log.send(&mut socket, &Reply::Error { error }).await?;
                return Ok(());
            }
        }
    };
    if req.kind != "sync" {
        let peer = state.lock().await.peers.get(&client_id).cloned();
        let reply = if let Some(peer) = peer {
            let task_request_id = req.args.get("request_id").cloned();
            let operation = async {
                let (mut send, mut recv) = peer.connection.open_bi().await?;
                wire::send(
                    &mut send,
                    &Task {
                        job_id: request_id.clone(),
                        request: req,
                        project: Project::default(),
                        manifest: None,
                    },
                )
                .await?;
                send.finish()?;
                wire::recv::<_, Reply>(&mut recv).await
            };
            match tokio::time::timeout(Duration::from_secs(35), operation).await {
                Ok(Ok(reply)) => reply,
                other => Reply::Error {
                    error: Fault::new(
                        "outcome_unknown",
                        format!(
                            "Connection/request interrupted: {other:?}. Query task by request_id {task_request_id:?}; do not repeat an action with a new ID."
                        ),
                    ),
                },
            }
        } else {
            Reply::Error {
                error: Fault::new("worker_offline", "worker is not connected"),
            }
        };
        log.send(&mut socket, &reply).await?;
        return Ok(());
    }
    let project = match Project::load(&req.workspace) {
        Ok(p) => p,
        Err(error) => {
            log.send(&mut socket, &Reply::Error { error }).await?;
            return Ok(());
        }
    };
    let mut s = state.lock().await;
    if let Some(a) = s.active.get(&client_id) {
        log.send(
            &mut socket,
            &Reply::Error {
                error: Fault::new("busy", format!("current job: {}", a.job_id)),
            },
        )
        .await?;
        return Ok(());
    }
    let Some(peer) = s.peers.get(&client_id).cloned() else {
        log.send(
            &mut socket,
            &Reply::Error {
                error: Fault::new("worker_offline", "worker is not connected"),
            },
        )
        .await?;
        return Ok(());
    };
    if peer.status["ready"] != true {
        log.send(
            &mut socket,
            &Reply::Error {
                error: Fault::new("recovery_required", peer.status.to_string()),
            },
        )
        .await?;
        return Ok(());
    }
    let job_id = id();
    let directory = data.join("sync-jobs").join(&job_id);
    std::fs::create_dir_all(&directory)?;
    let (reply, receiver) = broadcast::channel(512);
    let active = Active {
        job_id: job_id.clone(),
        directory: directory.clone(),
        reply,
    };
    let detached = req.args["detach"] == true;
    let task = Task {
        job_id,
        request: req,
        project,
        manifest: None,
    };
    let mut initial = initial_result(&task);
    initial["directory"] = json!(directory);
    initial["client_id"] = json!(client_id);
    wire::atomic_json(&directory.join("result.json"), &initial)?;
    wire::atomic_json(
        &data.join("active").join(format!("{client_id}.json")),
        &initial,
    )?;
    s.active.insert(client_id.clone(), active.clone());
    drop(s);
    let background = active.clone();
    tokio::spawn(async move {
        execute(task, peer, client_id, state, data, background, initial).await;
    });
    if detached {
        log.send(
            &mut socket,
            &Reply::Accepted {
                job_id: active.job_id,
                directory: active.directory,
            },
        )
        .await?;
        Ok(())
    } else {
        log.reply(&Reply::Accepted {
            job_id: active.job_id.clone(),
            directory: active.directory.clone(),
        });
        drop(log);
        follow(socket, active, receiver).await
    }
}
async fn follow(
    mut socket: UnixStream,
    active: Active,
    mut receiver: broadcast::Receiver<Reply>,
) -> Result<()> {
    wire::send(
        &mut socket,
        &Reply::Accepted {
            job_id: active.job_id,
            directory: active.directory,
        },
    )
    .await?;
    loop {
        match receiver.recv().await {
            Ok(reply) => {
                let done = matches!(reply, Reply::Done { .. });
                wire::send(&mut socket, &reply).await?;
                if done {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                wire::send(
                    &mut socket,
                    &Reply::Log {
                        name: "run.log".into(),
                        text: "CLI fell behind; complete logs are in the task directory\n".into(),
                    },
                )
                .await?;
            }
            Err(_) => break,
        }
    }
    Ok(())
}
async fn execute(
    mut task: Task,
    peer: Peer,
    client_id: String,
    state: Shared,
    data: PathBuf,
    active: Active,
    mut result: Value,
) {
    let r = execute_inner(&mut task, &peer, &active, &mut result).await;
    let error = r.err().map(|e| {
        e.downcast_ref::<Fault>()
            .cloned()
            .unwrap_or_else(|| Fault::new("worker_disconnected", e))
    });
    finish(&mut result, error);
    crate::logging::event(
        "server",
        "task_finished",
        json!({"operation":"sync", "job_id":task.job_id, "status":result["status"], "duration_ms":now().saturating_sub(result["started_at"].as_u64().unwrap_or(now())), "error_code":result.pointer("/error/code")}),
    );
    if wire::atomic_json(&active.directory.join("result.json"), &result).is_err() {
        crate::logging::event("server", "persist_failed", json!({"job_id":task.job_id}));
    }
    let _ = std::fs::remove_file(data.join("active").join(format!("{client_id}.json")));
    {
        let mut s = state.lock().await;
        s.last = Some(result.clone());
        s.active.remove(&client_id);
        if let Some(p) = s.peers.get_mut(&client_id)
            && p.instance == peer.instance
        {
            p.status["ready"] = json!(
                result.pointer("/error/code").and_then(Value::as_str) != Some("recovery_required")
            );
            p.status["job_id"] = Value::Null;
        }
    }
    let _ = active.reply.send(Reply::Done {
        result,
        directory: active.directory,
    });
}
async fn execute_inner(
    task: &mut Task,
    peer: &Peer,
    active: &Active,
    result: &mut Value,
) -> Result<()> {
    let sync_needed = task.request.kind == "sync";
    if sync_needed {
        let root = task.request.workspace.clone();
        let exclusions = task.project.exclude.clone();
        let before = now();
        task.manifest =
            Some(tokio::task::spawn_blocking(move || sync::scan(&root, &exclusions)).await??);
        result["phases_ms"]["scan"] = json!(now() - before);
    }
    let (mut send, mut recv) = peer.connection.open_bi().await?;
    wire::send(&mut send, task).await?;
    let mut phase = "dispatch".to_string();
    let mut phase_start = now();
    loop {
        let event: Event = wire::recv(&mut recv).await?;
        match event {
            Event::Need { paths } => {
                let transfer = send_files(task, &peer.connection, &paths, &mut send);
                if let Err(e) = transfer.await {
                    peer.connection.close(2u32.into(), b"sync source failed");
                    return Err(e);
                }
            }
            Event::Progress { phase: new } => {
                result["phases_ms"][&phase] = json!(now() - phase_start);
                phase = new;
                phase_start = now();
                result["status"] = json!(phase);
                wire::atomic_json(&active.directory.join("result.json"), result)?;
            }
            Event::Detail { value } => {
                if let Some(m) = value.as_object() {
                    for (k, v) in m {
                        result[k] = v.clone();
                    }
                }
            }
            Event::Done { error } => {
                result["phases_ms"][&phase] = json!(now() - phase_start);
                return error.map_or(Ok(()), |e| Err(e.into()));
            }
            _ => anyhow::bail!("unexpected task message"),
        }
    }
}
async fn send_files(
    task: &Task,
    conn: &quinn::Connection,
    paths: &[String],
    send: &mut quinn::SendStream,
) -> Result<()> {
    let manifest = task
        .manifest
        .as_ref()
        .ok_or_else(|| Fault::new("sync_failed", "missing manifest"))?;
    for path in paths {
        let Some(Entry::File { size, .. }) = manifest.entries.get(path) else {
            return Err(Fault::new("sync_failed", "worker requested unknown file").into());
        };
        let mut stream = conn.open_uni().await?;
        wire::send(
            &mut stream,
            &FileHeader {
                job_id: task.job_id.clone(),
                path: path.clone(),
                size: *size,
            },
        )
        .await?;
        let file =
            tokio::fs::File::open(task.request.workspace.join(sync::relative(path)?)).await?;
        let copied = tokio::io::copy(&mut file.take(*size), &mut stream).await?;
        if copied != *size {
            return Err(Fault::new("source_changed", path).into());
        }
        stream.finish()?;
    }
    let root = task.request.workspace.clone();
    let exclude = task.project.exclude.clone();
    let confirmed = tokio::task::spawn_blocking(move || sync::scan(&root, &exclude)).await??;
    wire::send(
        send,
        &Event::SyncCommit {
            manifest: confirmed,
        },
    )
    .await?;
    Ok(())
}

fn select_client(state: &State, target: Option<&str>) -> Outcome<String> {
    if let Some(target) = target {
        return state
            .peers
            .contains_key(target)
            .then(|| target.to_owned())
            .ok_or_else(|| Fault::new("worker_offline", "selected client is not connected"));
    }
    match state.peers.len() {
        0 => Err(Fault::new("worker_offline", "worker is not connected")),
        1 => Ok(state.peers.keys().next().unwrap().clone()),
        _ => Err(Fault::new(
            "client_required",
            "multiple clients connected; select client_id from status",
        )),
    }
}
