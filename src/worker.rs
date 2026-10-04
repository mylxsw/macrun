use crate::{
    config::WorkerConfig,
    engine::Engine,
    model::*,
    sync::{self, Entry},
    wire,
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub struct Options {
    pub server: std::net::SocketAddr,
    pub cert: PathBuf,
    pub token_file: PathBuf,
    pub data: PathBuf,
    pub config: Option<PathBuf>,
}
pub async fn worker(opts: Options) -> Result<()> {
    worker_managed(opts, None, None, false).await
}
pub async fn worker_managed(
    opts: Options,
    control: Option<PathBuf>,
    injected_token: Option<String>,
    parent_pipe: bool,
) -> Result<()> {
    let _lock = wire::lock(&opts.data)?;
    let config: WorkerConfig = if let Some(p) = &opts.config {
        toml::from_str(&std::fs::read_to_string(p)?)?
    } else {
        WorkerConfig::default()
    };
    let engine = Engine::open(opts.data.clone(), config)?;
    engine.prune_history().await?;
    let sync_lock = Arc::new(Mutex::new(()));
    let endpoint = wire::client(&opts.cert)?;
    let token = if let Some(token) = injected_token {
        token
    } else {
        std::fs::read_to_string(&opts.token_file)?
            .trim()
            .to_string()
    };
    anyhow::ensure!(!token.is_empty(), "empty connection token");
    let shutdown = CancellationToken::new();
    if parent_pipe {
        let stop = shutdown.clone();
        // A dedicated thread must not hold Tokio runtime shutdown open.
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 1];
            let _ = std::io::stdin().read(&mut buf);
            stop.cancel();
        });
    }
    let (connection, rx) = tokio::sync::watch::channel(crate::local::ConnectionState::initial(
        opts.server.to_string(),
    ));
    let local_task = control.map(|socket| {
        let e = engine.clone();
        let s = shutdown.clone();
        tokio::spawn(async move {
            let r = crate::local::serve(socket, e, rx, s.clone()).await;
            if r.is_err() {
                s.cancel();
            }
            r
        })
    });
    let cleanup_engine = engine.clone();
    let cleanup_stop = shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _=cleanup_stop.cancelled()=>break,
                _=tokio::time::sleep(Duration::from_secs(3600))=>{let _=cleanup_engine.prune_history().await;}
            }
        }
    });
    let instance = id();
    let mut retry = 1;
    loop {
        let attempt = async {
            connection.send_modify(|s|{s.state="connecting".into();s.checks=json!({"transport":false,"certificate":false,"authentication":false,"protocol":false});});
            let conn = tokio::time::timeout(
                Duration::from_secs(5),
                endpoint.connect(opts.server, "macrun")?,
            )
            .await??;
            connection.send_modify(|s| {
                s.checks["transport"] = json!(true);
                s.checks["certificate"] = json!(true);
            });
            session(
                conn,
                &opts,
                &token,
                &instance,
                engine.clone(),
                sync_lock.clone(),
                &connection,
            )
            .await
        };
        tokio::select! {r=attempt=>{connection.send_modify(|s|{s.state="disconnected".into();s.since=now();s.rtt_ms=None;s.error=r.as_ref().err().map(ToString::to_string);});crate::logging::event("worker", "reconnecting", json!({"status":if r.is_err(){"disconnected"}else{"closed"},"retry_seconds":retry}));},_=tokio::signal::ctrl_c()=>break,_=shutdown.cancelled()=>break}
        tokio::select! {_=tokio::time::sleep(Duration::from_secs(retry))=>{},_=tokio::signal::ctrl_c()=>break,_=shutdown.cancelled()=>break}
        retry = (retry * 2).min(5);
    }
    shutdown.cancel();
    engine.shutdown().await?;
    if let Some(task) = local_task {
        task.await??;
    }
    endpoint.close(0u32.into(), b"worker stopped");
    Ok(())
}
async fn session(
    conn: quinn::Connection,
    opts: &Options,
    token: &str,
    instance: &str,
    engine: Arc<Engine>,
    sync_lock: Arc<Mutex<()>>,
    connection: &tokio::sync::watch::Sender<crate::local::ConnectionState>,
) -> Result<()> {
    let (mut out, mut input) = conn.open_bi().await?;
    let status = json!({"ready":true,"platform":std::env::consts::OS,"version":env!("CARGO_PKG_VERSION"),"instance":instance});
    wire::send(
        &mut out,
        &Control::Hello {
            protocol: PROTOCOL,
            token: token.into(),
            instance: instance.into(),
            status: status.clone(),
        },
    )
    .await?;
    match wire::recv::<_, Control>(&mut input).await? {
        Control::Welcome { protocol } if protocol == PROTOCOL => {
            connection.send_modify(|s| {
                s.checks["authentication"] = json!(true);
                s.checks["protocol"] = json!(true);
            });
        }
        Control::Reject { error } => anyhow::bail!("{}", error),
        other => anyhow::bail!("registration refused: {other:?}"),
    }
    crate::logging::event(
        "worker",
        "connected",
        json!({"server":opts.server.to_string(),"instance":instance}),
    );
    connection.send_modify(|s| {
        s.state = "connected".into();
        s.since = now();
        s.rtt_ms = Some(conn.rtt().as_millis() as u64);
        s.error = None;
    });
    let mut rx = wire::read_channel::<_, Control>(input);
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    let mut last = now();
    loop {
        tokio::select! {
            m=rx.recv()=>{if let Control::Heartbeat{..}=m.ok_or_else(||anyhow::anyhow!("control closed"))?? {last=now();}},
            stream=conn.accept_bi()=>{let (out,input)=stream?;let e=engine.clone();let o=opts.clone();let c=conn.clone();let lock=sync_lock.clone();tokio::spawn(async move {if handle(out,input,c,o,e,lock).await.is_err(){crate::logging::event("worker", "request_error", json!({"reason":"request_decode_or_transport_failed"}));}});},
            _=tick.tick()=>{connection.send_modify(|s|s.rtt_ms=Some(conn.rtt().as_millis() as u64));if now()-last>15000{conn.close(1u32.into(),b"heartbeat timeout");anyhow::bail!("heartbeat timeout");}wire::send(&mut out,&Control::Heartbeat{status:status.clone()}).await?;},
            _=conn.closed()=>break,
        }
    }
    Ok(())
}
async fn handle(
    mut out: quinn::SendStream,
    mut input: quinn::RecvStream,
    conn: quinn::Connection,
    opts: Options,
    engine: Arc<Engine>,
    lock: Arc<Mutex<()>>,
) -> Result<()> {
    let task: Task = wire::recv(&mut input).await?;
    let mut log = crate::logging::Operation::new("worker", &task.request.kind, &task.job_id);
    log.context(&task.request.args);
    if task.request.kind != "sync" {
        let r = engine.handle(&task.request.kind, task.request.args).await;
        let reply = match r {
            Ok(value) => Reply::Status { value },
            Err(e) => Reply::Error {
                error: e
                    .downcast_ref::<Fault>()
                    .cloned()
                    .unwrap_or_else(|| Fault::new("operation_failed", e)),
            },
        };
        log.send(&mut out, &reply).await?;
        out.finish()?;
        return Ok(());
    }
    let Ok(_guard) = lock.try_lock() else {
        log.status("busy");
        wire::send(
            &mut out,
            &Event::Done {
                error: Some(Fault::new("busy", "another sync is active")),
            },
        )
        .await?;
        out.finish()?;
        return Ok(());
    };
    let (ident, cancel) = match engine
        .begin_external(
            "sync",
            json!({"remote_root":task.project.remote_root,"job_id":task.job_id}),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let error = e
                .downcast_ref::<Fault>()
                .cloned()
                .unwrap_or_else(|| Fault::new("operation_failed", e));
            wire::send(&mut out, &Event::Done { error: Some(error) }).await?;
            out.finish()?;
            return Ok(());
        }
    };
    let (tx, mut rx) = mpsc::channel(64);
    let writer = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if wire::send(&mut out, &event).await.is_err() {
                break;
            }
        }
        let _ = out.finish();
    });
    let r = receive_sync(
        &task, &mut input, &conn, &opts, &cancel, &tx, &engine, &ident,
    )
    .await;
    engine
        .finish_external(
            &ident,
            r.as_ref().err().map(ToString::to_string),
            r.as_ref().err().map(|e| e.code.as_str()),
        )
        .await?;
    log.status(if r.is_ok() { "succeeded" } else { "failed" });
    let _ = tx.send(Event::Done { error: r.err() }).await;
    drop(tx);
    let _ = writer.await;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
