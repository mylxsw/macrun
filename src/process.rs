use crate::{
    model::{Event, Fault, Outcome},
    process_tree::{OWNER_ENV, Tree},
    wire,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, sync::mpsc};
use tokio_util::sync::CancellationToken;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub pid: u32,
    pub started: u64,
    pub executable: PathBuf,
}
pub fn identity(pid: u32) -> Option<Identity> {
    let mut s = sysinfo::System::new();
    s.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        true,
    );
    let p = s.process(sysinfo::Pid::from_u32(pid))?;
    Some(Identity {
        pid,
        started: p.start_time(),
        executable: p.exe()?.into(),
    })
}
pub fn alive(i: &Identity) -> bool {
    identity(i.pid).is_some_and(|p| p.started == i.started && p.executable == i.executable)
}
pub fn signal_group(pid: u32, sig: i32) {
    unsafe {
        libc::kill(-(pid as i32), sig);
    }
}
// Execution parameters stay explicit at the two call sites.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    program: &str,
    args: &[String],
    cwd: &Path,
    timeout: u64,
    cancel: &CancellationToken,
    events: &mpsc::Sender<Event>,
    log: &str,
    journal: &Path,
) -> Outcome<()> {
    let code = run_command(
        program,
        args,
        cwd,
        &std::collections::BTreeMap::new(),
        timeout,
        cancel,
        events,
        log,
        journal,
    )
    .await?;
    if code == 0 {
        Ok(())
    } else {
        Err(Fault::new(
            "command_failed",
            format!("{program} exited {code}"),
        ))
    }
}
#[allow(clippy::too_many_arguments)]
pub async fn run_command(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &std::collections::BTreeMap<String, String>,
    timeout: u64,
    cancel: &CancellationToken,
    events: &mpsc::Sender<Event>,
    log: &str,
    journal: &Path,
) -> Outcome<i32> {
    run_command_until(
        program,
        args,
        cwd,
        env,
        tokio::time::Instant::now() + Duration::from_secs(timeout),
        cancel,
        events,
        log,
        journal,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub async fn run_command_until(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &std::collections::BTreeMap<String, String>,
    deadline: tokio::time::Instant,
    cancel: &CancellationToken,
    events: &mpsc::Sender<Event>,
    log: &str,
    journal: &Path,
) -> Outcome<i32> {
    let mut tree = Tree::new();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(env)
        .env(OWNER_ENV, tree.owner())
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd.process_group(0);
    wire::persist(
        journal,
        &serde_json::json!({"state":"spawning", "tree": &tree}),
    )
    .await
    .map_err(|e| Fault::new("recovery_required", e))?;
    if cancel.is_cancelled() {
        let _ = std::fs::remove_file(journal);
        return Err(Fault::new("cancelled", "cancelled before command dispatch"));
    }
    if tokio::time::Instant::now() >= deadline {
        let _ = std::fs::remove_file(journal);
        return Err(Fault::new(
            "timed_out",
            "command deadline exceeded before dispatch",
        ));
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Fault::new("invalid_config", format!("{program}: {e}")))?;
    let pid = child.id().unwrap();
    tree.add_root(pid);
    let root = identity(pid);
    tree.persist(journal, &root).await?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let mut pumps = tokio::task::JoinSet::new();
    pumps.spawn(pump(
        out,
        events.clone(),
        log.into(),
        "stdout",
        crate::metrics::current(),
    ));
    pumps.spawn(pump(
        err,
        events.clone(),
        log.into(),
        "stderr",
        crate::metrics::current(),
    ));
    let mut scan = tokio::time::interval(Duration::from_millis(20));
    scan.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        tokio::select! {
            status=child.wait()=>break status.map_err(|e|Fault::new("recovery_required",e)).map(|s| s.code().unwrap_or(128)),
            _=cancel.cancelled()=>break Err(Fault::new("cancelled","task cancelled")),
            _=tokio::time::sleep_until(deadline)=>break Err(Fault::new("timed_out",format!("{program} exceeded command deadline"))),
            _=scan.tick()=>{
                if tree.refresh_children() && let Err(error) = tree.persist(journal, &root).await {
                    break Err(error);
                }
            }
        }
    };
    // Collect cross-group children before terminating their parents. The marker
    // also finds fast-forked/reparented helpers that escaped the periodic scan.
    tree.refresh();
    let recorded = tree.persist(journal, &root).await;
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        let until = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            tree.signal(signal);
            let _ = child.try_wait();
            tree.refresh();
            if tree.is_empty() || tokio::time::Instant::now() >= until {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if tree.is_empty() {
            break;
        }
    }
    // Child owns the directly spawned PID even if process inspection failed.
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.start_kill();
    }
    let reaped = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    let drained = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(result) = pumps.join_next().await {
            result
                .map_err(|e| Fault::new("recovery_required", e))?
                .map_err(|e| Fault::new("recovery_required", e))?;
        }
        Ok::<_, Fault>(())
    })
    .await;
    // Dropping JoinHandles detaches their tasks. JoinSet instead aborts on Drop;
    // explicitly join aborted readers here so no sender outlives this command.
    pumps.shutdown().await;
    tree.refresh();
    if !tree.is_empty() || !matches!(reaped, Ok(Ok(_))) {
        tree.persist(journal, &root).await?;
        return Err(Fault::new(
            "recovery_required",
            "command descendants did not stop; process journal retained",
        ));
    }
    tree.disarm();
    recorded?;
    drained.map_err(|_| {
        Fault::new(
            "recovery_required",
            "command output did not close; log capture truncated, process journal retained",
        )
    })??;
    std::fs::remove_file(journal).map_err(|e| Fault::new("recovery_required", e))?;
    result
}

