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
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub server: String,
    pub cert: PathBuf,
    pub token_file: PathBuf,
    pub data: PathBuf,
    pub config: Option<PathBuf>,
}
pub async fn worker(opts: Options) -> Result<()> {
    worker_managed(opts, None, None, false).await
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub options: Options,
}
pub async fn worker_managed(
    opts: Options,
    control: Option<PathBuf>,
    injected_token: Option<String>,
    parent_pipe: bool,
) -> Result<()> {
    let mut tokens = std::collections::BTreeMap::new();
    if let Some(token) = injected_token {
        tokens.insert("primary".to_owned(), token);
    }
    worker_profiles(
        vec![Profile {
            id: "primary".into(),
            name: opts.server.to_string(),
            options: opts,
        }],
        control,
        tokens,
        parent_pipe,
    )
    .await
}
pub async fn worker_profiles(
    profiles: Vec<Profile>,
    control: Option<PathBuf>,
    tokens: std::collections::BTreeMap<String, String>,
    parent_pipe: bool,
) -> Result<()> {
    anyhow::ensure!(
        !profiles.is_empty() && profiles.len() <= 16,
        "one to sixteen server connections required"
    );
    let mut ids = std::collections::BTreeSet::new();
    let mut paths = std::collections::BTreeSet::new();
    let mut targets = std::collections::BTreeSet::new();
    for profile in &profiles {
        anyhow::ensure!(
            profile.id == "primary" || uuid::Uuid::parse_str(&profile.id).is_ok(),
            "invalid connection id"
        );
        anyhow::ensure!(
            ids.insert(profile.id.clone())
                && paths.insert(profile.options.data.clone())
                && targets.insert(profile.options.server.clone()),
            "duplicate connection id, data directory or server address"
        );
    }
    let mut locks = Vec::new();
    for profile in &profiles {
        locks.push(wire::lock(&profile.options.data)?);
    }
    let identity = profiles[0].options.data.join("client-id");
    let client_id = if identity.exists() {
        std::fs::read_to_string(&identity)?.trim().to_owned()
    } else {
        let value = id();
        wire::private_write(&identity, value.as_bytes())?;
        value
    };
    anyhow::ensure!(
        uuid::Uuid::parse_str(&client_id).is_ok(),
        "invalid saved client identity"
    );
    let shutdown = CancellationToken::new();
    if parent_pipe {
        let stop = shutdown.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 1];
            let _ = std::io::stdin().read(&mut buf);
            stop.cancel();
        });
    }
    let sync_lock = Arc::new(Mutex::new(()));
    let mut locals = Vec::new();
    let mut jobs = Vec::new();
    let mut engines = Vec::new();
    // Prepare every connection before spawning: a configuration failure cannot leave orphan jobs.
    for profile in profiles {
        let opts = &profile.options;
        let config = if let Some(path) = &opts.config {
            toml::from_str(&std::fs::read_to_string(path)?)?
        } else {
            WorkerConfig::default()
        };
        let engine = Engine::open(opts.data.clone(), config)?;
        engine.prune_history().await?;
        let endpoint = wire::client(&opts.cert)?;
        let token = tokens.get(&profile.id).cloned().unwrap_or(
            std::fs::read_to_string(&opts.token_file)
                .unwrap_or_default()
                .trim()
                .to_owned(),
        );
        anyhow::ensure!(
            !token.is_empty(),
            "missing credential for connection {}",
            profile.id
        );
        let (tx, rx) = tokio::sync::watch::channel(crate::local::ConnectionState::initial(
            opts.server.to_string(),
        ));
        locals.push(crate::local::LocalConnection {
            id: profile.id.clone(),
            name: profile.name.clone(),
            engine: engine.clone(),
            connection: rx,
        });
        engines.push(engine.clone());
        jobs.push((profile, endpoint, token, engine, tx));
    }
    let mut tasks = tokio::task::JoinSet::new();
    for (profile, endpoint, token, engine, connection) in jobs {
        let lock = sync_lock.clone();
        let stop = shutdown.clone();
        let client = client_id.clone();
        tasks.spawn(async move {
            run_connection(
                endpoint,
                stop,
                Session {
                    opts: profile.options,
                    token,
                    instance: id(),
                    client_id: client,
                    connection_id: profile.id,
                    engine,
                    sync_lock: lock,
                    connection,
                },
            )
            .await
        });
    }
    if let Some(socket) = control {
        let stop = shutdown.clone();
        tasks.spawn(async move {
            let result = crate::local::serve_connections(socket, locals, stop.clone()).await;
            if result.is_err() {
                stop.cancel();
            }
            result
        });
    }
    let cleanup = engines.clone();
    let stop = shutdown.clone();
    tasks.spawn(async move {loop{tokio::select!{_=stop.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(3600))=>{for engine in &cleanup{let _=engine.prune_history().await;}}}}Ok(())});
    let mut outcome = tokio::select! {
        _=shutdown.cancelled()=>Ok(()),
        result=tokio::signal::ctrl_c()=>result.map_err(anyhow::Error::from),
        result=tasks.join_next()=>match result {
            Some(Ok(result)) => result,
            Some(Err(error)) => Err(error.into()),
            None => Ok(()),
        }
    };
    shutdown.cancel();
    for engine in &engines {
        if let Err(error) = engine.shutdown().await
            && outcome.is_ok()
        {
            outcome = Err(error);
        }
    }
    while let Some(result) = tasks.join_next().await {
        let result = result.map_err(anyhow::Error::from).and_then(|r| r);
        if outcome.is_ok() {
            outcome = result;
        }
    }
    drop(locks);
    outcome
}
struct Session {
    opts: Options,
    token: String,
    instance: String,
    client_id: String,
    connection_id: String,
    engine: Arc<Engine>,
    sync_lock: Arc<Mutex<()>>,
    connection: tokio::sync::watch::Sender<crate::local::ConnectionState>,
}
async fn run_connection(
    endpoint: quinn::Endpoint,
    shutdown: CancellationToken,
    state: Session,
) -> Result<()> {
    let connection = &state.connection;
    let mut retry = 1;
    loop {
        let attempt = async {
            connection.send_modify(|s|{s.state="connecting".into();s.checks=json!({"transport":false,"certificate":false,"authentication":false,"protocol":false});});
            let conn =
                crate::transport::connect(&endpoint, &state.opts.server, &state.opts.cert).await?;
            connection.send_modify(|s| {
                s.checks["transport"] = json!(true);
                s.checks["certificate"] = json!(true);
            });
            session(conn, &state).await
        };
        tokio::select! {r=attempt=>{connection.send_modify(|s|{s.state="disconnected".into();s.since=now();s.rtt_ms=None;s.error=r.as_ref().err().map(ToString::to_string);});},_=shutdown.cancelled()=>break}
        tokio::select! {_=tokio::time::sleep(Duration::from_secs(retry))=>{},_=shutdown.cancelled()=>break}
        retry = (retry * 2).min(5);
    }
    endpoint.close(0u32.into(), b"worker stopped");
    Ok(())
}
async fn session(conn: crate::transport::Connection, state: &Session) -> Result<()> {
    let Session {
        opts,
        token,
        instance,
        client_id,
        connection_id,
        engine,
        sync_lock,
        connection,
    } = state;
    let (mut out, mut input) = conn.open_bi().await?;
    out.set_priority(10)?;
    let status = json!({"ready":true,"transport":conn.name(),"platform":std::env::consts::OS,"version":env!("CARGO_PKG_VERSION"),"instance":instance,"client_id":client_id,"capabilities":["sync_stream_v1", "sync_delta_pack_v1","binary_transfer_v1","artifact_refs_v1"],"name":sysinfo::System::host_name().unwrap_or_else(||"Macrun client".into())});
    wire::send(
        &mut out,
        &Control::Hello {
            protocol: PROTOCOL,
            token: token.clone(),
            instance: instance.clone(),
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
        s.rtt_ms = conn.rtt().map(|r| r.as_millis() as u64);
        s.error = None;
    });
    let mut rx = wire::read_channel::<_, Control>(input);
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    let mut last = now();
    loop {
        tokio::select! {
            m=rx.recv()=>{if let Control::Heartbeat{..}=m.ok_or_else(||anyhow::anyhow!("control closed"))?? {last=now();}},
            stream=conn.accept_bi()=>{let (out,input)=stream?;let e=engine.clone();let o=opts.clone();let c=conn.clone();let lock=sync_lock.clone();let source=connection_id.to_owned();tokio::spawn(async move {if handle(out,input,c,o,e,lock,source).await.is_err(){crate::logging::event("worker", "request_error", json!({"reason":"request_decode_or_transport_failed"}));}});},
            _=tick.tick()=>{connection.send_modify(|s|s.rtt_ms=conn.rtt().map(|r|r.as_millis() as u64));if now()-last>15000{conn.close(1u32.into(),b"heartbeat timeout");anyhow::bail!("heartbeat timeout");}wire::send(&mut out,&Control::Heartbeat{status:status.clone()}).await?;},
            _=conn.closed()=>break,
        }
    }
    Ok(())
}
async fn handle(
    mut out: crate::transport::SendStream,
    mut input: crate::transport::RecvStream,
    conn: crate::transport::Connection,
    opts: Options,
    engine: Arc<Engine>,
    lock: Arc<Mutex<()>>,
    source: String,
) -> Result<()> {
    let mut task: Task =
        tokio::time::timeout(Duration::from_secs(30), wire::recv(&mut input)).await??;
    out.set_priority(if crate::transfer::supported(&task.request.kind) {
        -1
    } else {
        5
    })?;
    if source != "primary" {
        if let Some(request) = task.request.args["request_id"].as_str() {
            task.request.args["request_id"] = json!(scoped_id(&source, request));
        }
        if matches!(
            task.request.kind.as_str(),
            "task.get" | "task.wait" | "task.cancel" | "artifact.download"
        ) && let Some(task_id) = task.request.args["task_id"].as_str()
            && !engine.owns_task(task_id)
        {
            task.request.args["task_id"] = json!(scoped_id(&source, task_id));
        }
    }
    let mut log = crate::logging::Operation::new("worker", &task.request.kind, &task.job_id);
    log.context(&task.request.args);
    if crate::transfer::supported(&task.request.kind) {
        let transfer = crate::transfer::serve(
            &task.request.kind,
            &task.request.args,
            &mut input,
            &mut out,
            &engine,
        )
        .await;
        match transfer {
            Ok(succeeded) => log.status(if succeeded { "succeeded" } else { "failed" }),
            Err(error) => {
                log.send(
                    &mut out,
                    &Reply::Error {
                        error: Fault::new("transfer_failed", error),
                    },
                )
                .await?;
            }
        }
        out.finish()?;
        return Ok(());
    }
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
    if task.request.args["stream_sync"] == true {
        let root = task.project.root();
        let state = opts
            .data
            .join("mirrors")
            .join(
                blake3::hash(root.to_string_lossy().as_bytes())
                    .to_hex()
                    .as_str(),
            )
            .join("manifest.json");
        let cached = crate::wire::blocking(move || sync::read_manifest(&state)).await?;
        let matched = cached.as_ref().ok().is_some_and(|m| {
            sync::digest(m).ok().as_deref() == task.request.args["manifest_digest"].as_str()
        });
        wire::send(&mut out, &Event::ManifestMatch { matched }).await?;
        task.manifest = Some(if matched {
            cached?
        } else {
            tokio::time::timeout(
                Duration::from_secs(task.project.sync_timeout_seconds),
                sync::recv_manifest(&mut input),
            )
            .await??
        });
    }
    let legacy_guard = if task.request.args["stream_sync"] == true {
        Ok(None)
    } else {
        lock.try_lock().map(Some)
    };
    let Ok(_guard) = legacy_guard else {
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
    input: &mut crate::transport::RecvStream,
    conn: &crate::transport::Connection,
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
        let lease = Arc::new(
            crate::workspace::Lease::acquire(std::slice::from_ref(&root)).map_err(sync_error)?,
        );
        let streamed = task.request.args["stream_sync"] == true;
        let operation = async {
            let r = root.clone();
            let st = state.clone();
            let guard = lease.clone();
            let strict = task.request.args["strict"] == true;
            let before = std::time::Instant::now();
            let receiver = Arc::new(
                crate::wire::blocking(move || {
                    let _guard = guard;
                    sync::Receiver::prepare_with(&r, &st, manifest, strict)
                })
                .await
                .map_err(sync_error)??,
            );
            let prepare_ms = before.elapsed().as_millis();
            let mut bytes = 0;
            let mut files = 0;
            let mut reused_bytes = 0;
            let mut packs = 0;
            if task.request.args["optimized_sync"] == true {
                let r = receiver.clone();
                let guard = lease.clone();
                let offers = crate::wire::blocking(move || {
                    let _guard = guard;
                    crate::sync_transfer::offers(&r)
                })
                .await
                .map_err(sync_error)?
                .map_err(sync_error)?;
                crate::sync_transfer::send_offers(tx, &offers)
                    .await
                    .map_err(sync_error)?;
                let stats =
                    crate::sync_transfer::receive(input, receiver.clone(), &offers, lease.clone())
                        .await
                        .map_err(sync_error)?;
                bytes = stats.bytes;
                files = stats.files;
                reused_bytes = stats.reused_bytes;
                packs = stats.packs;
            } else {
                if streamed {
                    for paths in receiver.needed.chunks(256) {
                        tx.send(Event::NeedPage {
                            paths: paths.to_vec(),
                            last: false,
                        })
                        .await
                        .map_err(disconnected)?;
                    }
                    tx.send(Event::NeedPage {
                        paths: vec![],
                        last: true,
                    })
                    .await
                    .map_err(disconnected)?;
                } else {
                    tx.send(Event::Need {
                        paths: receiver.needed.clone(),
                    })
                    .await
                    .map_err(disconnected)?;
                }

                engine
                    .external_progress(
                        ident,
                        json!({"received":0,"total":receiver.needed.len(),"bytes":0}),
                    )
                    .await
                    .map_err(sync_error)?;
                let mut remaining: std::collections::BTreeSet<_> =
                    receiver.needed.iter().cloned().collect();
                let mut progress_at = std::time::Instant::now();
                while !remaining.is_empty() {
                    let mut uni = if streamed {
                        None
                    } else {
                        Some(conn.accept_uni().await.map_err(disconnected)?)
                    };
                    let stream = if let Some(s) = uni.as_mut() {
                        s
                    } else {
                        &mut *input
                    };
                    let h: FileHeader = wire::recv(stream).await.map_err(disconnected)?;
                    if h.job_id != task.job_id || !remaining.remove(&h.path) {
                        return Err(Fault::new("sync_failed", "unexpected file stream"));
                    }
                    let Some(Entry::File { size, .. }) = receiver.manifest.entries.get(&h.path)
                    else {
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
                        &mut tokio::io::AsyncReadExt::take(
                            stream,
                            if streamed { *size } else { size + 1 },
                        ),
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
                    let installer = receiver.clone();
                    let guard = lease.clone();
                    crate::wire::blocking(move || {
                        let _guard = guard;
                        installer.install(&h.path, &temp)
                    })
                    .await
                    .map_err(sync_error)??;
                    bytes += copied;
                    files += 1;
                    if remaining.is_empty() || progress_at.elapsed() >= Duration::from_millis(250) {
                        engine
                        .external_progress(
                            ident,
                            json!({"received":files,"total":receiver.needed.len(),"bytes":bytes}),
                        )
                        .await
                        .map_err(sync_error)?;
                        progress_at = std::time::Instant::now();
                    }
                }
            }
            engine
                .external_progress(
                    ident,
                    json!({"received":files,"total":receiver.needed.len(),"bytes":bytes}),
                )
                .await
                .map_err(sync_error)?;
            let manifest = if streamed {
                let Event::ConfirmManifest { digest } =
                    wire::recv(input).await.map_err(disconnected)?
                else {
                    return Err(Fault::new("sync_failed", "missing source confirmation"));
                };
                if sync::digest(&receiver.manifest).map_err(sync_error)? != digest {
                    return Err(Fault::new("source_changed", "source manifest differs"));
                }
                receiver.manifest.clone()
            } else {
                let Event::SyncCommit { manifest } =
                    wire::recv(input).await.map_err(disconnected)?
                else {
                    return Err(Fault::new("sync_failed", "missing source confirmation"));
                };
                manifest
            };
            let commit = receiver.clone();
            let guard = lease.clone();
            let before = std::time::Instant::now();
            crate::wire::blocking(move || {
                let _guard = guard;
                commit.commit(&manifest)
            })
            .await
            .map_err(sync_error)??;
            let generation = crate::workspace::generation(&root).map_err(sync_error)?;
            detail(tx,json!({"generation":generation,"workspace_root":root,"sync":{"files":receiver.needed.len(),"bytes":bytes,"reused_bytes":reused_bytes,"packs":packs,"skipped":receiver.manifest.skipped,"prepare_ms":prepare_ms,"commit_ms":before.elapsed().as_millis()}})).await;
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

fn scoped_id(source: &str, request: &str) -> String {
    let digest = blake3::hash(format!("macrun-request:{source}:{request}").as_bytes());
    uuid::Uuid::from_slice(&digest.as_bytes()[..16])
        .unwrap()
        .to_string()
}