async fn receive_sync(
    task: &Task,
    input: &mut quinn::RecvStream,
    conn: &quinn::Connection,
    opts: &Options,
    cancel: &CancellationToken,
    tx: &mpsc::Sender<Event>,
    engine: &Engine,
    ident: &str,
) -> Outcome<()> {
    if let Some(manifest) = task.manifest.clone() {
        progress(tx, "syncing").await;
        let root = task.project.root();
        let state = opts.data.join("mirrors").join(
            blake3::hash(root.to_string_lossy().as_bytes())
                .to_hex()
                .as_str(),
        );
        let operation = async {
            let receiver = sync::Receiver::prepare(&root, &state, manifest)?;
            tx.send(Event::Need {
                paths: receiver.needed.clone(),
            })
            .await
            .map_err(disconnected)?;
            let mut bytes = 0;
            let mut files = 0;
            engine
                .external_progress(
                    ident,
                    json!({"received":0,"total":receiver.needed.len(),"bytes":0}),
                )
                .await
                .map_err(sync_error)?;
            let mut remaining: std::collections::BTreeSet<_> =
                receiver.needed.iter().cloned().collect();
            while !remaining.is_empty() {
                let mut stream = conn.accept_uni().await.map_err(disconnected)?;
                let h: FileHeader = wire::recv(&mut stream).await.map_err(disconnected)?;
                if h.job_id != task.job_id || !remaining.remove(&h.path) {
                    return Err(Fault::new("sync_failed", "unexpected file stream"));
                }
                let Some(Entry::File { size, .. }) = receiver.manifest.entries.get(&h.path) else {
                    return Err(Fault::new("sync_failed", "unexpected entry"));
                };
                if h.size != *size {
                    return Err(Fault::new("sync_failed", "size mismatch"));
                }
                let tempdir = root.join(".macrun/incoming");
                tokio::fs::create_dir_all(&tempdir)
                    .await
                    .map_err(sync_error)?;
                let temp = tempdir.join(id());
                let mut f = tokio::fs::File::create(&temp).await.map_err(sync_error)?;
                let copied = tokio::io::copy(
                    &mut tokio::io::AsyncReadExt::take(&mut stream, size + 1),
                    &mut f,
                )
                .await
                .map_err(sync_error)?;
                f.flush().await.map_err(sync_error)?;
                f.sync_all().await.map_err(sync_error)?;
                drop(f);
                if copied != *size {
                    return Err(Fault::new("sync_failed", "truncated file"));
                }
                receiver.install(&h.path, &temp)?;
                bytes += copied;
                files += 1;
                engine
                    .external_progress(
                        ident,
                        json!({"received":files,"total":receiver.needed.len(),"bytes":bytes}),
                    )
                    .await
                    .map_err(sync_error)?;
            }
            let Event::SyncCommit { manifest } = wire::recv(input).await.map_err(disconnected)?
            else {
                return Err(Fault::new("sync_failed", "missing source confirmation"));
            };
            receiver.commit(&manifest)?;
            detail(tx,json!({"sync":{"files":receiver.needed.len(),"bytes":bytes,"skipped":receiver.manifest.skipped}})).await;
            Ok(())
        };
        tokio::select! {result=operation=>result?,_=cancel.cancelled()=>return Err(Fault::new("cancelled","sync cancelled")),_=tokio::time::sleep(Duration::from_secs(task.project.sync_timeout_seconds))=>return Err(Fault::new("timed_out","sync timeout"))}
    }
    Ok(())
}
fn disconnected(e: impl ToString) -> Fault {
    Fault::new("worker_disconnected", e)
}
fn sync_error(e: impl ToString) -> Fault {
    Fault::new("sync_failed", e)
}
async fn progress(tx: &mpsc::Sender<Event>, phase: &str) {
    let _ = tx
        .send(Event::Progress {
            phase: phase.into(),
        })
        .await;
}
async fn detail(tx: &mpsc::Sender<Event>, value: Value) {
    let _ = tx.send(Event::Detail { value }).await;
}
