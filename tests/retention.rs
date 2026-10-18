use macrun::{
    config::WorkerConfig, engine::Engine, model::id, retention, safety::Safety, wire, workspace,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const DAY: u64 = 86_400_000;
const NOW: u64 = 100 * DAY;
const GIB: u64 = 1024 * 1024 * 1024;
fn task(data: &Path, end: u64, snapshot_bytes: u64, log_bytes: u64) -> PathBuf {
    let ident = id();
    let dir = data.join("tasks").join(&ident);
    wire::atomic_json(&dir.join("result.json"), &json!({"task_id":ident,"kind":"exec.start","status":"succeeded","started_at":end.saturating_sub(1),"ended_at":end,"request_fingerprint":"kept","result":{"exit_code":0,"workspace":dir.join("workspace"),"snapshot":true}})).unwrap();
    if snapshot_bytes > 0 {
        fs::create_dir_all(dir.join("workspace")).unwrap();
        fs::File::create(dir.join("workspace/build-output"))
            .unwrap()
            .set_len(snapshot_bytes)
            .unwrap();
    }
    fs::File::create(dir.join("output.log"))
        .unwrap()
        .set_len(log_bytes)
        .unwrap();
    dir
}
fn sweep(data: &Path, policy: &Safety) -> retention::TaskStorage {
    retention::task_storage(data, policy, &BTreeSet::new(), NOW).unwrap()
}
fn record(dir: &Path) -> Value {
    serde_json::from_slice(&fs::read(dir.join("result.json")).unwrap()).unwrap()
}

#[test]
fn legacy_policy_defaults_and_invalid_limits() {
    let mut policy: Safety = serde_json::from_value(json!({"retention_days":7})).unwrap();
    assert_eq!(policy.snapshot_retention_days, 1);
    assert_eq!(policy.storage_limit_gb, 10);
    policy.validate().unwrap();
    policy.snapshot_retention_days = 0;
    assert!(policy.validate().is_err());
    policy.snapshot_retention_days = 1;
    for value in [0, 2, u32::MAX] {
        policy.storage_limit_gb = value;
        assert!(policy.validate().is_err());
    }
}

#[test]
fn snapshots_expire_separately_without_following_links_or_deleting_sources() {
    let data = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("source"), "keep").unwrap();
    let old = task(data.path(), NOW - 2 * DAY, 1024, 32);
    let fresh = task(data.path(), NOW - DAY, 512, 32); // exact cutoff is retained
    std::os::unix::fs::symlink(outside.path(), old.join("workspace/link")).unwrap();
    let result = sweep(data.path(), &Safety::default());
    assert_eq!(result.workspaces_removed, 1);
    assert_eq!(result.removed, 0);
    assert_eq!(result.workspace_bytes, 512);
    assert!(result.reclaimed_bytes > 0);
    assert_eq!(record(&old)["workspace_expired"], true);
    assert!(old.join("output.log").exists());
    assert!(fresh.join("workspace").exists());
    assert_eq!(
        fs::read_to_string(outside.path().join("source")).unwrap(),
        "keep"
    );
    let again = sweep(data.path(), &Safety::default());
    assert_eq!(again.workspaces_removed, 0);
    assert_eq!(again.reclaimed_bytes, 0);
}

#[test]
fn budget_reclaims_oldest_snapshots_before_results() {
    let data = tempfile::tempdir().unwrap();
    let oldest = task(data.path(), NOW - 3 * DAY, GIB, 32);
    let next = task(data.path(), NOW - 2 * DAY, GIB / 2, 32);
    let policy = Safety {
        storage_limit_gb: 1,
        snapshot_retention_days: 30,
        ..Default::default()
    };
    let result = sweep(data.path(), &policy);
    assert_eq!(result.workspaces_removed, 1);
    assert_eq!(result.removed, 0);
    assert!(!oldest.join("workspace").exists());
    assert!(next.join("workspace").exists());
    assert!(oldest.join("result.json").exists());
    assert!(!result.over_limit);
    assert!(result.task_bytes < GIB);
}

