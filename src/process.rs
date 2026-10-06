use crate::{
    model::{Event, Fault, Outcome},
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
struct KillGroup(u32);
impl Drop for KillGroup {
    fn drop(&mut self) {
        signal_group(self.0, libc::SIGKILL);
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
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(env)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd.process_group(0);
    wire::persist(journal, &serde_json::json!({"state":"spawning"}))
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
    let guard = KillGroup(pid);
    wire::persist(
        journal,
        &serde_json::json!({"state":"running","identity":identity(pid),"pid":pid}),
    )
    .await
    .map_err(|e| Fault::new("recovery_required", e))?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let a = tokio::spawn(pump(
        out,
        events.clone(),
        log.into(),
        "stdout",
        crate::metrics::current(),
    ));
    let b = tokio::spawn(pump(
        err,
        events.clone(),
        log.into(),
        "stderr",
        crate::metrics::current(),
    ));
    let result = tokio::select! {
        status=child.wait()=>status.map_err(|e|Fault::new("recovery_required",e)).map(|s| s.code().unwrap_or(128)),
        _=cancel.cancelled()=>Err(Fault::new("cancelled","task cancelled")),
        _=tokio::time::sleep_until(deadline)=>Err(Fault::new("timed_out",format!("{program} exceeded command deadline"))),
    };
    if result
        .as_ref()
        .is_err_and(|e| matches!(e.code.as_str(), "cancelled" | "timed_out"))
    {
        signal_group(pid, libc::SIGTERM);
        if tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .is_err()
        {
            signal_group(pid, libc::SIGKILL);
            let _ = child.wait().await;
        }
    }
    // Also close lingering descendants/pipe writers after the parent exits.
    drop(guard);
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        let _ = a.await;
        let _ = b.await;
    })
    .await;
    std::fs::remove_file(journal).map_err(|e| Fault::new("recovery_required", e))?;
    result
}
async fn pump<R: tokio::io::AsyncRead + Unpin>(
    mut r: R,
    tx: mpsc::Sender<Event>,
    name: String,
    counter: &str,
    capture: Option<crate::metrics::Capture>,
) {
    let mut b = [0; 8192];
    let mut pending = Vec::new();
    loop {
        let n = r.read(&mut b).await.unwrap_or(0);
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
}

pub fn recover(journal: &Path) -> Outcome<()> {
    if !journal.exists() {
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_slice(
        &std::fs::read(journal).map_err(|e| Fault::new("recovery_required", e))?,
    )
    .map_err(|e| Fault::new("recovery_required", e))?;
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
