use macrun::{
    config::WorkerConfig,
    engine::Engine,
    metrics::{self, Capture, Profile, Store},
    model, wire,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

fn profile(data: &Path, id: &str) -> Profile {
    Profile {
        id: id.into(),
        name: format!("Connection {id}"),
        data: data.into(),
    }
}
fn record(data: &Path, i: u128, start: u64, end: u64, status: &str, metric: Value) -> String {
    let id = uuid::Uuid::from_u128(i + 1).to_string();
    wire::atomic_json(&data.join("tasks").join(&id).join("result.json"),&json!({"task_id":id,"kind":"exec.start","status":status,"started_at":start,"ended_at":end,"metrics":metric,"metrics_version":"fixture","arguments":{"command":"private-command","env":{"TOKEN":"private-secret"}},"result":{"content":"private-output"}})).unwrap();
    id
}
fn observation(wall: f64, payload: u64) -> Value {
    let mut m = metrics::Metrics {
        wall_ms: wall,
        complete: true,
        ..Default::default()
    };
    m.bytes.insert("payload".into(), payload);
    m.project_id = Some("opaque-project".into());
    m.project_name = Some("example".into());
    json!(m)
}

#[tokio::test]
async fn all_retained_profiles_page_without_duplicates_and_merge_raw_percentiles() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let stores = [
        Store::open(a.path().into()).unwrap(),
        Store::open(b.path().into()).unwrap(),
    ];
    let now = model::now();
    for i in 0..260 {
        record(
            a.path(),
            i,
            now - i as u64,
            now,
            "succeeded",
            observation(10.0, 10),
        );
    }
    record(b.path(), 0, now, now, "succeeded", observation(999.0, 20));
    let profiles = vec![profile(a.path(), "a"), profile(b.path(), "b")];
    let first = metrics::query(profiles.clone(), "metrics_dashboard", json!({"limit":50}))
        .await
        .unwrap();
    assert_eq!(first["total"], 261);
    assert_eq!(first["latency"]["p95_ms"], 10.0);
    assert_eq!(first["bytes"]["payload"], 2620);
    assert!(!first.to_string().contains("private-"));
    let mut seen = BTreeSet::new();
    let mut page = first;
    loop {
        for r in page["operations"].as_array().unwrap() {
            assert!(seen.insert(format!("{}:{}", r["connection_id"], r["task_id"])));
        }
        if page["next_cursor"].is_null() {
            break;
        }
        page = metrics::query(
            profiles.clone(),
            "metrics_operations",
            json!({"cursor":page["next_cursor"]}),
        )
        .await
        .unwrap();
    }
    assert_eq!(seen.len(), 261);
    let filtered=metrics::query(profiles,"metrics_summary",json!({"connection_id":"b","project_id":"opaque-project","kind":"exec.start","status":"succeeded"})).await.unwrap();
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["latency"]["p95_ms"], 999.0);
    drop(stores);
}

