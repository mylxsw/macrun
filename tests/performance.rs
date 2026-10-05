use macrun::{
    artifact,
    config::WorkerConfig,
    engine::Engine,
    model::{Reply, id},
    sync, transfer, wire, workspace,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn cached_hash_detects_replacement_and_restored_mtime() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a");
    fs::write(&p, "aaaa").unwrap();
    let modified = fs::metadata(&p).unwrap().modified().unwrap();
    let first = sync::cached_hash(&p, false).unwrap();
    assert_eq!(sync::cached_hash(&p, false).unwrap(), first);
    fs::write(&p, "bbbb").unwrap();
    fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let second = sync::cached_hash(&p, false).unwrap();
    assert_ne!(first, second);
    assert_eq!(second, sync::cached_hash(&p, true).unwrap());
    let replacement = d.path().join("b");
    fs::write(&replacement, "cccc").unwrap();
    fs::rename(replacement, &p).unwrap();
    assert_ne!(second, sync::cached_hash(&p, false).unwrap());
    assert!(sync::cached_hash(&d.path().join("missing"), false).is_err());
}

#[test]
fn leases_resolve_aliases_overlap_and_release() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    fs::create_dir(&root).unwrap();
    let alias = d.path().join("alias");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    let lease = workspace::Lease::acquire(std::slice::from_ref(&root)).unwrap();
    assert!(workspace::Lease::acquire(&[alias.join("future")]).is_err());
    assert!(workspace::Lease::acquire(&[d.path().into()]).is_err());
    assert!(workspace::Lease::acquire(&[d.path().join("other")]).is_ok());
    assert_eq!(
        workspace::resolve(&PathBuf::from(".")).unwrap(),
        fs::canonicalize(std::env::current_dir().unwrap()).unwrap()
    );
    assert_eq!(
        workspace::resolve(&root.join("../outside")).unwrap(),
        fs::canonicalize(d.path()).unwrap().join("outside")
    );
    drop(lease);
    assert!(workspace::Lease::acquire(&[alias]).is_ok());
}

#[test]
fn artifacts_are_compact_scoped_validated_and_legacy_readable() {
    let d = tempfile::tempdir().unwrap();
    let original = json!({"content":[{"type":"image","mimeType":"image/png","data":"aGVsbG8="},{"type":"text","text":"caption"}],"other":{"nested":[]}});
    let mut result = original.clone();
    let refs = artifact::store(d.path(), &mut result).unwrap();
    assert_eq!(refs.len(), 1);
    assert!(result["content"][0].get("data").is_none());
    let stored = result.clone();
    artifact::hydrate(d.path(), &mut result).unwrap();
    assert_eq!(result, original);
    assert!(artifact::path(d.path(), "../secret").is_err());
    let mut invalid = json!({"type":"image","data":"bad!"});
    assert!(artifact::store(d.path(), &mut invalid).is_err());
    fs::write(
        artifact::path(d.path(), refs[0]["id"].as_str().unwrap()).unwrap(),
        "changed",
    )
    .unwrap();
    let mut result = stored;
    assert!(artifact::hydrate(d.path(), &mut result).is_err());
}

#[tokio::test]
async fn manifest_pages_exceed_old_frame_limit_and_reject_duplicates() {
    let mut manifest = sync::Manifest::default();
    for i in 0..110_000 {
        manifest.entries.insert(
            format!("long/project/path/to/a/source/file/{i:06}.txt"),
            sync::Entry::File {
                size: 1,
                hash: "f".repeat(64),
                executable: false,
            },
        );
    }
    manifest.skipped.push("outside".into());
    assert!(serde_json::to_vec(&manifest).unwrap().len() > wire::MAX_FRAME);
    let (mut a, mut b) = tokio::io::duplex(65536);
    let (sent, received) = tokio::join!(
        sync::send_manifest(&mut a, &manifest),
        sync::recv_manifest(&mut b)
    );
    sent.unwrap();
    assert_eq!(received.unwrap(), manifest);
    let (mut a, mut b) = tokio::io::duplex(65536);
    let page = macrun::model::Event::ManifestPage {
        entries: std::collections::BTreeMap::from([("a".into(), sync::Entry::Directory)]),
        skipped: vec![],
        last: false,
    };
    wire::send(&mut a, &page).await.unwrap();
    wire::send(&mut a, &page).await.unwrap();
    assert!(sync::recv_manifest(&mut b).await.is_err());
}

#[tokio::test]
async fn wait_preserves_dedup_and_generation_blocks_stale_builds() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("work");
    fs::create_dir_all(root.join(".macrun")).unwrap();
    fs::write(root.join(".macrun/generation"), "v1").unwrap();
    let engine = Engine::open(d.path().join("state"), WorkerConfig::default()).unwrap();
    let request = id();
    let args = json!({"cwd":root,"workspace_root":root,"generation":"v1","command":"echo once >> count","request_id":request,"wait_ms":1000});
    let value = engine.handle("exec.start", args.clone()).await.unwrap();
    assert_eq!(value["status"], "succeeded");
    assert_eq!(
        engine.handle("exec.start", args).await.unwrap()["status"],
        "succeeded"
    );
    assert_eq!(fs::read_to_string(root.join("count")).unwrap(), "once\n");
    workspace::invalidate(&root).unwrap();
    workspace::invalidate(&root).unwrap();
    let failed=engine.handle("exec.start",json!({"cwd":root,"workspace_root":root,"generation":"v1","command":"touch stale","wait_ms":1000})).await.unwrap();
    assert_eq!(failed["status"], "failed");
    assert!(!root.join("stale").exists());
    let task = engine
        .handle(
            "exec.start",
            json!({"cwd":root,"command":"sleep 0.15; echo waited","wait_ms":1}),
        )
        .await
        .unwrap();
    let done = engine
        .handle(
            "task.wait",
            json!({"task_id":task["task_id"],"wait_ms":1000,"include_result":false}),
        )
        .await
        .unwrap();
    assert_eq!(done["status"], "succeeded");
    assert!(done.get("result").is_none());
}

