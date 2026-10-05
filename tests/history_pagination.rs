use macrun::{config::WorkerConfig, engine::Engine, model, wire};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path, time::Instant};

fn record(data: &Path, i: u128, started: u64, status: &str) -> String {
    let id = uuid::Uuid::from_u128(i + 1).to_string();
    let task = json!({"task_id":id,"kind":if i == 1400 {"mcp.call"} else {"exec.start"},"status":status,"started_at":started,"ended_at":if matches!(status,"accepted"|"running"|"awaiting_approval") {Value::Null} else {json!(started+1)},"arguments":{"command":format!("archive command {i}"),"cwd":"/work/archive","remote_root":format!("/sync/{i}"),"path":format!("/files/{i}"),"env":{"TOKEN":"fixture-secret"}},"request_fingerprint":"fixture-fingerprint","result":{"exit_code":0,"result":{"content":[{"type":"image","mimeType":"image/png","data":"fixture-image"}]}}});
    wire::atomic_json(&data.join("tasks").join(&id).join("result.json"), &task).unwrap();
    std::fs::write(
        data.join("tasks").join(&id).join("output.log"),
        "historical-output",
    )
    .unwrap();
    id
}

#[tokio::test]
async fn large_history_pages_searches_and_counts_all_retained_records() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path().into(), WorkerConfig::default()).unwrap();
    let now = model::now();
    for i in 0..1500 {
        record(
            data.path(),
            i,
            now - (i / 3) as u64,
            if i % 7 == 0 { "failed" } else { "succeeded" },
        );
    }
    let start = Instant::now();
    let first = engine.local_task_list(json!({"limit":50})).await.unwrap();
    let cold = start.elapsed();
    assert_eq!(first["total"], 1500);
    assert_eq!(first["counts"]["failed"], 215);
    assert_eq!(first["tasks"].as_array().unwrap().len(), 50);
    let serialized = first.to_string();
    assert!(!serialized.contains("fixture-secret"));
    assert!(!serialized.contains("request_fingerprint"));
    assert!(!serialized.contains("fixture-image"));
    assert!(!serialized.contains("historical-output"));

    // A new newest task cannot shift the older page boundary or duplicate rows.
    record(data.path(), 2000, now + 1000, "succeeded");
    let mut seen: BTreeSet<String> = first["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["task_id"].as_str().unwrap().to_owned())
        .collect();
    let mut cursor = first["next_cursor"].clone();
    while !cursor.is_null() {
        let page = engine
            .local_task_list(json!({"limit":50,"cursor":cursor}))
            .await
            .unwrap();
        for task in page["tasks"].as_array().unwrap() {
            assert!(seen.insert(task["task_id"].as_str().unwrap().to_owned()));
        }
        cursor = page["next_cursor"].clone();
    }
    assert_eq!(seen.len(), 1500);
    let start = Instant::now();
    let found = engine
        .local_task_list(json!({"query":"/FILES/1499","status":"succeeded"}))
        .await
        .unwrap();
    let search = start.elapsed();
    assert_eq!(found["filtered_total"], 1);
    assert_eq!(
        found["tasks"][0]["task_id"],
        uuid::Uuid::from_u128(1500).to_string()
    );
    let failed = engine
        .local_task_list(json!({"query":"archive","status":"failed","limit":1000}))
        .await
        .unwrap();
    assert_eq!(failed["filtered_total"], 215);
    assert_eq!(failed["counts"]["succeeded"], 1286);
    assert_eq!(failed["tasks"].as_array().unwrap().len(), 100);
    let replay = engine
        .local_task_list(json!({"kind":"mcp.call","limit":8}))
        .await
        .unwrap();
    assert_eq!(replay["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(
        replay["tasks"][0]["task_id"],
        uuid::Uuid::from_u128(1401).to_string()
    );
    let start = Instant::now();
    let snapshot = engine.local_snapshot().await.unwrap();
    let snapshot_time = start.elapsed();
    assert_eq!(snapshot["today_summary"]["total"], 1501);
    assert_eq!(snapshot["tasks"].as_array().unwrap().len(), 200);
    assert!(!snapshot.to_string().contains("historical-output"));
    eprintln!(
        "history benchmark: 1500 rows; cold page {:?}, warm search {:?}, warm snapshot {:?}; page {} bytes, snapshot {} bytes",
        cold,
        search,
        snapshot_time,
        serialized.len(),
        snapshot.to_string().len()
    );
}

#[tokio::test]
async fn snapshot_keeps_old_active_rows_and_detail_reads_only_requested_output_tail() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path().into(), WorkerConfig::default()).unwrap();
    let now = model::now();
    for i in 0..230 {
        record(data.path(), i, now - i as u64, "succeeded");
    }
    let running = record(data.path(), 240, now - 240, "running");
    let waiting = record(data.path(), 241, now - 241, "awaiting_approval");
    let snapshot = engine.local_snapshot().await.unwrap();
    let rows = snapshot["tasks"].as_array().unwrap();
    assert_eq!(rows.len(), 202);
    assert!(rows.iter().any(|task| task["task_id"] == running));
    assert!(rows.iter().any(|task| task["task_id"] == waiting));
    assert_eq!(snapshot["task_counts"]["running"], 1);
    let old_id = uuid::Uuid::from_u128(230).to_string();
    let output = format!("{}TAIL-END", "a".repeat(100_000));
    std::fs::write(
        data.path().join("tasks").join(&old_id).join("output.log"),
        output,
    )
    .unwrap();
    let detail = engine
        .handle("task.get", json!({"task_id":old_id,"tail_bytes":8192}))
        .await
        .unwrap();
    assert_eq!(detail["output"]["text"].as_str().unwrap().len(), 8192);
    assert!(
        detail["output"]["text"]
            .as_str()
            .unwrap()
            .ends_with("TAIL-END")
    );
    // Updating a task invalidates its cached summary; pruning removes the row.
    record(data.path(), 240, now - 240, "failed");
    let updated = engine
        .local_task_list(json!({"query":running}))
        .await
        .unwrap();
    assert_eq!(updated["tasks"][0]["status"], "failed");
    assert!(
        engine
            .local_task_list(json!({"status":"unsupported"}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn history_pagination_is_available_over_the_private_control_socket() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path().into(), WorkerConfig::default()).unwrap();
    let task_id = record(data.path(), 0, model::now(), "failed");
    let path = data.path().join("tasks").join(&task_id).join("result.json");
    let mut failed: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    failed["error"] = json!({"code":"fixture_failed","message":"Expected test failure"});
    wire::atomic_json(&path, &failed).unwrap();
    let socket = data.path().join("ipc/control.sock");
    let (_sender, receiver) =
        tokio::sync::watch::channel(macrun::local::ConnectionState::initial("test".into()));
    let stop = tokio_util::sync::CancellationToken::new();
    let serving = tokio::spawn(macrun::local::serve(
        socket.clone(),
        engine,
        receiver,
        stop.clone(),
    ));
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let page = macrun::local::request(&socket, "task_list", json!({"limit":50}))
        .await
        .unwrap();
    assert_eq!(page["total"], 1);
    let detail = macrun::local::request(
        &socket,
        "task_detail",
        json!({"task_id":task_id,"tail_bytes":8192}),
    )
    .await
    .unwrap();
    assert_eq!(detail["status"], "failed");
    assert_eq!(detail["error"]["code"], "fixture_failed");
    assert_eq!(detail["output"]["text"], "historical-output");
    assert!(
        macrun::local::request(&socket, "task_detail", json!({"task_id":uuid::Uuid::nil()}))
            .await
            .is_err()
    );
    stop.cancel();
    serving.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_day_without_tasks_still_reports_zero_total() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path().into(), WorkerConfig::default()).unwrap();
    record(data.path(), 0, 1, "succeeded");
    let snapshot = engine.local_snapshot().await.unwrap();
    assert_eq!(snapshot["total_tasks"], 1);
    assert_eq!(snapshot["today_summary"]["total"], 0);
}

