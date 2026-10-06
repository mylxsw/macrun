use macrun::{config::WorkerConfig, engine::Engine, safety::Safety};
use serde_json::json;

async fn wait_for_status(e: &std::sync::Arc<Engine>, id: &str, expected: &str) {
    let mut changes = e.subscribe();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let task = e.handle("task.get", json!({"task_id":id})).await.unwrap();
            if task["status"] == expected {
                return;
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("task {id} did not reach {expected}"));
}

#[test]
fn allowlist_checks_nonexistent_paths_symlinks_and_parent_traversal() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut p = Safety {
        restrict_paths: true,
        roots: vec![d.path().into()],
        ..Default::default()
    };
    p.validate().unwrap();
    assert!(p.check(&d.path().join("new/sub/file")).is_ok());
    assert!(p.check(&d.path().join("../escape")).is_err());
    assert!(p.check(outside.path()).is_err());
    std::os::unix::fs::symlink(outside.path(), d.path().join("link")).unwrap();
    assert!(p.check(&d.path().join("link/new")).is_err());
}
#[tokio::test]
async fn approvals_block_execution_and_rejection_is_durable() {
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().into(), WorkerConfig::default()).unwrap();
    e.set_safety(Safety {
        approval: "all".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    let a = json!({"request_id":macrun::model::id(),"cwd":d.path(),"command":"touch ran"});
    let v = e.handle("exec.start", a.clone()).await.unwrap();
    let id = v["task_id"].as_str().unwrap();
    wait_for_status(&e, id, "awaiting_approval").await;
    assert!(!d.path().join("ran").exists());
    e.approve(id, false).await.unwrap();
    wait_for_status(&e, id, "denied").await;
    assert!(e.approve(id, true).await.is_err());
    assert_eq!(e.handle("exec.start", a).await.unwrap()["duplicate"], true);
    assert!(!d.path().join("ran").exists());
}
#[tokio::test]
async fn approval_allows_once_and_stop_cancels_waiting() {
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().into(), WorkerConfig::default()).unwrap();
    e.set_safety(Safety {
        approval: "all".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    let v = e
        .handle("exec.start", json!({"cwd":d.path(),"command":"touch ran"}))
        .await
        .unwrap();
    e.approve(v["task_id"].as_str().unwrap(), true)
        .await
        .unwrap();
    for _ in 0..30 {
        if d.path().join("ran").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(d.path().join("ran").exists());
    let v = e
        .handle(
            "exec.start",
            json!({"cwd":d.path(),"command":"touch forbidden"}),
        )
        .await
        .unwrap();
    e.set_policy(None, None, true).await.unwrap();
    e.shutdown().await.unwrap();
    assert_eq!(
        e.handle("task.get", json!({"task_id":v["task_id"]}))
            .await
            .unwrap()["status"],
        "cancelled"
    );
    assert!(!d.path().join("forbidden").exists());
}
#[tokio::test]
async fn pruning_preserves_dedup_but_removes_output() {
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().into(), WorkerConfig::default()).unwrap();
    let args = json!({"request_id":macrun::model::id(),"cwd":d.path(),"command":"echo secret"});
    let v = e.handle("exec.start", args.clone()).await.unwrap();
    let id = v["task_id"].as_str().unwrap();
    e.shutdown().await.unwrap();
    let p = d.path().join("tasks").join(id).join("result.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    record["ended_at"] = json!(1);
    macrun::wire::atomic_json(&p, &record).unwrap();
    assert_eq!(e.prune_history().await.unwrap()["removed"], 1);
    assert!(!p.exists());
    assert_eq!(
        e.handle("exec.start", args).await.unwrap()["duplicate"],
        true
    );
    assert_eq!(
        e.handle("task.get", json!({"task_id":id})).await.unwrap()["history_expired"],
        true
    );
    let cancellation = e
        .handle("task.cancel", json!({"task_id":id}))
        .await
        .unwrap();
    assert_eq!(cancellation["cancel_requested"], false);
    assert_eq!(cancellation["status"], record["status"]);
}

#[tokio::test]
async fn task_reads_remain_available_during_retention_cleanup() {
    let d = tempfile::tempdir().unwrap();
    let ident = macrun::model::id();
    let dir = d.path().join("tasks").join(&ident);
    std::fs::create_dir_all(&dir).unwrap();
    let record = json!({"task_id":ident,"kind":"exec.start","status":"succeeded","started_at":1,"ended_at":2});
    macrun::wire::atomic_json(&dir.join("result.json"), &record).unwrap();
    std::fs::write(dir.join("output.log"), "completed output").unwrap();
    let e = Engine::open(d.path().into(), WorkerConfig::default()).unwrap();
    // task.get yields on async I/O, allowing cleanup to run concurrently.
    let (read, prune) = tokio::join!(
        e.handle("task.get", json!({"task_id":ident})),
        e.prune_history()
    );
    let read = read.unwrap();
    assert_eq!(read["status"], "succeeded");
    assert_eq!(prune.unwrap()["removed"], 1);
    assert_eq!(
        e.handle("task.get", json!({"task_id":ident}))
            .await
            .unwrap()["history_expired"],
        true
    );
}
#[tokio::test(start_paused = true)]
async fn approval_expires_after_sixty_seconds_without_execution() {
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().into(), WorkerConfig::default()).unwrap();
    e.set_safety(Safety {
        approval: "all".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    let v = e
        .handle(
            "exec.start",
            json!({"cwd":d.path(),"command":"touch forbidden"}),
        )
        .await
        .unwrap();
    let mut changes = e.subscribe();
    while e
        .handle("task.get", json!({"task_id":v["task_id"]}))
        .await
        .unwrap()["status"]
        != "awaiting_approval"
    {
        changes.changed().await.unwrap();
    }
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    loop {
        if e.handle("task.get", json!({"task_id":v["task_id"]}))
            .await
            .unwrap()["status"]
            == "denied"
        {
            break;
        }
        changes.changed().await.unwrap();
    }
    assert!(!d.path().join("forbidden").exists());
}
#[test]
fn risk_mode_requires_confirmation_for_unknown_programs_and_shell_operators() {
    for command in [
        "/bin/rm file",
        "python3 script.py",
        "env sh run",
        "git reset --hard",
        "echo $(touch file)",
        "ls; rm file",
    ] {
        assert!(macrun::safety::risky(command), "{command}");
    }
    assert!(!macrun::safety::risky("pwd"));
    assert!(!macrun::safety::risky("ls -la"));
}
#[tokio::test]
async fn restart_rejects_unanswered_approval_without_claiming_side_effects() {
    let d = tempfile::tempdir().unwrap();
    let id = macrun::model::id();
    macrun::wire::atomic_json(&d.path().join("tasks").join(&id).join("result.json"),&json!({"task_id":id,"status":"awaiting_approval","arguments":{"command":"touch forbidden","cwd":d.path()}})).unwrap();
    let e = Engine::open(d.path().into(), WorkerConfig::default()).unwrap();
    assert_eq!(
        e.handle("task.get", json!({"task_id":id})).await.unwrap()["status"],
        "denied"
    );
}
#[tokio::test]
async fn local_input_yield_rejects_new_desktop_calls() {
    let d = tempfile::tempdir().unwrap();
    let config: WorkerConfig = toml::from_str("[mcp.fixture]\ncommand='unused'\n").unwrap();
    let e = Engine::open(d.path().into(), config).unwrap();
    e.yield_desktop().await.unwrap();
    let err = e
        .handle(
            "mcp.call",
            json!({"server":"fixture","tool":"observe","session":"not-used"}),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("busy"));
    assert_eq!(e.local_snapshot().await.unwrap()["total_tasks"], 0);
}
