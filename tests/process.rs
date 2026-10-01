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