#[tokio::test]
async fn status_groups_cover_active_and_attention_records() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path().into(), WorkerConfig::default()).unwrap();
    for (i, status) in [
        "succeeded",
        "failed",
        "timed_out",
        "unknown",
        "denied",
        "cancelled",
    ]
    .into_iter()
    .enumerate()
    {
        record(data.path(), i as u128, 10 + i as u64, status);
    }
    let attention = engine
        .local_task_list(json!({"status":"attention"}))
        .await
        .unwrap();
    assert_eq!(attention["filtered_total"], 4);
    assert!(attention["tasks"].as_array().unwrap().iter().all(|t| {
        ["failed", "timed_out", "unknown", "denied"].contains(&t["status"].as_str().unwrap())
    }));
    // Counts stay per status so the UI can sum any group.
    assert_eq!(attention["counts"]["succeeded"], 1);
    // Startup recovery turns stale active records into unknown, so write an active one after opening.
    record(data.path(), 9, 99, "awaiting_approval");
    let active = engine
        .local_task_list(json!({"status":"active"}))
        .await
        .unwrap();
    assert_eq!(active["filtered_total"], 1);
    assert_eq!(active["tasks"][0]["status"], "awaiting_approval");
}

#[tokio::test]
async fn attention_leaves_out_commands_that_only_exited_non_zero() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path().into(), WorkerConfig::default()).unwrap();
    let now = model::now();
    let write = |i: u128, task: Value| {
        let id = uuid::Uuid::from_u128(i).to_string();
        let mut task = task;
        task["task_id"] = json!(id);
        task["kind"] = json!("exec.start");
        task["started_at"] = json!(now - i as u64);
        task["ended_at"] = json!(now - i as u64 + 1);
        task["arguments"] = json!({"command":"grep missing file","cwd":"/work"});
        wire::atomic_json(
            &data.path().join("tasks").join(&id).join("result.json"),
            &task,
        )
        .unwrap();
        id
    };
    let exited = write(1, json!({"status":"failed","result":{"exit_code":1}}));
    let broken = write(
        2,
        json!({"status":"failed","error":{"message":"spawn failed"}}),
    );
    let timed_out = write(
        3,
        json!({"status":"timed_out","error":{"message":"timed_out"}}),
    );
    write(4, json!({"status":"succeeded","result":{"exit_code":0}}));

    let attention = engine
        .local_task_list(json!({"status":"attention"}))
        .await
        .unwrap();
    let ids: BTreeSet<_> = attention["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["task_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, BTreeSet::from([broken.clone(), timed_out]));
    assert_eq!(attention["filtered_total"], 2);
    assert_eq!(attention["counts"]["failed"], 2);
    assert_eq!(attention["counts"]["exited"], 1);

    // The plain status filter still returns every failed record, and rows keep
    // the exit code so the client can label them without a detail read.
    let failed = engine
        .local_task_list(json!({"status":"failed"}))
        .await
        .unwrap();
    assert_eq!(failed["filtered_total"], 2);
    let row = failed["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["task_id"] == exited)
        .unwrap();
    assert_eq!(row["result"], json!({"exit_code":1}));

    let snapshot = engine.local_snapshot().await.unwrap();
    assert_eq!(snapshot["today_summary"]["failed"], 2);
    assert_eq!(snapshot["today_summary"]["exited"], 1);
}
