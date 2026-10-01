use macrun::{
    cua::{Cua, Run},
    process::Identity,
};
use serde_json::json;
#[tokio::test]
async fn persistent_mcp_snapshot_resizes_without_losing_coordinate_mapping() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cua_mcp.py");
    let mut driver = Cua::connect(fixture.to_str().unwrap(), None).await.unwrap();
    let run = Run {
        run_id: "run1".into(),
        identity: Identity {
            pid: 123,
            started: 1,
            executable: "/fixture".into(),
        },
        app_path: "/fixture.app".into(),
        bundle_id: "fixture".into(),
    };
    assert_eq!(driver.windows(123).await.unwrap().len(), 1);
    let t = tempfile::tempdir().unwrap();
    let (snapshot, content, png) = driver.observe(&run, 42, 10, t.path()).await.unwrap();
    assert_eq!(image::load_from_memory(&png).unwrap().width(), 10);
    assert_eq!(content["detail"]["screenshot"]["pixels_per_point_x"], 1.0);
    let args = driver
        .action_args("click", &run, &snapshot, &json!({"x":5.0,"y":2.0}), false)
        .unwrap();
    assert_eq!(args["x"], 10.0);
    assert_eq!(args["y"], 4.0);
    assert_eq!(args["target"]["window_id"], 42);
    let result = driver.call("click", args).await.unwrap();
    assert_eq!(
        result["structuredContent"]["echo"]["delivery_mode"],
        "background"
    );
    assert_eq!(
        driver
            .action_args(
                "click",
                &run,
                &snapshot,
                &json!({"element":"invented"}),
                false
            )
            .unwrap_err()
            .code,
        "stale_snapshot"
    );
}
