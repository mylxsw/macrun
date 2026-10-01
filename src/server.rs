use crate::{
    config::Project,
    model::*,
    sync::{self, Entry},
    wire,
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Mutex, broadcast, mpsc},
};
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
struct Peer {
    connection: quinn::Connection,
    control: mpsc::Sender<Control>,
    instance: String,
    status: Value,
    seen: u64,
}
#[derive(Clone)]
struct Active {
    job_id: String,
    directory: PathBuf,
    reply: broadcast::Sender<Reply>,
    cancel: CancellationToken,
}
#[derive(Default)]
struct State {
    peer: Option<Peer>,
    active: Option<Active>,
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
    if let Ok(v) = std::fs::read(data.join("active.json")) {
        let mut v: Value = serde_json::from_slice(&v)?;
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
        std::fs::remove_file(data.join("active.json"))?;
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
    eprintln!(
        "macrun-server listening on {}, CLI {}",
        endpoint.local_addr()?,
        socket.display()
    );
    loop {
        tokio::select! {
            incoming=endpoint.accept()=>{if let Some(incoming)=incoming {let s=state.clone();let t=token.clone();tokio::spawn(async move {if let Err(e)=register(incoming,s,t).await {eprintln!("worker connection: {e}");}});}},
            accepted=local.accept()=>{let (stream,_)=accepted?;let s=state.clone();let d=data.clone();tokio::spawn(async move {if let Err(e)=handle_cli(stream,s,d).await{eprintln!("CLI: {e}");}});},
            _=tokio::signal::ctrl_c()=>break,
        }
    }
    endpoint.close(0u32.into(), b"server shutdown");
    Ok(())
}
async fn register(incoming: quinn::Incoming, state: Shared, token: String) -> Result<()> {
    let conn = tokio::time::timeout(Duration::from_secs(5), incoming).await??;
    let (mut send, mut recv) =
        tokio::time::timeout(Duration::from_secs(5), conn.accept_bi()).await??;
    let hello: Control =
        tokio::time::timeout(Duration::from_secs(5), wire::recv(&mut recv)).await??;
    let Control::Hello {
        protocol,
        token: provided,
        instance,
        status,
    } = hello
    else {
        anyhow::bail!("expected hello");
    };
    if protocol != PROTOCOL || provided != token {
        wire::send(
            &mut send,
            &Control::Reject {
                error: Fault::new("connection_auth_failed", "protocol or token mismatch"),
            },
        )
        .await?;
        send.finish()?;
        return Ok(());
    }
    let (tx, mut rx) = mpsc::channel(32);
    {
        let mut s = state.lock().await;
        if s.peer.is_some() {
            wire::send(
                &mut send,
                &Control::Reject {
                    error: Fault::new("busy", "worker already connected"),
                },
            )
            .await?;
            return Ok(());
        }
        wire::send(&mut send, &Control::Welcome { protocol: PROTOCOL }).await?;
        s.peer = Some(Peer {
            connection: conn.clone(),
            control: tx,
            instance: instance.clone(),
            status,
            seen: now(),
        });
    }
    let mut control_rx = wire::read_channel::<_, Control>(recv);
    let result:Result<()>=async {
        let mut heartbeat=tokio::time::interval(Duration::from_secs(5));
        loop {tokio::select! {
            message=control_rx.recv()=>{match message.ok_or_else(||anyhow::anyhow!("control stream closed"))?? {Control::Heartbeat{mut status}=>{
                let mut s=state.lock().await;
                // A heartbeat queued before Done must not make a completed worker look busy again.
                if s.active.is_none() && s.last.as_ref().is_some_and(|last|status["job_id"]==last["job_id"]) {
                    status["ready"]=json!(s.last.as_ref().unwrap().pointer("/error/code").and_then(Value::as_str)!=Some("recovery_required"));
                    status["job_id"]=Value::Null;
                }
                if let Some(p)=s.peer.as_mut(){p.status=status;p.seen=now();}
            },_=>anyhow::bail!("unexpected control")}},
            Some(message)=rx.recv()=>wire::send(&mut send,&message).await?,
            _=heartbeat.tick()=>{if state.lock().await.peer.as_ref().is_none_or(|p|now()-p.seen>15000){anyhow::bail!("heartbeat timeout");}wire::send(&mut send,&Control::Heartbeat{status:json!({})}).await?;},
            _=conn.closed()=>break,
        }}Ok(())
    }.await;
    conn.close(1u32.into(), b"control disconnected");
    let mut s = state.lock().await;
    if s.peer.as_ref().is_some_and(|p| p.instance == instance) {
        s.peer = None;
    }
    drop(s);
    result
}
async fn handle_cli(mut socket: UnixStream, state: Shared, data: PathBuf) -> Result<()> {
    let req: Request = wire::recv(&mut socket).await?;
    if req.kind == "status" {
        let s = state.lock().await;
        let value = json!({"connected":s.peer.is_some(),"worker":s.peer.as_ref().map(|p|&p.status),"sampled_at":s.peer.as_ref().map(|p|p.seen),"active":s.active.as_ref().map(|a|json!({"job_id":a.job_id,"directory":a.directory})),"last":s.last});
        wire::send(&mut socket, &Reply::Status { value }).await?;
        return Ok(());
    }
    if req.kind == "cancel" {
        let s = state.lock().await;
        let Some(active) = s.active.clone().filter(|a| req.args["job_id"] == a.job_id) else {
            wire::send(
                &mut socket,
                &Reply::Error {
                    error: Fault::new("invalid_config", "job is not active"),
                },
            )
            .await?;
            return Ok(());
        };
        let receiver = active.reply.subscribe();
        active.cancel.cancel();
        if let Some(peer) = &s.peer {
            let _ = peer
                .control
                .send(Control::Cancel {
                    job_id: active.job_id.clone(),
                })
                .await;
        }
        drop(s);
        return follow(socket, active, receiver).await;
    }
    if ![
        "sync", "build", "test", "run", "shot", "stop", "snapshot", "click", "type", "key",
        "scroll",
    ]
    .contains(&req.kind.as_str())
    {
        wire::send(
            &mut socket,
            &Reply::Error {
                error: Fault::new("invalid_config", "unknown command"),
            },
        )
        .await?;
        return Ok(());
    }
    let project = match Project::load(&req.workspace) {
        Ok(p) => p,
        Err(error) => {
            wire::send(&mut socket, &Reply::Error { error }).await?;
            return Ok(());
        }
    };
    let mut s = state.lock().await;
    if let Some(a) = &s.active {
        wire::send(
            &mut socket,
            &Reply::Error {
                error: Fault::new("busy", format!("current job: {}", a.job_id)),
            },
        )
        .await?;
        return Ok(());
    }
    let Some(peer) = s.peer.clone() else {
        wire::send(
            &mut socket,
            &Reply::Error {
                error: Fault::new("worker_offline", "worker is not connected"),
            },
        )
        .await?;
        return Ok(());
    };
    if peer.status["ready"] != true {
        wire::send(
            &mut socket,
            &Reply::Error {
                error: Fault::new("recovery_required", peer.status.to_string()),
            },
        )
        .await?;
        return Ok(());
    }
    let job_id = id();
    let directory = req.workspace.join(".macrun/artifacts").join(&job_id);
    std::fs::create_dir_all(&directory)?;
    let (reply, receiver) = broadcast::channel(512);
    let active = Active {
        job_id: job_id.clone(),
        directory: directory.clone(),
        reply,
        cancel: CancellationToken::new(),
    };
    let task = Task {
        job_id,
        request: req,
        project,
        manifest: None,
    };
    let mut initial = initial_result(&task);
    initial["directory"] = json!(directory);
    wire::atomic_json(&directory.join("result.json"), &initial)?;
    wire::atomic_json(&data.join("active.json"), &initial)?;
    s.active = Some(active.clone());
    drop(s);
    let background = active.clone();
    tokio::spawn(async move {
        execute(task, peer, state, data, background, initial).await;
    });
    follow(socket, active, receiver).await
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
    if error
        .as_ref()
        .is_some_and(|e| e.code == "worker_disconnected")
        && ["click", "type", "key", "scroll"].contains(&task.request.kind.as_str())
    {
        result["action_status"] = json!("unknown");
    }
    finish(&mut result, error);
    if let Err(e) = wire::atomic_json(&active.directory.join("result.json"), &result) {
        eprintln!("persist result: {e}");
    }
    let _ = std::fs::remove_file(data.join("active.json"));
    {
        let mut s = state.lock().await;
        s.last = Some(result.clone());
        s.active = None;
        if let Some(p) = s.peer.as_mut()
            && p.instance == peer.instance
        {
            p.status["ready"] = json!(
                result.pointer("/error/code").and_then(Value::as_str) != Some("recovery_required")
            );
            if let Some(run) = result.get("run") {
                p.status["run"] = run.clone();
            }
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
    let sync_needed = ["sync", "build", "test", "run"].contains(&task.request.kind.as_str());
    if sync_needed {
        let root = task.request.workspace.clone();
        let exclusions = task.project.exclude.clone();
        let before = now();
        task.manifest =
            Some(tokio::task::spawn_blocking(move || sync::scan(&root, &exclusions)).await??);
        result["phases_ms"]["scan"] = json!(now() - before);
    }
    if active.cancel.is_cancelled() {
        return Err(Fault::new("cancelled", "cancelled before dispatch").into());
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
                tokio::select! {r=transfer=>{if let Err(e)=r {let _=peer.control.send(Control::Cancel{job_id:task.job_id.clone()}).await;peer.connection.close(2u32.into(),b"sync source failed");return Err(e);}},_ = active.cancel.cancelled()=>{let _=peer.control.send(Control::Cancel{job_id:task.job_id.clone()}).await;}}
            }
            Event::Log { name, text } => {
                if !["build.log", "test.log", "run.log"].contains(&name.as_str()) {
                    anyhow::bail!("unexpected log path");
                }
                let mut f = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(active.directory.join(&name))
                    .await?;
                f.write_all(text.as_bytes()).await?;
                if !result["logs"].as_array().unwrap().contains(&json!(name)) {
                    result["logs"].as_array_mut().unwrap().push(json!(name));
                }
                let _ = active.reply.send(Reply::Log { name, text });
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
            Event::Artifact { path, data } => {
                use base64::Engine;
                let rel = sync::relative(&path)?;
                let dest = active.directory.join(rel);
                if let Some(p) = dest.parent() {
                    tokio::fs::create_dir_all(p).await?;
                }
                let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
                let mut f = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dest)
                    .await?;
                f.write_all(&bytes).await?;
                if !result["artifacts"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(path))
                {
                    result["artifacts"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(path));
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
