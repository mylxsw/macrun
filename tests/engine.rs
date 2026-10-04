use macrun::{config::WorkerConfig, engine::Engine, model::id};
use serde_json::{Value, json};
use std::time::Duration;
async fn wait(e: &std::sync::Arc<Engine>, id: &str) -> Value {
    for _ in 0..100 {
        let v = e.handle("task.get", json!({"task_id":id})).await.unwrap();
        if v.get("ended_at").is_some() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("task did not finish")
}
#[tokio::test]
async fn command_idempotency_exit_code_environment_and_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let e = Engine::open(dir.path().into(), WorkerConfig::default()).unwrap();
    let request_id = id();
    let a = json!({"request_id":request_id,"command":"printf '%s' \"$GREETING\"; echo once >> count; exit 7","cwd":dir.path(),"env":{"GREETING":"你好"}});
    let (x, y) = tokio::join!(
        e.handle("exec.start", a.clone()),
        e.handle("exec.start", a.clone())
    );
    assert!(x.is_ok() && y.is_ok());
    let v = wait(&e, &request_id).await;
    assert_eq!(v["status"], "failed");
    assert_eq!(v["result"]["exit_code"], 7);
    assert_eq!(v["output"]["text"], "你好");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("count")).unwrap(),
        "once\n"
    );
    let mut changed = a.clone();
    changed["command"] = json!("echo twice");
    assert!(e.handle("exec.start", changed).await.is_err());
    drop(e);
    let e = Engine::open(dir.path().into(), WorkerConfig::default()).unwrap();
    assert_eq!(e.handle("exec.start", a).await.unwrap()["duplicate"], true);
}
#[tokio::test]
async fn unfinished_tasks_become_unknown_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let task = id();
    let p = dir.path().join("tasks").join(&task).join("result.json");
    macrun::wire::atomic_json(&p, &json!({"task_id":task,"status":"running"})).unwrap();
    let e = Engine::open(dir.path().into(), WorkerConfig::default()).unwrap();
    assert_eq!(
        e.handle("task.get", json!({"task_id":task})).await.unwrap()["status"],
        "unknown"
    );
}
#[tokio::test]
async fn command_timeout_and_cancel_are_queryable() {
    let dir = tempfile::tempdir().unwrap();
    let e = Engine::open(dir.path().into(), WorkerConfig::default()).unwrap();
    for cancel in [false, true] {
        let task = id();
        e.handle(
            "exec.start",
            json!({"request_id":task,"command":"sleep 30","cwd":dir.path(),"timeout_seconds":1}),
        )
        .await
        .unwrap();
        if cancel {
            e.handle("task.cancel", json!({"task_id":task}))
                .await
                .unwrap();
        }
        assert_eq!(
            wait(&e, &task).await["status"],
            if cancel { "cancelled" } else { "timed_out" }
        );
    }
}

