use macrun::{
    config::{Backend, WorkerConfig},
    engine::Engine,
    model::id,
    sync, sync_transfer, wire, workspace,
};
use serde_json::{Value, json};
use std::{fs, sync::Arc, time::Duration};

#[tokio::test]
async fn delta_and_pack_roundtrip_and_changed_base_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let root = dir.path().join("mirror");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&root).unwrap();
    let old = vec![42u8; 8 * 1024 * 1024];
    let mut new = old.clone();
    new[1024 * 1024 + 9] = 12;
    fs::write(source.join("large"), &new).unwrap();
    fs::write(root.join("large"), &old).unwrap();
    for i in 0..20 {
        fs::write(source.join(format!("small-{i}")), vec![b'x'; 4096]).unwrap();
    }
    let manifest = sync::scan(&source, &[]).unwrap();
    let receiver = Arc::new(
        sync::Receiver::prepare(&root, &dir.path().join("state"), manifest.clone()).unwrap(),
    );
    let offers = sync_transfer::offers(&receiver).unwrap();
    let lease = Arc::new(workspace::Lease::acquire(std::slice::from_ref(&root)).unwrap());
    let (mut a, mut b) = tokio::io::duplex(65536);
    let (sent, received) = tokio::join!(
        sync_transfer::send(&source, &manifest, &offers, &mut a),
        sync_transfer::receive(&mut b, receiver.clone(), &offers, lease.clone())
    );
    sent.unwrap();
    let stats = received.unwrap();
    assert_eq!(stats.files, 21);
    assert!(stats.bytes < 1_100_000);
    assert_eq!(stats.reused_bytes, 7 * 1024 * 1024);
    assert_eq!(stats.packs, 1);
    receiver.commit(&manifest).unwrap();
    assert_eq!(fs::read(root.join("large")).unwrap(), new);
    drop(lease);
    // Reuse signatures, then mutate an unchanged base block. No mixed-version publish.
    new[3 * 1024 * 1024] = 99;
    fs::write(source.join("large"), &new).unwrap();
    let manifest = sync::scan(&source, &[]).unwrap();
    let receiver = Arc::new(
        sync::Receiver::prepare(&root, &dir.path().join("state"), manifest.clone()).unwrap(),
    );
    let offers = sync_transfer::offers(&receiver).unwrap();
    let mut corrupt = fs::read(root.join("large")).unwrap();
    corrupt[0] = 0;
    fs::write(root.join("large"), &corrupt).unwrap();
    let lease = Arc::new(workspace::Lease::acquire(std::slice::from_ref(&root)).unwrap());
    let mut stream = Vec::new();
    sync_transfer::send(&source, &manifest, &offers, &mut stream)
        .await
        .unwrap();
    assert!(
        sync_transfer::receive(&mut stream.as_slice(), receiver, &offers, lease)
            .await
            .is_err()
    );
    assert_eq!(fs::read(root.join("large")).unwrap(), corrupt);
}

#[tokio::test]
async fn packed_corruption_and_unoffered_paths_are_rejected() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("source");
    let root = d.path().join("mirror");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("small"), "original").unwrap();
    let manifest = sync::scan(&source, &[]).unwrap();
    let r = Arc::new(
        sync::Receiver::prepare(&root, &d.path().join("state"), manifest.clone()).unwrap(),
    );
    let offers = sync_transfer::offers(&r).unwrap();
    let lease = Arc::new(workspace::Lease::acquire(std::slice::from_ref(&root)).unwrap());
    let mut bytes = Vec::new();
    sync_transfer::send(&source, &manifest, &offers, &mut bytes)
        .await
        .unwrap();
    *bytes.last_mut().unwrap() ^= 0xff;
    assert!(
        sync_transfer::receive(&mut bytes.as_slice(), r.clone(), &offers, lease.clone())
            .await
            .is_err()
    );
    assert!(!root.join("small").exists());
    let mut bad = Vec::new();
    wire::send(
        &mut bad,
        &json!({"kind":"file","path":"../escape","size":0}),
    )
    .await
    .unwrap();
    assert!(
        sync_transfer::receive(&mut bad.as_slice(), r, &offers, lease)
            .await
            .is_err()
    );
}

