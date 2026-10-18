use macrun::wire;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
#[tokio::test]
async fn fragmented_frame_survives_other_control_work() {
    let (mut writer, reader) = tokio::io::duplex(128);
    let mut messages = wire::read_channel::<_, Value>(reader);
    let data = serde_json::to_vec(&json!({"ready":true})).unwrap();
    let length = (data.len() as u32).to_be_bytes();
    writer.write_all(&length[..2]).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), messages.recv())
            .await
            .is_err()
    );
    writer.write_all(&length[2..]).await.unwrap();
    writer.write_all(&data[..3]).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), messages.recv())
            .await
            .is_err()
    );
    writer.write_all(&data[3..]).await.unwrap();
    assert_eq!(
        messages.recv().await.unwrap().unwrap(),
        json!({"ready":true})
    );
}
#[tokio::test]
async fn oversized_frames_are_rejected_before_allocating_payload() {
    let (mut writer, mut reader) = tokio::io::duplex(16);
    writer
        .write_u32((wire::MAX_FRAME + 1) as u32)
        .await
        .unwrap();
    assert!(
        wire::recv::<_, Value>(&mut reader)
            .await
            .unwrap_err()
            .to_string()
            .contains("frame limit")
    );
}

#[test]
fn atomic_json_does_not_follow_existing_temp_links_and_cleans_failed_writes() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("keep");
    fs::write(&outside, "unchanged").unwrap();
    std::os::unix::fs::symlink(&outside, root.path().join("result.tmp")).unwrap();
    let path = root.path().join("result.json");
    wire::atomic_json(&path, &json!({"state":"saved"})).unwrap();
    assert_eq!(fs::read_to_string(&outside).unwrap(), "unchanged");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap()["state"],
        "saved"
    );
    let invalid = root.path().join("directory.json");
    fs::create_dir(&invalid).unwrap();
    assert!(wire::atomic_json(&invalid, &json!({})).is_err());
    let leftovers = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n.to_string_lossy().ends_with(".tmp"))
        .collect::<Vec<_>>();
    assert_eq!(leftovers, ["result.tmp"]);
}