async fn pump<R: tokio::io::AsyncRead + Unpin>(
    mut r: R,
    tx: mpsc::Sender<Event>,
    name: String,
    counter: &str,
    capture: Option<crate::metrics::Capture>,
) -> std::io::Result<()> {
    let mut b = [0; 8192];
    let mut pending = Vec::new();
    loop {
        let n = r.read(&mut b).await?;
        if let Some(c) = &capture {
            c.add("bytes", counter, n as u64);
        }
        pending.extend_from_slice(&b[..n]);
        let mut text = String::new();
        loop {
            match std::str::from_utf8(&pending) {
                Ok(valid) => {
                    text.push_str(valid);
                    pending.clear();
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    text.push_str(std::str::from_utf8(&pending[..valid]).unwrap());
                    pending.drain(..valid);
                    if let Some(invalid) = e.error_len() {
                        text.push('\u{fffd}');
                        pending.drain(..invalid);
                    } else {
                        if n == 0 {
                            text.push('\u{fffd}');
                            pending.clear();
                        }
                        break;
                    }
                }
            }
        }
        if !text.is_empty()
            && tx
                .send(Event::Log {
                    name: name.clone(),
                    text,
                })
                .await
                .is_err()
        {
            break;
        }
        if n == 0 {
            break;
        }
    }
    Ok(())
}

pub fn recover(journal: &Path) -> Outcome<()> {
    if !journal.exists() {
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_slice(
        &std::fs::read(journal).map_err(|e| Fault::new("recovery_required", e))?,
    )
    .map_err(|e| Fault::new("recovery_required", e))?;
    if let Some(value) = v.get("tree") {
        let tree = Tree::recover(value)?;
        if !tree.is_empty() {
            tree.signal(libc::SIGKILL);
            return Err(Fault::new(
                "recovery_required",
                "descendant cleanup requested or ownership unresolved; journal retained for verification on restart",
            ));
        }
        std::fs::remove_file(journal).map_err(|e| Fault::new("recovery_required", e))?;
        return Ok(());
    }
    let Some(i) = v
        .get("identity")
        .and_then(|v| serde_json::from_value::<Identity>(v.clone()).ok())
    else {
        return Err(Fault::new(
            "recovery_required",
            "interrupted spawn has unknown identity; inspect processes and archive process.json before restart",
        ));
    };
    if alive(&i) {
        signal_group(i.pid, libc::SIGKILL);
        return Err(Fault::new(
            "recovery_required",
            "terminated recorded orphan; inspect and restart worker",
        ));
    }
    // The leader may have exited while descendants still own its process group.
    if unsafe { libc::kill(-(i.pid as i32), 0) } == 0 {
        return Err(Fault::new(
            "recovery_required",
            "recorded process group still exists; inspect before clearing the journal",
        ));
    }
    std::fs::remove_file(journal).map_err(|e| Fault::new("recovery_required", e))?;
    Ok(())
}