fn fixture() -> Backend {
    Backend {
        command: "python3".into(),
        args: vec![format!(
            "{}/tests/fixtures/mcp.py",
            env!("CARGO_MANIFEST_DIR")
        )],
        env: Default::default(),
        cwd: None,
    }
}
async fn done(e: &Arc<Engine>, task: &Value) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let v = e
                .handle("task.wait", json!({"task_id":task,"wait_ms":1000}))
                .await
                .unwrap();
            if v.get("ended_at").is_some() {
                return v;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn sequence_orders_steps_stops_on_condition_and_deduplicates() {
    let d = tempfile::tempdir().unwrap();
    let mut cfg = WorkerConfig::default();
    cfg.mcp.insert("a".into(), fixture());
    let e = Engine::open(d.path().into(), cfg).unwrap();
    let session = e.handle("mcp.tools", json!({"server":"a"})).await.unwrap()["session"].clone();
    let args = json!({"server":"a","session":session,"request_id":id(),"steps":[{"tool":"observe","arguments":{"text":"first"}},{"tool":"observe","arguments":{"text":"last"}}]});
    let task = e.handle("desktop.sequence", args.clone()).await.unwrap()["task_id"].clone();
    let result = done(&e, &task).await;
    assert_eq!(result["status"], "succeeded", "{result}");
    assert_eq!(result["result"]["steps"].as_array().unwrap().len(), 2);
    assert_eq!(
        e.handle("desktop.sequence", args).await.unwrap()["task_id"],
        task
    );
    let text: Value = serde_json::from_str(
        result["result"]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(text["count"], 2);
    assert!(result["result"]["result"]["content"][1]["data"].is_string());
    let task=e.handle("desktop.sequence",json!({"server":"a","session":session,"request_id":id(),"steps":[{"tool":"observe","expect":{"pointer":"/isError","equals":true}},{"tool":"observe"}]})).await.unwrap()["task_id"].clone();
    let result = done(&e, &task).await;
    assert_eq!(result["status"], "failed");
    assert_eq!(result["result"]["steps"].as_array().unwrap().len(), 1);
    let parent=e.handle("desktop.sequence",json!({"server":"a","session":session,"request_id":id(),"steps":[{"tool":"observe","arguments":{"crash":true}},{"tool":"observe"}]})).await.unwrap()["task_id"].clone();
    assert_eq!(done(&e, &parent).await["status"], "unknown");
    let parent = e
        .handle(
            "desktop.sequence",
            json!({"server":"a","session":session,"request_id":id(),"steps":[{"tool":"observe"}]}),
        )
        .await
        .unwrap()["task_id"]
        .clone();
    assert_eq!(done(&e, &parent).await["status"], "failed");
    let session = e.handle("mcp.tools", json!({"server":"a"})).await.unwrap()["session"].clone();
    let parent=e.handle("desktop.sequence",json!({"server":"a","session":session,"request_id":id(),"timeout_seconds":1,"steps":[{"tool":"observe","arguments":{"sleep":5}},{"tool":"observe"}]})).await.unwrap()["task_id"].clone();
    assert_eq!(done(&e, &parent).await["status"], "unknown");
    e.shutdown().await.unwrap();
}

#[tokio::test]
async fn resource_groups_share_only_the_declared_resource() {
    let a = macrun::scheduler::resource("group-a");
    let b = macrun::scheduler::resource("group-b");
    let held = a.lock().await;
    assert!(macrun::scheduler::resource("group-a").try_lock().is_err());
    assert!(b.try_lock().is_ok());
    drop(held);
    assert!(a.try_lock().is_ok());
}

#[tokio::test]
async fn persisted_history_reconciles_corrupt_index_and_receives_updates() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("tasks")).unwrap();
    let id = id();
    let path = d.path().join("tasks").join(&id).join("result.json");
    let record = json!({"task_id":id,"kind":"exec.start","status":"succeeded","started_at":1,"ended_at":2,"arguments":{"env":{"SECRET":"hidden"}},"result":{"content":[{"type":"image","data":"must not enter index"}]}});
    wire::atomic_json(&path, &record).unwrap();
    let h = macrun::history::TaskHistory::new(d.path().into());
    assert_eq!(h.records().await.unwrap().len(), 1);
    h.checkpoint().await.unwrap();
    let index = d.path().join("history-index.json");
    let stored = fs::read_to_string(&index).unwrap();
    assert!(!stored.contains("hidden") && !stored.contains("must not enter index"));
    let before = h.shared_records().await.unwrap();
    assert!(Arc::ptr_eq(&before, &h.shared_records().await.unwrap()));
    drop(h);
    fs::write(&index, "broken checkpoint").unwrap();
    let h = macrun::history::TaskHistory::new(d.path().into());
    assert_eq!(h.records().await.unwrap().len(), 1);
    let mut updated = record;
    updated["status"] = json!("failed");
    wire::atomic_json(&path, &updated).unwrap();
    assert_eq!(h.records().await.unwrap()[0]["status"], "failed");
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
    h.removed(&id);
    assert!(h.records().await.unwrap().is_empty());
}

