use macrun::{model::Event, process};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
#[tokio::test]
async fn cancellation_kills_descendants_and_preserves_streamed_output() {
    let t = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::channel::<Event>(100);
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    let root = t.path().to_path_buf();
    let task = tokio::spawn(async move {
        process::run(
            "/bin/sh",
            &["-c".into(), "sleep 60 & echo CHILD:$!; wait".into()],
            &root,
            30,
            &c,
            &tx,
            "build.log",
            &root.join("process.json"),
        )
        .await
    });
    let child = loop {
        let e = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        if let Event::Log { text, .. } = e
            && let Some(p) = text.trim().strip_prefix("CHILD:")
        {
            break p.parse::<u32>().unwrap();
        }
    };
    let identity = process::identity(child);
    cancel.cancel();
    assert_eq!(task.await.unwrap().unwrap_err().code, "cancelled");
    if let Some(i) = identity {
        for _ in 0..30 {
            if !process::alive(&i) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!process::alive(&i));
    }
    assert!(!t.path().join("process.json").exists());
}

#[tokio::test]
async fn pipe_reads_do_not_split_utf8_characters() {
    let t = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::channel(100);
    process::run(
        "/bin/sh",
        &[
            "-c".into(),
            "printf '\\344'; sleep 0.1; printf '\\275\\240'".into(),
        ],
        t.path(),
        5,
        &CancellationToken::new(),
        &tx,
        "output.log",
        &t.path().join("process.json"),
    )
    .await
    .unwrap();
    drop(tx);
    let mut output = String::new();
    while let Some(Event::Log { text, .. }) = rx.recv().await {
        output.push_str(&text);
    }
    assert_eq!(output, "你");
}

// Cleanup guards keep a failed assertion from leaving this test's fixture alive.
struct Fixture(tempfile::TempDir);
impl Fixture {
    fn pid(&self) -> Option<u32> {
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(self.0.path().join("child.json")).ok()?).ok()?;
        v["pid"].as_u64().map(|p| p as u32)
    }
    async fn ready(&self) -> u32 {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(pid) = self.pid() {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(pid) = self.pid() {
            // The fixture has a 30-second self deadline, and all assertions finish
            // within it. Only its exact recorded PID is a fallback cleanup target.
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }
}
fn fixture_args(mode: &str, root: &std::path::Path) -> Vec<String> {
    vec![
        format!(
            "{}/tests/fixtures/process_tree.py",
            env!("CARGO_MANIFEST_DIR")
        ),
        mode.into(),
        root.to_str().unwrap().into(),
    ]
}

#[tokio::test]
async fn cross_session_helpers_and_log_senders_finish_for_every_exit_path() {
    for mode in ["exit", "closed", "timeout", "cancel", "exec", "platform"] {
        let fixture = Fixture(tempfile::tempdir().unwrap());
        let root = fixture.0.path().to_path_buf();
        let (tx, mut rx) = mpsc::channel(100);
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        let job = tokio::spawn(async move {
            process::run(
                "python3",
                &fixture_args(mode, &root),
                &root,
                2,
                &c,
                &tx,
                "output.log",
                &root.join("process.json"),
            )
            .await
        });
        let pid = fixture.ready().await;
        if mode == "exec" {
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let journal: serde_json::Value = serde_json::from_slice(
                        &std::fs::read(fixture.0.path().join("process.json")).unwrap(),
                    )
                    .unwrap();
                    if journal["tree"]["processes"].get(pid.to_string()).is_some() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                std::fs::write(fixture.0.path().join("exec-now"), "").unwrap();
                while !process::identity(pid).is_some_and(|i| i.executable.ends_with("sleep")) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
        }
        if mode == "cancel" || mode == "exec" {
            cancel.cancel();
        }
        let result = tokio::time::timeout(Duration::from_secs(7), job)
            .await
            .unwrap()
            .unwrap();
        match mode {
            "timeout" => assert_eq!(result.unwrap_err().code, "timed_out"),
            "cancel" | "exec" => assert_eq!(result.unwrap_err().code, "cancelled"),
            _ => result.unwrap(),
        }
        assert!(process::identity(pid).is_none(), "{mode}: helper survived");
        assert!(!fixture.0.path().join("process.json").exists());
        let output = tokio::time::timeout(Duration::from_secs(1), async {
            let mut output = String::new();
            while let Some(Event::Log { text, .. }) = rx.recv().await {
                output.push_str(&text);
            }
            output
        })
        .await
        .expect("log sender leaked");
        assert!(output.contains("parent output"));
        if !matches!(mode, "closed" | "platform") {
            assert!(output.contains("child output"));
        }
    }
}

#[tokio::test]
async fn dropping_execution_cleans_helpers_and_aborts_readers() {
    let fixture = Fixture(tempfile::tempdir().unwrap());
    let root = fixture.0.path().to_path_buf();
    let (tx, mut rx) = mpsc::channel(100);
    let job = tokio::spawn(async move {
        process::run(
            "python3",
            &fixture_args("abort", &root),
            &root,
            30,
            &CancellationToken::new(),
            &tx,
            "output.log",
            &root.join("process.json"),
        )
        .await
    });
    let pid = fixture.ready().await;
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), async {
        while process::identity(pid).is_some() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        while rx.recv().await.is_some() {}
    })
    .await
    .unwrap();
    assert!(fixture.0.path().join("process.json").exists());
    process::recover(&fixture.0.path().join("process.json")).unwrap();
}

#[tokio::test]
async fn blocked_log_consumer_is_bounded_and_does_not_leak_senders() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::channel(1);
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        process::run(
            "python3",
            &["-c".into(), "print('x' * 100000)".into()],
            dir.path(),
            1,
            &CancellationToken::new(),
            &tx,
            "output.log",
            &dir.path().join("process.json"),
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(result.code, "recovery_required");
    assert!(result.message.contains("truncated"));
    drop(tx);
    tokio::time::timeout(Duration::from_secs(1), async {
        while rx.recv().await.is_some() {}
    })
    .await
    .unwrap();
    assert!(dir.path().join("process.json").exists());
    process::recover(&dir.path().join("process.json")).unwrap();
}

#[tokio::test]
async fn fast_exiting_parent_still_cleans_platform_binary_in_its_group() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::channel(100);
    process::run(
        "/bin/sh",
        &["-c".into(), "sleep 30 & echo $!".into()],
        dir.path(),
        3,
        &CancellationToken::new(),
        &tx,
        "output.log",
        &dir.path().join("process.json"),
    )
    .await
    .unwrap();
    drop(tx);
    let mut output = String::new();
    while let Some(Event::Log { text, .. }) = rx.recv().await {
        output.push_str(&text);
    }
    let pid = output.trim().parse().unwrap();
    assert!(process::identity(pid).is_none());
}

#[test]
fn legacy_and_invalid_recovery_records_are_handled_conservatively() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("process.json");
    process::recover(&path).unwrap();
    std::fs::write(&path, "not json").unwrap();
    assert_eq!(
        process::recover(&path).unwrap_err().code,
        "recovery_required"
    );
    std::fs::write(&path, r#"{"state":"spawning"}"#).unwrap();
    assert!(process::recover(&path).is_err());
    assert!(path.exists());
    // Impossible PID, preserving the legacy schema without signalling any process.
    std::fs::write(
        &path,
        r#"{"identity":{"pid":2147483647,"started":1,"executable":"/missing"}}"#,
    )
    .unwrap();
    process::recover(&path).unwrap();
    assert!(!path.exists());
}