async fn upload_attempt(
    engine: Arc<Engine>,
    args: Value,
    bytes: Vec<u8>,
    complete: bool,
) -> (Value, Option<Value>) {
    let (mut client, server) = tokio::io::duplex(32768);
    let worker = tokio::spawn(async move {
        let (mut input, mut out) = tokio::io::split(server);
        let _ = transfer::serve("file.upload", &args, &mut input, &mut out, &engine).await;
    });
    let ready = transfer::response(&mut client).await.unwrap();
    client.write_all(&bytes).await.unwrap();
    let reply = if complete {
        Some(transfer::response(&mut client).await.unwrap())
    } else {
        client.shutdown().await.unwrap();
        None
    };
    drop(client);
    worker.await.unwrap();
    (ready, reply)
}

#[tokio::test]
async fn upload_resumes_checksums_and_records_one_task_per_attempt() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("out.bin");
    let engine = Engine::open(d.path().join("state"), WorkerConfig::default()).unwrap();
    let data = vec![42u8; 1_400_000];
    let hash = blake3::hash(&data).to_hex().to_string();
    let offset = 400_000;
    let args = json!({"path":path,"size":data.len(),"hash":hash,"transfer_id":id()});
    let (first, _) =
        upload_attempt(engine.clone(), args.clone(), data[..offset].to_vec(), false).await;
    assert_eq!(first["offset"], 0);
    assert!(!path.exists());
    let (resumed, end) = upload_attempt(engine.clone(), args, data[offset..].to_vec(), true).await;
    assert_eq!(resumed["offset"], offset);
    assert_eq!(end.unwrap()["hash"], hash);
    assert_eq!(fs::read(&path).unwrap(), data);
    assert_eq!(engine.active_count().await, 0);
    assert_eq!(
        fs::read_dir(d.path().join("state/tasks")).unwrap().count(),
        2
    );
    assert!(fs::read_dir(d.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("macrun-part")
    }));
}

#[tokio::test]
async fn binary_download_ranges_and_copy_failures() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("input");
    fs::write(&path, b"abcdef").unwrap();
    let engine = Engine::open(d.path().join("state"), WorkerConfig::default()).unwrap();
    let (mut client, server) = tokio::io::duplex(65536);
    let worker = tokio::spawn(async move {
        let (mut input, mut out) = tokio::io::split(server);
        transfer::serve(
            "file.download",
            &json!({"path":path,"offset":2}),
            &mut input,
            &mut out,
            &engine,
        )
        .await
        .unwrap();
    });
    assert_eq!(transfer::response(&mut client).await.unwrap()["size"], 4);
    let mut data = [0; 4];
    client.read_exact(&mut data).await.unwrap();
    assert_eq!(&data, b"cdef");
    assert_eq!(
        transfer::response(&mut client).await.unwrap()["hash"],
        blake3::hash(b"cdef").to_hex().as_str()
    );
    worker.await.unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    assert!(
        transfer::copy(&mut tokio::io::empty(), &mut tokio::io::sink(), 10, &cancel)
            .await
            .is_err()
    );
    assert!(
        transfer::copy(
            &mut tokio::io::empty(),
            &mut tokio::io::sink(),
            10,
            &tokio_util::sync::CancellationToken::new()
        )
        .await
        .is_err()
    );
    let (mut a, mut b) = tokio::io::duplex(4096);
    wire::send(
        &mut a,
        &Reply::Accepted {
            job_id: id(),
            directory: d.path().into(),
        },
    )
    .await
    .unwrap();
    assert!(transfer::response(&mut b).await.is_err());
}

#[tokio::test]
async fn native_watcher_detects_creation_and_retains_pending_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (_watcher, mut events) = macrun::watch::source(dir.path()).unwrap();
    fs::write(dir.path().join("new.txt"), "first").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), events.changed())
        .await
        .unwrap()
        .unwrap();
    let before = *events.borrow_and_update();
    fs::write(dir.path().join("new.txt"), "second").unwrap();
    // An event remains pending even while a sync consumer is doing other work.
    tokio::time::timeout(std::time::Duration::from_secs(10), events.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(*events.borrow() > before);
    assert!(macrun::watch::source(&dir.path().join("missing")).is_err());
}

#[test]
fn cancelled_persist_cannot_overwrite_a_newer_terminal_record() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.json");
        let (release, blocked) = std::sync::mpsc::channel();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
        });
        ready.await.unwrap();
        let initial = path.clone();
        let writer = tokio::spawn(async move {
            wire::persist(&initial, &json!({"status":"running"}))
                .await
                .unwrap();
        });
        tokio::task::yield_now().await;
        writer.abort();
        assert!(writer.await.unwrap_err().is_cancelled());
        let final_path = path.clone();
        let terminal = tokio::spawn(async move {
            wire::persist(&final_path, &json!({"status":"timed_out"}))
                .await
                .unwrap();
        });
        release.send(()).unwrap();
        blocker.await.unwrap();
        terminal.await.unwrap();
        let record: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(record["status"], "timed_out");
    });
}