#[test]
fn budget_evicts_oldest_results_but_preserves_dedup_and_fresh_data() {
    let data = tempfile::tempdir().unwrap();
    let old = task(data.path(), NOW - 2 * DAY, 0, GIB);
    let fresh = task(data.path(), NOW - 1, GIB, 32);
    let policy = Safety {
        storage_limit_gb: 1,
        ..Default::default()
    };
    let result = sweep(data.path(), &policy);
    assert_eq!(result.removed, 1);
    assert_eq!(result.workspaces_removed, 0);
    assert!(!old.exists());
    assert!(fresh.join("workspace").exists());
    assert!(result.over_limit); // protected fresh data may exceed the soft budget
    let tombstone: Value = serde_json::from_slice(
        &fs::read(data.path().join("dedup").join(format!(
            "{}.json",
            old.file_name().unwrap().to_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(tombstone["history_expired"], true);
    assert_eq!(tombstone["request_fingerprint"], "kept");
    assert!(tombstone.get("result").is_none());
}

#[test]
fn active_leased_unfinished_and_recovery_tasks_are_never_removed() {
    let data = tempfile::tempdir().unwrap();
    let active = task(data.path(), 1, 1024, 32);
    let leased = task(data.path(), 1, 1024, 32);
    let recovery = task(data.path(), 1, 1024, 32);
    let unfinished = task(data.path(), 1, 1024, 32);
    let future = task(data.path(), NOW + 1, 1024, 32);
    let mut unfinished_record = record(&unfinished);
    unfinished_record["status"] = json!("running");
    wire::atomic_json(&unfinished.join("result.json"), &unfinished_record).unwrap();
    fs::write(recovery.join("process.json"), "{}").unwrap();
    let held = workspace::Lease::acquire(&[leased.join("workspace/build-output")]).unwrap();
    let ids = BTreeSet::from([active.file_name().unwrap().to_str().unwrap().to_owned()]);
    let result = retention::task_storage(data.path(), &Safety::default(), &ids, NOW).unwrap();
    assert_eq!(result.removed, 0);
    assert_eq!(result.workspaces_removed, 0);
    for path in [&active, &leased, &recovery, &unfinished, &future] {
        assert!(path.join("workspace").exists());
    }
    drop(held);
    assert_eq!(sweep(data.path(), &Safety::default()).removed, 2);
}

#[test]
fn unowned_malformed_and_symlinked_directories_fail_closed() {
    let data = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let unowned = data.path().join("tasks/manual-project");
    fs::create_dir_all(&unowned).unwrap();
    fs::write(unowned.join("source"), "keep").unwrap();
    let mismatch = task(data.path(), 1, 8, 0);
    let mut value = record(&mismatch);
    value["task_id"] = json!(id());
    wire::atomic_json(&mismatch.join("result.json"), &value).unwrap();
    let corrupt = task(data.path(), 1, 8, 0);
    fs::write(corrupt.join("result.json"), "broken").unwrap();
    let linked_record = task(data.path(), 1, 8, 0);
    fs::remove_file(linked_record.join("result.json")).unwrap();
    fs::write(external.path().join("result"), "{}").unwrap();
    std::os::unix::fs::symlink(
        external.path().join("result"),
        linked_record.join("result.json"),
    )
    .unwrap();
    std::os::unix::fs::symlink(external.path(), data.path().join("tasks").join(id())).unwrap();
    let linked_workspace = task(data.path(), NOW - 2 * DAY, 0, 0);
    std::os::unix::fs::symlink(external.path(), linked_workspace.join("workspace")).unwrap();
    let result = sweep(data.path(), &Safety::default());
    assert_eq!(result.removed, 0);
    assert_eq!(result.workspaces_removed, 0);
    assert_eq!(result.errors, 3);
    assert!(unowned.join("source").exists());
    assert!(mismatch.exists() && corrupt.exists() && linked_record.exists());
    assert!(external.path().join("result").exists());
    // Replacing the entire tasks root with a symlink must also fail closed.
    fs::remove_dir_all(data.path().join("tasks")).unwrap();
    std::os::unix::fs::symlink(external.path(), data.path().join("tasks")).unwrap();
    assert!(
        retention::task_storage(data.path(), &Safety::default(), &BTreeSet::new(), NOW).is_err()
    );
}

#[test]
fn failed_dedup_write_keeps_task_and_reports_failure() {
    let data = tempfile::tempdir().unwrap();
    let old = task(data.path(), 1, 0, 1024);
    fs::write(data.path().join("dedup"), "blocked").unwrap();
    let result = sweep(data.path(), &Safety::default());
    assert_eq!(result.errors, 1);
    assert_eq!(result.removed, 0);
    assert!(old.join("result.json").exists());
}

#[tokio::test]
async fn completion_sweeps_expired_data_and_restart_preserves_deduplication() {
    let data = tempfile::tempdir().unwrap();
    let expired = task(data.path(), 1, 1024, 32);
    let engine = Engine::open(data.path().into(), WorkerConfig::default()).unwrap();
    let args = json!({"request_id":id(),"cwd":data.path(),"command":"echo once >> count"});
    let submitted = engine.handle("exec.start", args.clone()).await.unwrap();
    // shutdown waits for the completion cleanup as well as execution.
    let ident = submitted["task_id"].as_str().unwrap();
    let mut changes = engine.subscribe();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while engine.active_count().await > 0 {
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(!expired.exists());
    assert!(
        engine.local_snapshot().await.unwrap()["storage"]["scanned_at"]
            .as_u64()
            .unwrap()
            > 0
    );
    let dir = data.path().join("tasks").join(ident);
    let mut value = record(&dir);
    value["ended_at"] = json!(1);
    wire::atomic_json(&dir.join("result.json"), &value).unwrap();
    assert_eq!(engine.prune_history().await.unwrap()["removed"], 1);
    engine.shutdown().await.unwrap();
    drop(engine);
    let engine = Engine::open(data.path().into(), WorkerConfig::default()).unwrap();
    assert_eq!(
        engine.handle("exec.start", args).await.unwrap()["duplicate"],
        true
    );
    assert_eq!(
        engine
            .handle("task.get", json!({"task_id":ident}))
            .await
            .unwrap()["history_expired"],
        true
    );
    assert_eq!(
        fs::read_to_string(data.path().join("count")).unwrap(),
        "once\n"
    );
    engine.shutdown().await.unwrap();
}

#[test]
fn interrupted_directory_deletion_retries_from_its_dedup_record() {
    let data = tempfile::tempdir().unwrap();
    let dir = task(data.path(), 1, 0, 128);
    let mut tombstone = record(&dir);
    tombstone["history_expired"] = json!(true);
    let ident = dir.file_name().unwrap().to_str().unwrap();
    wire::atomic_json(
        &data.path().join("dedup").join(format!("{ident}.json")),
        &tombstone,
    )
    .unwrap();
    fs::remove_file(dir.join("result.json")).unwrap();
    let result = sweep(data.path(), &Safety::default());
    assert_eq!(result.removed, 1);
    assert_eq!(result.errors, 0);
    assert!(!dir.exists());
}

#[test]
fn read_pins_allow_normal_writers_but_delay_cleanup() {
    let data = tempfile::tempdir().unwrap();
    let dir = task(data.path(), 1, 512, 32);
    let pin = workspace::Lease::pin(&[dir.join("workspace/build-output")]).unwrap();
    let writer = workspace::Lease::acquire(&[dir.join("workspace")]).unwrap();
    drop(writer);
    assert_eq!(sweep(data.path(), &Safety::default()).removed, 0);
    drop(pin);
    assert_eq!(sweep(data.path(), &Safety::default()).removed, 1);
}

#[test]
fn resumable_transfer_ownership_survives_until_partials_are_old_enough() {
    let data = tempfile::tempdir().unwrap();
    let destination = data.path().join("download");
    let dir = task(data.path(), 1, 0, 32);
    let mut value = record(&dir);
    value["kind"] = json!("file.upload");
    wire::atomic_json(&dir.join("result.json"), &value).unwrap();
    let transfer_id = id();
    let part = data
        .path()
        .join(format!("download.{transfer_id}.macrun-part"));
    let meta = part.with_extension("macrun-meta");
    fs::write(&part, "partial").unwrap();
    wire::atomic_json(&meta, &json!({"path":destination})).unwrap();
    wire::atomic_json(
        &dir.join("transfer.json"),
        &json!({"destination":destination,"transfer_id":transfer_id}),
    )
    .unwrap();
    let recent = std::time::UNIX_EPOCH + std::time::Duration::from_millis(NOW);
    for path in [&part, &meta] {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(recent))
            .unwrap();
    }
    assert_eq!(sweep(data.path(), &Safety::default()).removed, 0);
    assert!(dir.join("transfer.json").exists());
    let result = retention::task_storage(
        data.path(),
        &Safety::default(),
        &BTreeSet::new(),
        NOW + 2 * DAY,
    )
    .unwrap();
    assert_eq!(result.transfers_removed, 1);
    assert_eq!(result.removed, 1);
    assert!(!dir.exists() && !part.exists() && !meta.exists());
}