#[tokio::test]
async fn tcp_tls_multiplexes_streams_and_rejects_wrong_certificate() {
    let data = tempfile::tempdir().unwrap();
    wire::init_tls(data.path()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let acceptor = macrun::transport::acceptor(data.path()).unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let conn = macrun::transport::accept(socket, acceptor.clone())
            .await
            .unwrap();
        let (mut out, mut input) = conn.accept_bi().await.unwrap();
        let first: Value = wire::recv(&mut input).await.unwrap();
        assert_eq!(first["first"], true);
        let (mut reply, mut request) = conn.accept_bi().await.unwrap();
        let second: Value = wire::recv(&mut request).await.unwrap();
        wire::send(&mut reply, &second).await.unwrap();
        wire::send(&mut out, &first).await.unwrap();
        out.finish().unwrap();
        reply.finish().unwrap();
        // Keep the connection alive until the peer confirms both replies.
        let _: Value = wire::recv(&mut input).await.unwrap();
        conn.close(0u32.into(), b"test complete");
        let (socket, _) = listener.accept().await.unwrap();
        assert!(macrun::transport::accept(socket, acceptor).await.is_err());
    });
    let client = macrun::transport::connect_tcp(&address, &data.path().join("cert.der"))
        .await
        .unwrap();
    let (mut out, mut input) = client.open_bi().await.unwrap();
    wire::send(&mut out, &json!({"first":true})).await.unwrap();
    let (mut second, mut response) = client.open_bi().await.unwrap();
    wire::send(&mut second, &json!({"second":true}))
        .await
        .unwrap();
    assert_eq!(
        wire::recv::<_, Value>(&mut response).await.unwrap()["second"],
        true
    );
    assert_eq!(
        wire::recv::<_, Value>(&mut input).await.unwrap()["first"],
        true
    );
    wire::send(&mut out, &json!({"ack":true})).await.unwrap();
    let wrong = tempfile::tempdir().unwrap();
    wire::init_tls(wrong.path()).unwrap();
    assert!(
        macrun::transport::connect_tcp(&address, &wrong.path().join("cert.der"))
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn snapshot_command_uses_committed_input_and_keeps_source_unchanged() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("source");
    let root = d.path().join("root");
    let data = d.path().join("worker");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("input"), "committed").unwrap();
    let manifest = sync::scan(&source, &[]).unwrap();
    let state = data.join("mirrors").join(
        blake3::hash(root.to_string_lossy().as_bytes())
            .to_hex()
            .as_str(),
    );
    let r = sync::Receiver::prepare(&root, &state, manifest.clone()).unwrap();
    let temp = root.join("temporary");
    fs::copy(source.join("input"), &temp).unwrap();
    r.install("input", &temp).unwrap();
    r.commit(&manifest).unwrap();
    let e = Engine::open(data, WorkerConfig::default()).unwrap();
    let task=e.handle("exec.start",json!({"command":"cat input; echo output > input","cwd":root,"snapshot":true,"generation":workspace::generation(&root).unwrap(),"request_id":id()})).await.unwrap()["task_id"].clone();
    let result = done(&e, &task).await;
    assert_eq!(result["status"], "succeeded", "{result}");
    assert!(
        result["output"]["text"]
            .as_str()
            .unwrap()
            .contains("committed")
    );
    assert_eq!(fs::read_to_string(root.join("input")).unwrap(), "committed");
    assert_eq!(result["result"]["snapshot"], true);
    let snap = result["result"]["workspace"].as_str().unwrap();
    assert_eq!(
        fs::read_to_string(std::path::Path::new(snap).join("input")).unwrap(),
        "output\n"
    );
    fs::write(root.join("input"), "edited!").unwrap();
    let task = e
        .handle(
            "exec.start",
            json!({"command":"touch forbidden","cwd":root,"snapshot":true,"request_id":id()}),
        )
        .await
        .unwrap()["task_id"]
        .clone();
    assert_eq!(done(&e, &task).await["status"], "failed");
    e.shutdown().await.unwrap();
}

