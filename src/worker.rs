use crate::{
    config::Project,
    cua::{Cua, Run, Snapshot},
    model::*,
    process,
    sync::{self, Entry},
    wire,
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    process::Command,
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub struct Options {
    pub server: std::net::SocketAddr,
    pub cert: PathBuf,
    pub token_file: PathBuf,
    pub data: PathBuf,
    pub cua_binary: String,
    pub cua_socket: Option<String>,
}
struct Runtime {
    run: Option<Run>,
    snapshot: Option<Snapshot>,
    cua: Option<Cua>,
}
struct Activity {
    job: Option<String>,
    cancel: CancellationToken,
    pending: Vec<String>,
}
pub async fn worker(options: Options) -> Result<()> {
    let _lock = wire::lock(&options.data)?;
    let recovery = process::recover(&options.data.join("process.json")).err();
    let run = std::fs::read(options.data.join("run.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Run>(&b).ok())
        .filter(|r| process::alive(&r.identity));
    let runtime = Arc::new(Mutex::new(Runtime {
        run,
        snapshot: None,
        cua: None,
    }));
    let activity = Arc::new(Mutex::new(Activity {
        job: None,
        cancel: CancellationToken::new(),
        pending: vec![],
    }));
    let status = Arc::new(Mutex::new(
        json!({"ready":recovery.is_none(),"recovery":recovery,"platform":std::env::consts::OS,"version":env!("CARGO_PKG_VERSION"),"last_restart":"worker_restarted"}),
    ));
    status.lock().await["environment"] = environment(&options).await;
    let endpoint = wire::client(&options.cert)?;
    let token = std::fs::read_to_string(&options.token_file)?
        .trim()
        .to_string();
    let instance = id();
    let mut retry = 1;
    loop {
        let attempt = tokio::time::timeout(
            Duration::from_secs(5),
            endpoint.connect(options.server, "macrun")?,
        )
        .await;
        match attempt {
            Ok(Ok(conn)) => {
                retry = 1;
                if let Err(e) = session(
                    conn,
                    &options,
                    &token,
                    &instance,
                    runtime.clone(),
                    activity.clone(),
                    status.clone(),
                )
                .await
                {
                    eprintln!("connection: {e}");
                }
            }
            other => eprintln!("connect failed: {other:?}"),
        }
        if status.lock().await["stopping"] == true {
            break;
        }
        tokio::select! {_ = tokio::time::sleep(Duration::from_secs(retry))=>{},_ = tokio::signal::ctrl_c()=>break}
        retry = (retry * 2).min(5);
    }
    Ok(())
}
async fn session(
    conn: quinn::Connection,
    opts: &Options,
    token: &str,
    instance: &str,
    runtime: Arc<Mutex<Runtime>>,
    activity: Arc<Mutex<Activity>>,
    status: Arc<Mutex<Value>>,
) -> Result<()> {
    {
        let mut r = runtime.lock().await;
        r.snapshot = None;
        r.cua = None;
        status.lock().await["run"] = json!(r.run);
    }
    let (mut send, mut recv) = conn.open_bi().await?;
    wire::send(
        &mut send,
        &Control::Hello {
            protocol: PROTOCOL,
            token: token.into(),
            instance: instance.into(),
            status: status.lock().await.clone(),
        },
    )
    .await?;
    match wire::recv::<_, Control>(&mut recv).await? {
        Control::Welcome { protocol } if protocol == PROTOCOL => {}
        Control::Reject { error } => return Err(error.into()),
        _ => anyhow::bail!("incompatible server"),
    }
    eprintln!("worker connected to {}", opts.server);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    let mut last = now();
    let mut running: Option<tokio::task::JoinHandle<()>> = None;
    let mut control_rx = wire::read_channel::<_, Control>(recv);
    let result:Result<()>=async {loop {tokio::select! {
        control=control_rx.recv()=>{match control.ok_or_else(||anyhow::anyhow!("control stream closed"))?? {
            Control::Heartbeat{..}=>last=now(),
            Control::Cancel{job_id}=>{last=now();let mut a=activity.lock().await;if a.job.as_deref()==Some(&job_id){a.cancel.cancel();}else{a.pending.push(job_id);}},
            _=>anyhow::bail!("unexpected control message"),
        }},
        stream=conn.accept_bi()=>{
            let (out,input)=stream?;
            if status.lock().await["ready"]!=true {let mut out=out;wire::send(&mut out,&Event::Done{error:Some(Fault::new("busy","worker not ready"))}).await?;out.finish()?;continue;}
            status.lock().await["ready"]=json!(false);
            let r=runtime.clone();let a=activity.clone();let s=status.clone();let o=opts.clone();let c=conn.clone();
            running=Some(tokio::spawn(async move{handle_task(out,input,c,o,r,a,s).await;}));
        },
        _=heartbeat.tick()=>{if now()-last>15000{anyhow::bail!("heartbeat timeout");}wire::send(&mut send,&Control::Heartbeat{status:status.lock().await.clone()}).await?;},
        _=conn.closed()=>break,
        _=tokio::signal::ctrl_c()=>{status.lock().await["stopping"]=json!(true);conn.close(0u32.into(),b"worker stopping");break;},
    }}Ok(())}.await;
    conn.close(1u32.into(), b"worker disconnect");
    activity.lock().await.cancel.cancel();
    if let Some(task) = running {
        let _ = task.await;
    }
    result
}
async fn handle_task(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    conn: quinn::Connection,
    opts: Options,
    runtime: Arc<Mutex<Runtime>>,
    activity: Arc<Mutex<Activity>>,
    status: Arc<Mutex<Value>>,
) {
    let task: Task = match wire::recv(&mut recv).await {
        Ok(t) => t,
        Err(_) => {
            status.lock().await["ready"] = json!(true);
            return;
        }
    };
    let cancel = CancellationToken::new();
    {
        let mut a = activity.lock().await;
        a.job = Some(task.job_id.clone());
        a.cancel = cancel.clone();
        if a.pending.contains(&task.job_id) {
            cancel.cancel();
        }
        a.pending.clear();
    }
    status.lock().await["job_id"] = json!(task.job_id);
    let (tx, mut rx) = mpsc::channel(64);
    let writer = tokio::spawn(async move {
        while let Some(e) = rx.recv().await {
            if wire::send(&mut send, &e).await.is_err() {
                break;
            }
        }
        let _ = send.finish();
    });
    let mut r = runtime.lock().await;
    let previous_run = r.run.as_ref().map(|r| r.run_id.clone());
    let outcome = execute(&task, &mut recv, &conn, &opts, &mut r, &cancel, &tx).await;
    if outcome
        .as_ref()
        .is_err_and(|e| matches!(e.code.as_str(), "cancelled" | "worker_disconnected"))
        && r.run.as_ref().map(|r| r.run_id.clone()) != previous_run
    {
        let _ = stop(&mut r, &opts.data).await;
    }
    if outcome.is_err() {
        r.cua = None;
        r.snapshot = None;
    }
    let ready = outcome
        .as_ref()
        .err()
        .is_none_or(|e| e.code != "recovery_required");
    {
        let mut s = status.lock().await;
        s["ready"] = json!(ready);
        s["job_id"] = Value::Null;
        s["run"] = json!(r.run);
        s["last_error"] = json!(outcome.as_ref().err());
    }
    activity.lock().await.job = None;
    let _ = tx
        .send(Event::Done {
            error: outcome.err(),
        })
        .await;
    drop(tx);
    let _ = writer.await;
}
async fn execute(
    task: &Task,
    input: &mut quinn::RecvStream,
    conn: &quinn::Connection,
    opts: &Options,
    r: &mut Runtime,
    cancel: &CancellationToken,
    tx: &mpsc::Sender<Event>,
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
    let p = &task.project;
    if ["build", "test", "run"].contains(&task.request.kind.as_str()) {
        let testing = task.request.kind == "test";
        progress(tx, if testing { "testing" } else { "building" }).await;
        let (program, args) = build_command(p, testing, &task.request.args)?;
        tokio::fs::create_dir_all(p.cache())
            .await
            .map_err(sync_error)?;
        process::run(
            &program,
            &args,
            &p.root(),
            if testing {
                p.test_timeout_seconds
            } else {
                p.build_timeout_seconds
            },
            cancel,
            tx,
            if testing { "test.log" } else { "build.log" },
            &opts.data.join("process.json"),
        )
        .await?;
    }
    match task.request.kind.as_str() {
        "sync" | "build" | "test" => return Ok(()),
        "stop" => {
            check_run(r, &task.request.args)?;
            return stop(r, &opts.data).await;
        }
        "run" => {
            progress(tx, "launching").await;
            launch(p, r, &opts.data, cancel).await?;
            detail(tx,json!({"run":r.run,"run_id":r.run.as_ref().unwrap().run_id,"artifact_path":r.run.as_ref().unwrap().app_path})).await;
        }
        _ => {
            check_run(r, &task.request.args)?;
        }
    }
    let operation = gui(task, opts, r, tx);
    tokio::select! {result=operation=>result,_=cancel.cancelled()=>Err(Fault::new("cancelled","UI task cancelled; observe before another action")),_=tokio::time::sleep(Duration::from_secs(if task.request.kind=="run"{p.launch_timeout_seconds}else{p.ui_timeout_seconds}))=>Err(Fault::new("timed_out","window operation timed out"))}
}
pub fn build_command(p: &Project, testing: bool, args: &Value) -> Outcome<(String, Vec<String>)> {
    if p.package {
        let mut a = vec![
            if testing { "test" } else { "build" }.into(),
            "--scratch-path".into(),
            p.cache().to_string_lossy().into(),
        ];
        if let Some(f) = args["filter"].as_str() {
            a.extend(["--filter".into(), f.into()]);
        }
        return Ok(("swift".into(), a));
    }
    let mut a = vec![
        if p.workspace.is_some() {
            "-workspace"
        } else {
            "-project"
        }
        .into(),
        p.workspace.as_ref().or(p.project.as_ref()).unwrap().clone(),
        "-scheme".into(),
        p.scheme.clone(),
        "-configuration".into(),
        p.configuration.clone(),
        "-destination".into(),
        "platform=macOS".into(),
        "-derivedDataPath".into(),
        p.cache().to_string_lossy().into(),
        if testing { "test" } else { "build" }.into(),
    ];
    if let Some(f) = args["filter"].as_str() {
        a.push(format!("-only-testing:{f}"));
    }
    Ok(("xcodebuild".into(), a))
}
fn check_run(r: &Runtime, args: &Value) -> Outcome<()> {
    let run = r
        .run
        .as_ref()
        .ok_or_else(|| Fault::new("app_not_running", "no active run"))?;
    if args["run"].as_str().is_some_and(|id| id != run.run_id) {
        return Err(Fault::new("target_mismatch", "run id mismatch"));
    }
    if !process::alive(&run.identity) {
        return Err(Fault::new("app_not_running", "recorded process exited"));
    }
    Ok(())
}
async fn gui(
    task: &Task,
    opts: &Options,
    r: &mut Runtime,
    tx: &mpsc::Sender<Event>,
) -> Outcome<()> {
    if r.cua.is_none() {
        r.cua = Some(Cua::connect(&opts.cua_binary, opts.cua_socket.as_deref()).await?);
    }
    let run = r
        .run
        .clone()
        .ok_or_else(|| Fault::new("app_not_running", "no active run"))?;
    let cua = r.cua.as_mut().unwrap();
    let start = tokio::time::Instant::now();
    let windows = loop {
        if !process::alive(&run.identity) {
            return Err(Fault::new("app_exited", "application exited"));
        }
        let windows = cua.windows(run.identity.pid).await?;
        if !windows.is_empty() {
            break windows;
        }
        if task.request.kind != "run"
            || start.elapsed().as_secs() >= task.project.launch_timeout_seconds.saturating_sub(1)
        {
            return Err(Fault::new("window_not_found", "no application windows"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    detail(tx, json!({"windows":windows})).await;
    let requested = task.request.args["window"].as_u64();
    let w = if let Some(w) = requested {
        windows
            .iter()
            .find(|v| v["window_id"] == w)
            .ok_or_else(|| Fault::new("target_mismatch", "window not owned by current run"))?
    } else if windows.len() == 1 || task.request.kind == "run" {
        &windows[0]
    } else {
        return Err(Fault::new(
            "window_required",
            "multiple windows; pass --window",
        ));
    };
    let window = w["window_id"]
        .as_u64()
        .ok_or_else(|| Fault::new("observation_failed", "missing window id"))?;
    let action = matches!(
        task.request.kind.as_str(),
        "click" | "type" | "key" | "scroll"
    );
    if action {
        progress(tx, "acting").await;
        let snap = r
            .snapshot
            .take()
            .ok_or_else(|| Fault::new("stale_snapshot", "observe before action"))?;
        if task.request.args["snapshot"] != snap.id
            || snap.run_id != run.run_id
            || snap.window != window
            || !["x", "y", "width", "height"]
                .iter()
                .all(|k| w["bounds"][k].as_f64() == snap.bounds[k].as_f64())
        {
            return Err(Fault::new(
                "stale_snapshot",
                "snapshot expired or window geometry changed",
            ));
        }
        let tool = match task.request.kind.as_str() {
            "type" => "type_text",
            "key" => "press_key",
            v => v,
        };
        let args = cua.action_args(tool, &run, &snap, &task.request.args, false)?;
        detail(
            tx,
            json!({"action_status":"unknown","delivery":"background"}),
        )
        .await;
        let mut result = cua.call(tool, args).await;
        if result
            .as_ref()
            .is_err_and(|e| e.code == "background_unavailable")
            && task.project.allow_foreground_fallback
        {
            detail(
                tx,
                json!({"cua_refusal":"background_unavailable","delivery":"foreground"}),
            )
            .await;
            result = cua
                .call(
                    tool,
                    cua.action_args(tool, &run, &snap, &task.request.args, true)?,
                )
                .await;
        }
        match result {
            Ok(v) => {
                detail(
                    tx,
                    json!({"action_status":"accepted","cua_action":crate::cua::structured(&v)}),
                )
                .await;
            }
            Err(e) => {
                detail(tx,json!({"action_status":if e.code=="cua_unavailable"{"unknown"}else{"refused"},"cua_refusal":e.code})).await;
                return Err(Fault::new("cua_refused", e));
            }
        }
    }
    r.snapshot = None;
    progress(tx, "observing").await;
    let observation = cua
        .observe(
            &run,
            window,
            task.project.screenshot_max_edge,
            &opts.data.join("captures"),
        )
        .await;
    let (snap, content, png) = match observation {
        Ok(v) => v,
        Err(e) => {
            detail(tx, json!({"observation_status":"failed"})).await;
            return Err(e);
        }
    };
    detail(tx, content["detail"].clone()).await;
    artifact(
        tx,
        "accessibility.json",
        &serde_json::to_vec_pretty(&content["accessibility"]).unwrap(),
    )
    .await?;
    artifact(tx, "screenshots/001.png", &png).await?;
    r.snapshot = Some(snap);
    Ok(())
}
async fn launch(
    p: &Project,
    r: &mut Runtime,
    data: &Path,
    cancel: &CancellationToken,
) -> Outcome<()> {
    if !cfg!(target_os = "macos") {
        return Err(Fault::new("no_gui_session", "run requires macOS"));
    }
    let gui = gui_environment();
    if gui["gui_user_matches"] == false {
        return Err(Fault::new(
            "no_gui_session",
            "worker must run as the logged-in graphical user",
        ));
    }
    if gui["display_count"] == 0 {
        return Err(Fault::new("no_display", "no active display"));
    }
    let relative = p
        .app_relative_path
        .as_ref()
        .ok_or_else(|| Fault::new("invalid_config", "run requires app_relative_path"))?;
    let app = std::fs::canonicalize(p.cache().join(sync::relative(relative)?))
        .map_err(|e| Fault::new("artifact_not_found", e))?;
    let info = plist::Value::from_file(app.join("Contents/Info.plist"))
        .map_err(|e| Fault::new("artifact_not_found", e))?;
    let info = info
        .as_dictionary()
        .ok_or_else(|| Fault::new("artifact_not_found", "invalid Info.plist"))?;
    let bundle = info
        .get("CFBundleIdentifier")
        .and_then(plist::Value::as_string)
        .ok_or_else(|| Fault::new("artifact_not_found", "missing bundle id"))?
        .to_string();
    let executable = info
        .get("CFBundleExecutable")
        .and_then(plist::Value::as_string)
        .ok_or_else(|| Fault::new("artifact_not_found", "missing executable"))?;
    let exe = std::fs::canonicalize(app.join("Contents/MacOS").join(executable))
        .map_err(|e| Fault::new("artifact_not_found", e))?;
    if bundle_pids(&bundle)
        .iter()
        .any(|pid| r.run.as_ref().is_none_or(|run| run.identity.pid != *pid))
    {
        return Err(Fault::new(
            "app_instance_conflict",
            "another instance of this bundle is already running",
        ));
    }
    stop(r, data).await?;
    let mut system = sysinfo::System::new_all();
    if system.processes().values().any(|p| p.exe() == Some(&exe)) {
        return Err(Fault::new(
            "app_instance_conflict",
            "target executable already running without a macrun session",
        ));
    }
    let out = Command::new("/usr/bin/open")
        .args(["-n", "-g"])
        .arg(&app)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| Fault::new("no_gui_session", e))?;
    if !out.status.success() {
        return Err(Fault::new(
            "no_gui_session",
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    let start = tokio::time::Instant::now();
    loop {
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let found: Vec<_> = system
            .processes()
            .values()
            .filter(|p| p.exe() == Some(&exe))
            .collect();
        if found.len() > 1 {
            return Err(Fault::new(
                "app_instance_conflict",
                "multiple matching processes",
            ));
        }
        if let Some(proc) = found.first() {
            let identity = process::identity(proc.pid().as_u32())
                .ok_or_else(|| Fault::new("app_exited", "application exited during launch"))?;
            r.run = Some(Run {
                run_id: id(),
                identity,
                app_path: app.to_string_lossy().into(),
                bundle_id: bundle,
            });
            r.snapshot = None;
            wire::atomic_json(&data.join("run.json"), &r.run)
                .map_err(|e| Fault::new("recovery_required", e))?;
            return Ok(());
        }
        if cancel.is_cancelled() {
            return Err(Fault::new("cancelled", "launch cancelled"));
        }
        if start.elapsed().as_secs() >= p.launch_timeout_seconds {
            return Err(Fault::new("app_exited", "no process found after launch"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
async fn stop(r: &mut Runtime, data: &Path) -> Outcome<()> {
    if let Some(run) = r.run.as_ref()
        && process::alive(&run.identity)
    {
        request_quit(run.identity.pid);
        let start = tokio::time::Instant::now();
        while process::alive(&run.identity) && start.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if process::alive(&run.identity) {
            unsafe {
                libc::kill(run.identity.pid as i32, libc::SIGKILL);
            }
            for _ in 0..30 {
                if !process::alive(&run.identity) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if process::alive(&run.identity) {
                return Err(Fault::new(
                    "recovery_required",
                    "application did not exit after termination",
                ));
            }
        }
    }
    r.run = None;
    r.snapshot = None;
    if data.join("run.json").exists() {
        std::fs::remove_file(data.join("run.json"))
            .map_err(|e| Fault::new("recovery_required", e))?;
    }
    Ok(())
}
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn request_quit(pid: u32) {
    use objc::{class, msg_send, sel, sel_impl};
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}
    unsafe {
        let app: *mut objc::runtime::Object = msg_send![class!(NSRunningApplication),runningApplicationWithProcessIdentifier:pid as i32];
        if !app.is_null() {
            let _: bool = msg_send![app, terminate];
        }
    }
}
#[cfg(not(target_os = "macos"))]
fn request_quit(pid: u32) {
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
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
async fn artifact(tx: &mpsc::Sender<Event>, path: &str, bytes: &[u8]) -> Outcome<()> {
    use base64::Engine;
    for b in bytes.chunks(65536) {
        tx.send(Event::Artifact {
            path: path.into(),
            data: base64::engine::general_purpose::STANDARD.encode(b),
        })
        .await
        .map_err(disconnected)?;
    }
    Ok(())
}
fn sync_error(e: impl ToString) -> Fault {
    Fault::new("sync_failed", e)
}
fn disconnected(e: impl ToString) -> Fault {
    Fault::new("worker_disconnected", e)
}

async fn environment(options: &Options) -> Value {
    let mut v = gui_environment();
    v["xcode_version"] = json!(probe("xcodebuild", &["-version"]).await);
    v["cua_version"] = json!(probe(&options.cua_binary, &["--version"]).await);
    v["cua_status"] = json!(probe(&options.cua_binary, &["status"]).await);
    v["launch_context"] = json!(probe("/bin/launchctl", &["managername"]).await);
    v
}
async fn probe(program: &str, args: &[&str]) -> String {
    match tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(program).args(args).kill_on_drop(true).output(),
    )
    .await
    {
        Ok(Ok(o)) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Ok(Ok(o)) => format!("unavailable: {}", String::from_utf8_lossy(&o.stderr).trim()),
        _ => "unavailable".into(),
    }
}
#[cfg(target_os = "macos")]
fn gui_environment() -> Value {
    use std::os::unix::fs::MetadataExt;
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    }
    let mut count = 0;
    let mut displays = [0; 16];
    let code = unsafe { CGGetActiveDisplayList(16, displays.as_mut_ptr(), &mut count) };
    let uid = unsafe { libc::getuid() };
    json!({"gui_user_matches":std::fs::metadata("/dev/console").ok().is_some_and(|m|m.uid()==uid),"display_count":if code==0{Some(count)}else{None}})
}
#[cfg(not(target_os = "macos"))]
fn gui_environment() -> Value {
    json!({"gui_user_matches":false,"display_count":0})
}
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn bundle_pids(bundle: &str) -> Vec<u32> {
    use objc::{class, msg_send, sel, sel_impl};
    let Ok(c) = std::ffi::CString::new(bundle) else {
        return vec![];
    };
    unsafe {
        let text: *mut objc::runtime::Object =
            msg_send![class!(NSString),stringWithUTF8String:c.as_ptr()];
        let apps: *mut objc::runtime::Object =
            msg_send![class!(NSRunningApplication),runningApplicationsWithBundleIdentifier:text];
        if apps.is_null() {
            return vec![];
        }
        let count: usize = msg_send![apps, count];
        (0..count)
            .map(|i| {
                let app: *mut objc::runtime::Object = msg_send![apps,objectAtIndex:i];
                let pid: i32 = msg_send![app, processIdentifier];
                pid as u32
            })
            .collect()
    }
}
#[cfg(not(target_os = "macos"))]
fn bundle_pids(_bundle: &str) -> Vec<u32> {
    vec![]
}