#[tokio::test]
async fn legacy_gaps_cross_boundary_failures_and_attempts_keep_correct_denominators() {
    let data = tempfile::tempdir().unwrap();
    let _store = Store::open(data.path().into()).unwrap();
    let now = model::now();
    // An operation accepted before the window contributes latency when it completes inside.
    record(
        data.path(),
        0,
        now - 2000,
        now - 100,
        "succeeded",
        observation(1900.0, 90),
    );
    let legacy = uuid::Uuid::from_u128(2).to_string();
    wire::atomic_json(&data.path().join("tasks").join(&legacy).join("result.json"),&json!({"task_id":legacy,"kind":"exec.start","status":"failed","started_at":now-500,"ended_at":now})).unwrap();
    record(
        data.path(),
        2,
        now - 500,
        now,
        "cancelled",
        observation(300.0, 5),
    );
    record(
        data.path(),
        3,
        now - 500,
        now,
        "unknown",
        observation(300.0, 7),
    );
    let p = vec![profile(data.path(), "a")];
    let v = metrics::query(
        p.clone(),
        "metrics_dashboard",
        json!({"from_ms":now-1000,"to_ms":now+1}),
    )
    .await
    .unwrap();
    assert_eq!(v["total"], 3);
    assert_eq!(v["measured"], 2);
    assert_eq!(v["success_denominator"], 2);
    assert_eq!(v["latency"]["n"], 1);
    assert_eq!(v["latency"]["p95_ms"], 1900.0);
    assert_eq!(v["bytes"]["payload"], 12);
    let empty = metrics::query(p.clone(), "metrics_series", json!({"kind":"sync"}))
        .await
        .unwrap();
    assert!(
        empty["series"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["p95_ms"].is_null())
    );
    for args in [
        json!({"from_ms":0,"to_ms":91*86400000u64}),
        json!({"task_id":"../secret"}),
        json!({"connection_id":"other"}),
        json!({"path":"/private"}),
    ] {
        let action = if args.get("task_id").is_some() {
            "metrics_detail"
        } else {
            "metrics_summary"
        };
        assert!(metrics::query(p.clone(), action, args).await.is_err());
    }
    assert!(
        metrics::query(p, "metrics_arbitrary", json!({}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn canonical_details_revisions_rebuild_permissions_and_exact_large_counters() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path().into()).unwrap();
    let now = model::now();
    let mut metric = observation(40.0, 9_007_199_254_740_993);
    metric["outputs"] = json!([{"name":"build/app","bytes":23,"status":"complete"}]);
    metric["samples"] = json!([{"offset_ms":1,"payload_bytes":42}]);
    let id = record(data.path(), 0, now, now, "succeeded", metric.clone());
    let p = vec![profile(data.path(), "a")];
    let first = metrics::query(p.clone(), "metrics_dashboard", json!({}))
        .await
        .unwrap();
    assert_eq!(first["bytes"]["payload"], "9007199254740993");
    assert!(
        first["operations"][0]["metrics"]["outputs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let full = metrics::query(
        p.clone(),
        "metrics_detail",
        json!({"connection_id":"a","task_id":id}),
    )
    .await
    .unwrap();
    assert_eq!(full["metrics"]["outputs"][0]["bytes"], 23);
    record(data.path(), 0, now, now, "failed", metric);
    assert_eq!(
        metrics::query(p.clone(), "metrics_summary", json!({}))
            .await
            .unwrap()["total"],
        1
    );
    drop(store);
    std::fs::write(data.path().join("metrics.sqlite"), b"broken-index").unwrap();
    let store = Store::open(data.path().into()).unwrap();
    assert!(data.path().join("metrics.corrupt").exists());
    let rebuilt = metrics::query(p.clone(), "metrics_summary", json!({}))
        .await
        .unwrap();
    assert_eq!(rebuilt["counts"]["failed"], 1);
    for suffix in ["", "-wal", "-shm"] {
        let path = data.path().join(format!("metrics.sqlite{suffix}"));
        if path.exists() {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    std::fs::remove_dir_all(data.path().join("tasks").join(&id)).unwrap();
    store.reconcile().unwrap();
    assert_eq!(
        metrics::query(p, "metrics_summary", json!({}))
            .await
            .unwrap()["total"],
        0
    );
}

#[tokio::test]
async fn export_streams_all_filtered_rows_atomically_and_excludes_sensitive_data() {
    let data = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let _store = Store::open(data.path().into()).unwrap();
    let now = model::now();
    for i in 0..160 {
        record(data.path(), i, now, now, "succeeded", observation(1.0, 1));
    }
    let mut p = profile(data.path(), "a");
    p.name = "=formula\"\nnew".into();
    let profiles = vec![p];
    let path = dest.path().join("metrics.json");
    let count = metrics::export(profiles.clone(), json!({}), "json".into(), path.clone())
        .await
        .unwrap();
    assert_eq!(count, 160);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("private-"));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["operations"].as_array().unwrap().len(), 160);
    assert_eq!(v["metadata"]["schema_version"], 1);
    metrics::export(profiles.clone(), json!({}), "csv".into(), path.clone())
        .await
        .unwrap();
    let csv = std::fs::read_to_string(&path).unwrap();
    assert!(csv.contains("\"'=formula\"\"\nnew\""));
    let before = csv;
    assert!(
        metrics::export(
            profiles.clone(),
            json!({"connection_id":"bad"}),
            "json".into(),
            path.clone()
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    assert!(
        metrics::export(
            profiles,
            json!({}),
            "json".into(),
            data.path().join("bad.json")
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 1);
}

#[test]
fn declared_outputs_bound_paths_globs_and_count_unique_files() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("build")).unwrap();
    std::fs::write(root.path().join("build/a.zip"), b"12345").unwrap();
    std::fs::hard_link(
        root.path().join("build/a.zip"),
        root.path().join("build/b.zip"),
    )
    .unwrap();
    std::fs::write(outside.path().join("secret"), b"secret").unwrap();
    symlink(outside.path(), root.path().join("link")).unwrap();
    let (out, total, files) =
        metrics::measure_output_totals(root.path(), &["build".into(), "build/*.zip".into()]);
    assert_eq!(out[0].bytes, Some(5));
    assert_eq!(out[1].bytes, Some(5));
    assert_eq!(total, Some(5));
    assert_eq!(files, 1);
    let bad = metrics::measure_outputs(
        root.path(),
        &[
            "../secret".into(),
            "/etc/passwd".into(),
            "link/secret".into(),
            "missing".into(),
            "[".into(),
        ],
    );
    assert_eq!(bad[0].status, "outside_workspace");
    assert_eq!(bad[1].status, "outside_workspace");
    assert_eq!(bad[2].status, "outside_workspace");
    assert_eq!(bad[3].status, "missing");
    assert_eq!(bad[4].status, "invalid_pattern");
    assert!(bad.iter().all(|o| o.bytes.is_none()));
}

#[tokio::test]
async fn actual_command_output_and_failures_are_persisted_and_report_is_profile_scoped() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path().into(), WorkerConfig::default()).unwrap();
    let r=engine.handle("exec.start",json!({"command":"printf 12345 > result.bin; printf abc; printf de >&2; exit 7","cwd":root.path(),"outputs":["result.bin"]})).await.unwrap();
    let id = r["task_id"].as_str().unwrap();
    let task = engine
        .handle("task.wait", json!({"task_id":id,"wait_seconds":10}))
        .await
        .unwrap();
    assert_eq!(task["status"], "failed");
    assert_eq!(task["metrics"]["bytes"]["stdout"], 3);
    assert_eq!(task["metrics"]["bytes"]["stderr"], 2);
    assert_eq!(task["metrics"]["bytes"]["declared_outputs"], 5);
    assert_eq!(task["metrics"]["files"]["declared_outputs"], 1);
    assert!(
        task["metrics"]["phases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "process_run")
    );
    let mut m = metrics::Metrics {
        origin: "server".into(),
        complete: true,
        ..Default::default()
    };
    assert!(
        engine
            .handle("performance.report", json!({"job_id":id,"metrics":m}))
            .await
            .is_err()
    );
    let job = model::id();
    let (sync, _) = engine
        .begin_external("sync", json!({"job_id":job,"remote_root":root.path()}))
        .await
        .unwrap();
    assert!(
        engine
            .handle("performance.report", json!({"job_id":job,"metrics":m}))
            .await
            .is_err()
    );
    engine
        .finish_external_metrics(&sync, None, None, Some(metrics::Metrics::default()))
        .await
        .unwrap();
    m.wall_ms = 21.0;
    engine
        .handle("performance.report", json!({"job_id":job,"metrics":m}))
        .await
        .unwrap();
    engine
        .handle("performance.report", json!({"job_id":job,"metrics":m}))
        .await
        .unwrap();
    let detail = metrics::query(
        vec![profile(data.path(), "a")],
        "metrics_detail",
        json!({"task_id":sync}),
    )
    .await
    .unwrap();
    assert_eq!(detail["server_metrics"]["wall_ms"], 21.0);
    m.origin = "worker".into();
    assert!(metrics::validate_report(&m).is_err());
    m.origin = "server".into();
    m.project_id = Some("arbitrary".into());
    assert!(metrics::validate_report(&m).is_err());
}

#[tokio::test]
async fn partial_io_monotonic_phases_and_saturating_counters_do_not_invent_bytes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let capture = Capture::new("worker");
    capture
        .scope(async {
            let phase = metrics::phase("copy_payload");
            let (mut write, read) = tokio::io::duplex(32);
            write.write_all(b"partial").await.unwrap();
            drop(write);
            let mut read = metrics::Metered::new(read, "payload");
            let mut buffer = [0; 15];
            assert!(read.read_exact(&mut buffer).await.is_err());
            drop(phase);
            wire::blocking(|| {
                let _phase = metrics::phase("hash");
                metrics::add("bytes", "hashed", 12);
            })
            .await
            .unwrap();
        })
        .await;
    let m = capture.snapshot(true);
    assert_eq!(m.bytes["payload"], 7);
    assert_eq!(m.bytes["hashed"], 12);
    assert_eq!(m.phases.len(), 2);
    assert!(m.wall_ms >= m.phases[0].wall_ms);
    assert_eq!(m.samples.last().unwrap().payload_bytes, 7);
    capture.add("bytes", "payload", u64::MAX);
    assert_eq!(capture.snapshot(true).bytes["payload"], u64::MAX);
}

#[tokio::test]
async fn children_and_artifact_attempts_do_not_inflate_operations_and_retention_removes_both() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path().into(), WorkerConfig::default()).unwrap();
    let now = model::now();
    let old = now - 100 * 86_400_000;
    let parent = record(
        data.path(),
        0,
        old,
        old + 1,
        "succeeded",
        observation(100.0, 0),
    );
    let path = data.path().join("tasks").join(&parent).join("result.json");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["kind"] = json!("desktop.sequence");
    v["metrics"]["bytes"]["artifacts"] = json!(500);
    wire::atomic_json(&path, &v).unwrap();
    let mut child = observation(20.0, 0);
    child["parent_task_id"] = json!(parent);
    child["bytes"]["artifacts"] = json!(500);
    record(data.path(), 1, old, old + 1, "succeeded", child);
    let mut m = metrics::Metrics {
        wall_ms: 10.0,
        complete: true,
        parent_task_id: Some(parent),
        ..Default::default()
    };
    m.bytes.insert("payload".into(), 50);
    engine
        .record_metrics_attempt("artifact.download", m, true)
        .await
        .unwrap();
    let p = vec![profile(data.path(), "a")];
    let past = metrics::query(
        p.clone(),
        "metrics_summary",
        json!({"from_ms":old,"to_ms":old+1000}),
    )
    .await
    .unwrap();
    assert_eq!(past["total"], 1);
    assert_eq!(past["bytes"]["artifacts"], 500);
    let current = metrics::query(p.clone(), "metrics_summary", json!({}))
        .await
        .unwrap();
    assert_eq!(current["total"], 0);
    assert_eq!(current["bytes"]["payload"], 50);
    assert_eq!(current["operations"].as_array().unwrap().len(), 1);
    let attempt = std::fs::read_dir(data.path().join("metrics-attempts"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut value: Value = serde_json::from_slice(&std::fs::read(&attempt).unwrap()).unwrap();
    value["ended_at"] = json!(old);
    value["started_at"] = json!(old);
    wire::atomic_json(&attempt, &value).unwrap();
    let pruned = engine.prune_history().await.unwrap();
    assert_eq!(pruned["removed"], 2);
    assert_eq!(pruned["metrics_attempts_removed"], 1);
    assert_eq!(
        metrics::query(p, "metrics_dashboard", json!({}))
            .await
            .unwrap()["operations"],
        json!([])
    );
    let mut samples =
        json!({"ended_at":old,"metrics":{"samples":[1]},"server_metrics":{"samples":[2]}});
    assert!(metrics::prune_samples(&mut samples, old + 1));
    assert!(!metrics::prune_samples(&mut samples, old + 1));
}

#[tokio::test]
async fn locked_index_degrades_visibly_but_canonical_results_survive_and_rebuild_repairs_it() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path().into()).unwrap();
    let db = rusqlite::Connection::open(data.path().join("metrics.sqlite")).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let now = model::now();
    let id = record(data.path(), 0, now, now, "succeeded", observation(10.0, 20));
    assert!(
        data.path()
            .join("tasks")
            .join(id)
            .join("result.json")
            .exists()
    );
    assert!(data.path().join("metrics.index-dirty").exists());
    let p = vec![profile(data.path(), "a")];
    let v = metrics::query(p.clone(), "metrics_dashboard", json!({}))
        .await
        .unwrap();
    assert_eq!(v["errors"].as_array().unwrap().len(), 1);
    db.execute_batch("ROLLBACK").unwrap();
    store.reconcile().unwrap();
    let v = metrics::query(p, "metrics_dashboard", json!({}))
        .await
        .unwrap();
    assert_eq!(v["total"], 1);
    assert_eq!(v["errors"], json!([]));
}

#[test]
fn newer_schemas_and_symlinked_indexes_are_rejected_without_overwriting_them() {
    let data = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(data.path().join("metrics.sqlite")).unwrap();
    db.execute_batch("PRAGMA user_version=2").unwrap();
    drop(db);
    assert!(Store::open(data.path().into()).is_err());
    let db = rusqlite::Connection::open(data.path().join("metrics.sqlite")).unwrap();
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        2
    );
    drop(db);
    let link = tempfile::tempdir().unwrap();
    symlink(
        data.path().join("metrics.sqlite"),
        link.path().join("metrics.sqlite"),
    )
    .unwrap();
    assert!(Store::open(link.path().into()).is_err());
}