#[tokio::test]
async fn interrupted_download_resumes_verified_prefix_before_atomic_publish() {
    use tokio::io::AsyncWriteExt;
    let d = tempfile::tempdir().unwrap();
    let socket = d.path().join("socket");
    let output = d.path().join("result.bin");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    #[cfg(target_os = "linux")]
    let output_for_server = output.clone();
    let server = tokio::spawn(async move {
        #[cfg(target_os = "linux")]
        let mut inherited_locks = Vec::new();
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: macrun::model::Request = wire::recv(&mut stream).await.unwrap();
            assert_eq!(request.args["offset"], if attempt == 0 { 0 } else { 2 });
            if attempt == 1 {
                assert_eq!(request.args["version"], "v1");
            }
            #[cfg(target_os = "linux")]
            if attempt == 0 {
                // A concurrent fork inherits open file descriptions until exec.
                // Retain equivalent duplicates across the immediate retry, so
                // releasing locks by descriptor close alone fails reliably.
                // Resolve every descriptor before duplicating any: a duplicate
                // could reuse the descriptor of the now-closed directory scan.
                let descriptors: Vec<i32> = fs::read_dir("/proc/self/fd")
                    .unwrap()
                    .flatten()
                    .filter_map(|entry| {
                        let path = fs::read_link(entry.path()).ok()?;
                        (path == output_for_server.with_extension("macrun-download-lock")
                            || path == output_for_server.with_extension("macrun-download"))
                        .then(|| entry.file_name().to_str().unwrap().parse().unwrap())
                    })
                    .collect();
                for fd in descriptors {
                    // SAFETY: the downloader owns both descriptors and is
                    // awaiting this server's reply on this single-threaded
                    // runtime, so neither descriptor can close here.
                    let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
                    inherited_locks.push(borrowed.try_clone_to_owned().unwrap());
                }
                assert_eq!(inherited_locks.len(), 2);
            }
            wire::send(&mut stream,&macrun::model::Reply::Status{value:json!({"ready":true,"size":if attempt==0{6}else{4},"version":"v1","prefix_hash":blake3::hash(b"ab").to_hex().to_string()})}).await.unwrap();
            if attempt == 0 {
                stream.write_all(b"ab").await.unwrap();
            } else {
                stream.write_all(b"cdef").await.unwrap();
                wire::send(
                    &mut stream,
                    &macrun::model::Reply::Status {
                        value: json!({"hash":blake3::hash(b"cdef").to_hex().to_string()}),
                    },
                )
                .await
                .unwrap();
            }
        }
        #[cfg(target_os = "linux")]
        drop(inherited_locks);
    });
    assert!(
        macrun::transfer::download_resumable(&socket, d.path(), "/source", &output, None)
            .await
            .is_err()
    );
    assert!(!output.exists());
    assert_eq!(
        fs::read(output.with_extension("macrun-download")).unwrap(),
        b"ab"
    );
    macrun::transfer::download_resumable(&socket, d.path(), "/source", &output, None)
        .await
        .unwrap();
    assert_eq!(fs::read(output).unwrap(), b"abcdef");
    server.await.unwrap();
}

