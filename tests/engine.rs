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