#[tokio::test]
async fn local_pause_preserves_running_tasks_and_deduplication() {
    let dir = tempfile::tempdir().unwrap();
    let e = Engine::open(dir.path().into(), WorkerConfig::default()).unwrap();
    let args = json!({"request_id":id(),"command":"sleep 0.2; echo completed","cwd":dir.path()});
    e.handle("exec.start", args.clone()).await.unwrap();
    e.set_policy(Some(true), None, false).await.unwrap();
    assert_eq!(
        e.handle("exec.start", args.clone()).await.unwrap()["duplicate"],
        true
    );
    let rejected = e
        .handle(
            "exec.start",
            json!({"command":"echo forbidden","cwd":dir.path()}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        rejected
            .downcast_ref::<macrun::model::Fault>()
            .unwrap()
            .code,
        "busy"
    );
    assert_eq!(
        wait(&e, args["request_id"].as_str().unwrap()).await["status"],
        "succeeded"
    );
    assert!(
        e.handle(
            "file.write",
            json!({"path":dir.path().join("no"),"data":"eA=="})
        )
        .await
        .is_err()
    );
    assert!(!dir.path().join("no").exists());
    assert!(e.begin_external("sync", json!({})).await.is_err());
    e.set_policy(Some(false), None, false).await.unwrap();
    assert!(e.handle("file.read",json!({"path":dir.path().join("tasks").join(args["request_id"].as_str().unwrap()).join("output.log"),"text":true})).await.is_ok());
}

#[tokio::test]
async fn emergency_stop_cancels_commands_and_sync_and_persists_policy() {
    let dir = tempfile::tempdir().unwrap();
    let e = Engine::open(dir.path().into(), WorkerConfig::default()).unwrap();
    let task = id();
    e.handle(
        "exec.start",
        json!({"request_id":task,"command":"sleep 30","cwd":dir.path()}),
    )
    .await
    .unwrap();
    let (sync, cancel) = e
        .begin_external("sync", json!({"remote_root":"/tmp/example"}))
        .await
        .unwrap();
    e.set_policy(None, None, true).await.unwrap();
    assert!(cancel.is_cancelled());
    e.finish_external(&sync, Some("cancelled".into()), None)
        .await
        .unwrap();
    assert_eq!(wait(&e, &task).await["status"], "cancelled");
    assert!(e.policy().await.paused);
    assert!(!e.policy().await.desktop_enabled);
    drop(e);
    let e = Engine::open(dir.path().into(), WorkerConfig::default()).unwrap();
    assert!(e.policy().await.paused);
    assert!(!e.policy().await.desktop_enabled);
}

#[tokio::test]
async fn environment_values_are_not_persisted_but_still_part_of_deduplication() {
    let dir = tempfile::tempdir().unwrap();
    let e = Engine::open(dir.path().into(), WorkerConfig::default()).unwrap();
    let task = id();
    let mut args = json!({"request_id":task,"command":"test -n \"$SECRET\"","cwd":dir.path(),"env":{"SECRET":"private-value-123"}});
    e.handle("exec.start", args.clone()).await.unwrap();
    let v = wait(&e, &task).await;
    assert_eq!(v["arguments"]["env"]["SECRET"], "••••");
    let raw =
        std::fs::read_to_string(dir.path().join("tasks").join(&task).join("result.json")).unwrap();
    assert!(!raw.contains("private-value-123"));
    assert!(
        !e.local_snapshot()
            .await
            .unwrap()
            .to_string()
            .contains("private-value-123")
    );
    args["env"]["SECRET"] = json!("different");
    assert!(e.handle("exec.start", args).await.is_err());
}

#[tokio::test]
async fn remote_cannot_change_local_controls() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = WorkerConfig::default();
    config.mcp.insert(
        "computer".into(),
        macrun::config::Backend {
            command: "unused".into(),
            args: vec![],
            env: Default::default(),
            cwd: None,
        },
    );
    let e = Engine::open(dir.path().into(), config).unwrap();
    e.set_policy(None, Some(false), false).await.unwrap();
    let err = e
        .handle(
            "mcp.call",
            json!({"server":"computer","tool":"click","session":"none"}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<macrun::model::Fault>().unwrap().code,
        "desktop_disabled"
    );
    assert!(e.handle("desktop", json!({"enabled":true})).await.is_err());
    assert!(e.handle("pause", json!({"paused":false})).await.is_err());
    let r = e
        .handle("exec.start", json!({"command":"true","cwd":dir.path()}))
        .await
        .unwrap();
    assert_eq!(
        wait(&e, r["task_id"].as_str().unwrap()).await["status"],
        "succeeded"
    );
}

#[tokio::test]
async fn paginated_tool_counts_accumulate_distinct_names_and_reset_with_discovery() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = WorkerConfig::default();
    config.mcp.insert(
        "fixture".into(),
        macrun::config::Backend {
            command: "python3".into(),
            args: vec![format!(
                "{}/tests/fixtures/mcp.py",
                env!("CARGO_MANIFEST_DIR")
            )],
            env: [("MACRUN_TEST_TOOLS_PAGINATED".into(), "1".into())].into(),
            cwd: None,
        },
    );
    let engine = Engine::open(dir.path().into(), config).unwrap();
    let first = engine
        .handle("mcp.tools", json!({"server":"fixture"}))
        .await
        .unwrap();
    assert_eq!(
        engine.local_snapshot().await.unwrap()["backends"][0]["tool_count"],
        2
    );
    let next = engine.handle("mcp.tools", json!({"server":"fixture","cursor":first["result"]["nextCursor"],"session":first["session"]})).await.unwrap();
    assert!(next["result"]["nextCursor"].is_null());
    assert_eq!(
        engine.local_snapshot().await.unwrap()["backends"][0]["tool_count"],
        3
    );
    engine
        .handle("mcp.tools", json!({"server":"fixture"}))
        .await
        .unwrap();
    assert_eq!(
        engine.local_snapshot().await.unwrap()["backends"][0]["tool_count"],
        2
    );
    engine.restart_backend("fixture").await.unwrap();
    assert!(engine.local_snapshot().await.unwrap()["backends"][0]["tool_count"].is_null());
    let restarted = engine
        .handle("mcp.tools", json!({"server":"fixture"}))
        .await
        .unwrap();
    assert_ne!(restarted["session"], first["session"]);
    assert_eq!(
        engine.local_snapshot().await.unwrap()["backends"][0]["tool_count"],
        2
    );
    engine.shutdown().await.unwrap();
}