#[tokio::test]
async fn resume_rejects_changed_prefix_without_publishing_or_appending() {
    let d = tempfile::tempdir().unwrap();
    let socket = d.path().join("sock");
    let output = d.path().join("result.bin");
    let partial = output.with_extension("macrun-download");
    fs::write(&output, b"existing output").unwrap();
    fs::write(&partial, b"tampered").unwrap();
    wire::atomic_json(&output.with_extension("macrun-download-meta"),&json!({"identity":{"socket":socket,"workspace":d.path(),"remote":"/source","client":null},"version":"v1"})).unwrap();
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _: macrun::model::Request = wire::recv(&mut stream).await.unwrap();
        wire::send(
            &mut stream,
            &macrun::model::Reply::Status {
                value: json!({"ready":true,"size":10,"version":"v1","prefix_hash":"wrong"}),
            },
        )
        .await
        .unwrap();
    });
    let error = macrun::transfer::download_resumable(&socket, d.path(), "/source", &output, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("prefix mismatch"));
    assert_eq!(fs::read(&output).unwrap(), b"existing output");
    assert_eq!(fs::read(&partial).unwrap(), b"tampered");
    server.await.unwrap();
}

#[test]
fn incoming_cleanup_refuses_symlinked_private_directories() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    let outside = d.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir_all(outside.join("incoming")).unwrap();
    let unrelated = outside.join("incoming").join(id());
    fs::write(&unrelated, "keep").unwrap();
    fs::write(outside.join("generation"), "keep").unwrap();
    symlink(&outside, root.join(".macrun")).unwrap();
    assert!(macrun::retention::incoming(&root, u64::MAX).is_err());
    assert!(sync::Receiver::prepare(&root, &d.path().join("state"), Default::default()).is_err());
    assert!(unrelated.exists());
    assert!(outside.join("generation").exists());
}

#[tokio::test]
async fn delta_offers_batch_small_paths_and_bound_signature_pages() {
    let mut offers: Vec<_> = (0..1000)
        .map(|i| sync_transfer::Offer {
            path: i.to_string(),
            blocks: vec![],
        })
        .collect();
    offers.push(sync_transfer::Offer {
        path: "large-a".into(),
        blocks: vec!["a".repeat(64); 8192],
    });
    offers.push(sync_transfer::Offer {
        path: "large-b".into(),
        blocks: vec!["b".repeat(64); 8192],
    });
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    sync_transfer::send_offers(&tx, &offers).await.unwrap();
    drop(tx);
    let mut count = 0;
    let mut pages = 0;
    let mut last_seen = false;
    while let Some(event) = rx.recv().await {
        assert!(!last_seen);
        assert!(serde_json::to_vec(&event).unwrap().len() < 2 * 1024 * 1024);
        if let macrun::model::Event::NeedObjects { objects, last } = event {
            count += objects.len();
            pages += 1;
            last_seen = last;
        } else {
            panic!("unexpected event")
        }
    }
    assert!(last_seen && pages < 10);
    assert_eq!(count, offers.len());
}

#[test]
fn snapshots_copy_directories_modes_and_internal_links_but_reject_escape() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    let state = d.path().join("state");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("bin/run"), "#!/bin/sh\ntrue\n").unwrap();
    fs::set_permissions(root.join("bin/run"), fs::Permissions::from_mode(0o755)).unwrap();
    symlink("bin/run", root.join("link")).unwrap();
    let manifest = sync::scan(&root, &[]).unwrap();
    let generation = sync::digest(&manifest).unwrap();
    wire::atomic_json(&state.join("manifest.json"), &manifest).unwrap();
    fs::create_dir(root.join(".macrun")).unwrap();
    fs::write(root.join(".macrun/generation"), &generation).unwrap();
    let output = d.path().join("snapshot");
    macrun::snapshot::create(&root, &state, &output, &generation).unwrap();
    assert_eq!(
        fs::read_link(output.join("link")).unwrap(),
        std::path::Path::new("bin/run")
    );
    assert_eq!(
        fs::metadata(output.join("bin/run"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(macrun::snapshot::create(&root, &state, &output, &generation).is_err());
    assert!(macrun::snapshot::create(&root, &state, &d.path().join("stale"), "wrong").is_err());
    fs::remove_file(root.join("link")).unwrap();
    symlink("/tmp", root.join("link")).unwrap();
    let output = d.path().join("refused");
    assert!(macrun::snapshot::create(&root, &state, &output, &generation).is_err());
    assert!(!output.exists());
}

#[tokio::test]
async fn independent_backend_groups_make_progress_concurrently() {
    let d = tempfile::tempdir().unwrap();
    let mut cfg = WorkerConfig::default();
    for name in ["a", "b"] {
        cfg.mcp.insert(name.into(), fixture());
        cfg.resource_groups
            .insert(name.into(), format!("{}-{name}", d.path().display()));
    }
    let e = Engine::open(d.path().join("state"), cfg).unwrap();
    let mut tasks = vec![];
    for (name, other) in [("a", "b"), ("b", "a")] {
        let session =
            e.handle("mcp.tools", json!({"server":name})).await.unwrap()["session"].clone();
        tasks.push(e.handle("mcp.call",json!({"server":name,"session":session,"tool":"observe","arguments":{"barrier":{"arrive":d.path().join(name),"other":d.path().join(other)}},"request_id":id()})).await.unwrap()["task_id"].clone());
    }
    for task in tasks {
        assert_eq!(done(&e, &task).await["status"], "succeeded");
    }
    e.shutdown().await.unwrap();
}

#[tokio::test]
async fn sequence_does_not_bypass_approval_or_dispatch_after_cancel() {
    let d = tempfile::tempdir().unwrap();
    let mut cfg = WorkerConfig::default();
    cfg.mcp.insert("a".into(), fixture());
    let e = Engine::open(d.path().into(), cfg).unwrap();
    let session = e.handle("mcp.tools", json!({"server":"a"})).await.unwrap()["session"].clone();
    let mut safety = macrun::safety::Safety::default();
    safety.desktop.observe = "confirm".into();
    e.set_safety(safety).await.unwrap();
    let parent=e.handle("desktop.sequence",json!({"server":"a","session":session,"request_id":id(),"steps":[{"tool":"observe"},{"tool":"observe"}]})).await.unwrap()["task_id"].clone();
    let child = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let page = e.local_task_list(json!({"kind":"mcp.call"})).await.unwrap();
            if let Some(task) = page["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["status"] == "awaiting_approval")
            {
                break task["task_id"].clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    e.handle("task.cancel", json!({"task_id":parent}))
        .await
        .unwrap();
    let _ = done(&e, &parent).await;
    assert_eq!(done(&e, &child).await["status"], "cancelled");
    assert_eq!(
        e.local_task_list(json!({"kind":"mcp.call"})).await.unwrap()["filtered_total"],
        1
    );
    assert_eq!(
        e.handle("mcp.tools", json!({"server":"a"})).await.unwrap()["session"],
        session
    );
    e.shutdown().await.unwrap();
}

#[test]
fn retention_preserves_active_leases_and_recent_or_unowned_files() {
    let d = tempfile::tempdir().unwrap();
    let task = d.path().join("task");
    fs::create_dir(&task).unwrap();
    let destination = d.path().join("out");
    let transfer = id();
    let part = destination.with_file_name(format!("out.{transfer}.macrun-part"));
    let meta = part.with_extension("macrun-meta");
    fs::write(&part, "partial").unwrap();
    wire::atomic_json(&meta, &json!({"path":destination})).unwrap();
    wire::atomic_json(
        &task.join("transfer.json"),
        &json!({"destination":destination,"transfer_id":transfer}),
    )
    .unwrap();
    assert!(!macrun::retention::transfers(&task, 0).unwrap());
    let lease = workspace::Lease::acquire(std::slice::from_ref(&destination)).unwrap();
    assert!(!macrun::retention::transfers(&task, u64::MAX).unwrap());
    drop(lease);
    assert!(macrun::retention::transfers(&task, u64::MAX).unwrap());
    assert!(!part.exists() && !meta.exists());
    let finished = d.path().join("sync-jobs").join(id());
    let active = d.path().join("sync-jobs").join(id());
    wire::atomic_json(&finished.join("result.json"), &json!({"ended_at":1})).unwrap();
    wire::atomic_json(&active.join("result.json"), &json!({"status":"syncing"})).unwrap();
    assert_eq!(macrun::retention::sync_jobs(d.path(), 2).unwrap(), 1);
    assert!(active.exists());
}
