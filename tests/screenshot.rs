use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{DynamicImage, GenericImageView, ImageFormat, RgbImage, RgbaImage};
use macrun::screenshot::{Config, Frame, Mode, Session, fit, mode, optimize};
use serde_json::{Value, json};
use std::io::Cursor;

fn result(image: DynamicImage) -> Value {
    let (w, h) = image.dimensions();
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png).unwrap();
    let meta = json!({"screenshot_width":w,"screenshot_height":h,"screenshot_scale":2.0,"window_bounds":{"x":-100,"y":80,"width":w/2,"height":h/2},"elements":[{"frame":{"x":20,"y":40,"w":80,"h":20}}]});
    json!({"content":[{"type":"text","text":meta.to_string()},{"type":"image","mimeType":"image/png","data":STANDARD.encode(bytes.into_inner())}],"structuredContent":meta})
}
fn decode(v: &Value) -> DynamicImage {
    image::load_from_memory(
        &STANDARD
            .decode(v["content"][1]["data"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap()
}
fn frame(task: &str) -> Frame {
    Frame {
        source_width: 3024,
        source_height: 1964,
        width: 1662,
        height: 1080,
        task_id: task.into(),
    }
}
#[test]
fn config_and_modes_are_validated_and_scoped() {
    let mut c = Config::default();
    assert!(c.validate().is_ok());
    assert!(c.applies(
        "computer",
        "/Applications/CuaDriver.app/Contents/MacOS/cua-driver"
    ));
    assert!(c.applies("computer", "cua-driver.exe"));
    assert!(!c.applies("computer", "python3"));
    c.cua_backends.push("computer".into());
    assert!(c.applies("computer", "wrapper"));
    c.enabled = false;
    assert!(!c.applies("computer", "cua-driver"));
    c.max_width = 0;
    assert!(c.validate().is_err());
    c.max_width = 1920;
    c.max_height = 8193;
    assert!(c.validate().is_err());
    c.max_height = 1080;
    c.jpeg_quality = 0;
    assert!(c.validate().is_err());
    assert_eq!(mode(&json!({})).unwrap(), Mode::Auto);
    for (name, expected) in [
        ("auto", Mode::Auto),
        ("png", Mode::Png),
        ("jpeg", Mode::Jpeg),
        ("original", Mode::Original),
    ] {
        assert_eq!(mode(&json!({"screenshot_mode":name})).unwrap(), expected);
    }
    assert!(mode(&json!({"screenshot_mode":"bad"})).is_err());
    assert!(mode(&json!({"screenshot_mode":42})).is_err());
}
#[test]
fn dimensions_fit_landscape_portrait_and_extreme_aspect_without_upscaling() {
    assert_eq!(fit(3024, 1964, 1920, 1080), (1662, 1080));
    assert_eq!(fit(3840, 2160, 1920, 1080), (1920, 1080));
    assert_eq!(fit(2160, 3840, 1920, 1080), (607, 1080));
    assert_eq!(fit(4000, 1000, 1920, 1080), (1920, 480));
    assert_eq!(fit(600, 400, 1920, 1080), (600, 400));
    assert_eq!(fit(1, 16384, 1920, 1080), (1, 1080));
    assert_eq!(fit(16384, 1, 1920, 1080), (1920, 1));
}
#[test]
fn resizes_updates_both_metadata_copies_and_retains_native_geometry() {
    let input = result(DynamicImage::ImageRgb8(RgbImage::from_pixel(
        240,
        160,
        image::Rgb([20, 40, 60]),
    )));
    let cfg = Config {
        max_width: 120,
        max_height: 60,
        ..Config::default()
    };
    let output = optimize(input.clone(), &cfg, Mode::Auto, "task").unwrap();
    assert_eq!(decode(&output.result).dimensions(), (90, 60));
    let meta = &output.result["structuredContent"];
    assert_eq!(meta["screenshot_width"], 90);
    assert_eq!(meta["screenshot_height"], 60);
    assert_eq!(meta["screenshot_scale"], 2.0);
    assert_eq!(
        output.result["macrun_screenshot"]["pixels_per_point_x"],
        0.75
    );
    assert_eq!(
        meta["window_bounds"],
        input["structuredContent"]["window_bounds"]
    );
    assert_eq!(meta["elements"], input["structuredContent"]["elements"]);
    assert_eq!(
        serde_json::from_str::<Value>(output.result["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        *meta
    );
    assert_eq!(
        output.result["macrun_screenshot"]["source_screenshot_scale"],
        2.0
    );
    assert_eq!(output.result["macrun_screenshot"]["task_id"], "task");
    let original = optimize(input.clone(), &cfg, Mode::Original, "original").unwrap();
    assert_eq!(original.result["content"], input["content"]);
    assert_eq!(
        original.result["structuredContent"],
        input["structuredContent"]
    );
}
#[test]
fn small_text_image_stays_lossless_and_never_grows() {
    let img = RgbImage::from_fn(120, 60, |x, y| {
        image::Rgb(if (x / 3 + y / 7) % 2 == 0 {
            [0, 0, 0]
        } else {
            [255, 255, 255]
        })
    });
    let input = result(DynamicImage::ImageRgb8(img.clone()));
    for mode in [Mode::Auto, Mode::Png, Mode::Jpeg] {
        let output = optimize(input.clone(), &Config::default(), mode, "t")
            .unwrap()
            .result;
        assert_eq!(decode(&output).to_rgb8(), img);
        assert!(
            output["macrun_screenshot"]["bytes"].as_u64().unwrap()
                <= output["macrun_screenshot"]["source_bytes"]
                    .as_u64()
                    .unwrap()
        );
    }
}
#[test]
fn jpeg_is_opt_in_and_transparency_is_preserved() {
    let mut seed = 42u32;
    let img = RgbImage::from_fn(200, 160, |_, _| {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        image::Rgb([(seed >> 8) as u8, (seed >> 16) as u8, (seed >> 24) as u8])
    });
    let input = result(DynamicImage::ImageRgb8(img));
    let auto = optimize(input.clone(), &Config::default(), Mode::Auto, "a").unwrap();
    assert_eq!(auto.result["content"][1]["mimeType"], "image/png");
    let jpeg = optimize(input, &Config::default(), Mode::Jpeg, "j").unwrap();
    assert_eq!(jpeg.result["content"][1]["mimeType"], "image/jpeg");
    assert_eq!(decode(&jpeg.result).dimensions(), (200, 160));
    let transparent = result(DynamicImage::ImageRgba8(RgbaImage::from_pixel(
        40,
        20,
        image::Rgba([10, 20, 30, 80]),
    )));
    let out = optimize(transparent, &Config::default(), Mode::Jpeg, "a").unwrap();
    assert_eq!(out.result["content"][1]["mimeType"], "image/png");
    assert_eq!(decode(&out.result).to_rgba8().get_pixel(0, 0).0[3], 80);
}
#[test]
fn rejects_corrupt_ambiguous_or_mislabeled_images_and_accepts_tree_only() {
    let input = result(DynamicImage::new_rgb8(20, 10));
    for bad in ["bad", ""] {
        let mut v = input.clone();
        v["content"][1]["data"] = json!(bad);
        assert!(optimize(v, &Config::default(), Mode::Auto, "t").is_err());
    }
    let mut v = input.clone();
    v["content"][1]["mimeType"] = json!("image/jpeg");
    assert!(optimize(v, &Config::default(), Mode::Auto, "t").is_err());
    let mut v = input.clone();
    v["content"]
        .as_array_mut()
        .unwrap()
        .push(input["content"][1].clone());
    assert!(optimize(v, &Config::default(), Mode::Auto, "t").is_err());
    let tree =
        json!({"content":[{"type":"text","text":"tree"}],"structuredContent":{"elements":[]}});
    let out = optimize(tree.clone(), &Config::default(), Mode::Auto, "t").unwrap();
    assert!(out.frame.is_none());
    assert_eq!(out.result, tree);
}
#[test]
fn coordinates_map_per_session_target_and_task_with_retina_rounding() {
    let mut s = Session::default();
    let args = json!({"pid":10,"window_id":20,"session":"alice"});
    s.record("get_window_state", &args, Some(frame("t")))
        .unwrap();
    for tool in [
        "click",
        "double_click",
        "right_click",
        "scroll",
        "type_text",
        "move_cursor",
    ] {
        let a = json!({"target":{"kind":"window","pid":10,"window_id":20},"session":"alice","x":831,"y":540});
        let out = s.arguments(tool, &a, Some("t")).unwrap();
        assert_eq!(out["x"], 1512.0);
        assert_eq!(out["y"], 982.0);
        assert_eq!(out["target"], a["target"]);
    }
    let a = json!({"pid":10,"window_id":20,"session":"alice","from_x":0,"from_y":0,"to_x":831,"to_y":540});
    let out = s.arguments("drag", &a, Some("t")).unwrap();
    assert_eq!(out["to_x"], 1512.0);
    assert_eq!(out["to_y"], 982.0);
    let a = json!({"pid":10,"window_id":20,"session":"alice","x1":0,"y1":0,"x2":1662,"y2":1080});
    let out = s.arguments("zoom", &a, Some("t")).unwrap();
    assert_eq!(out["x2"], 3024.0);
    assert_eq!(out["y2"], 1964.0);
    let mut a = json!({"pid":10,"window_id":20,"session":"alice","x":831,"y":540});
    assert!(s.arguments("click", &a, Some("old")).is_err());
    a["session"] = json!("bob");
    assert!(s.arguments("click", &a, Some("t")).is_err());
    assert!(s.arguments("click", &a, None).is_err());
    a["session"] = json!("alice");
    a["window_id"] = json!(21);
    assert!(s.arguments("click", &a, Some("t")).is_err());
    s.record("get_window_state", &args, None).unwrap();
    a["window_id"] = json!(20);
    assert!(s.arguments("click", &a, None).is_err());
    s.clear();
    assert!(s.arguments("click", &a, Some("t")).is_err());
}
#[test]
fn desktop_mapping_and_ax_zoom_passthrough_are_distinct() {
    let mut s = Session::default();
    s.record("get_desktop_state", &json!({}), Some(frame("desktop")))
        .unwrap();
    let mut a = json!({"scope":"desktop","x":831,"y":540});
    assert_eq!(
        s.arguments("click", &a, Some("desktop")).unwrap()["y"],
        982.0
    );
    a["target"] = json!({"kind":"desktop","display_id":"primary"});
    assert_eq!(s.arguments("click", &a, None).unwrap()["x"], 1512.0);
    let conflict =
        json!({"scope":"desktop","target":{"kind":"window","pid":10,"window_id":20},"x":1,"y":1});
    assert!(s.arguments("click", &conflict, None).is_err());
    a["pid"] = json!(10);
    assert!(s.arguments("click", &a, None).is_err());
    for a in [
        json!({"pid":10,"element_token":"token"}),
        json!({"pid":10,"element_index":1,"snapshot_id":"s00000000"}),
        json!({"pid":10,"x":20,"y":30,"from_zoom":true}),
    ] {
        assert_eq!(s.arguments("click", &a, None).unwrap(), a);
        assert!(s.arguments("click", &a, Some("desktop")).is_err());
    }
    assert_eq!(
        s.arguments("launch_app", &json!({"name":"Test"}), None)
            .unwrap(),
        json!({"name":"Test"})
    );
    assert!(
        s.arguments("launch_app", &json!({}), Some("desktop"))
            .is_err()
    );
    for a in [
        json!({"scope":"desktop","x":-1,"y":0}),
        json!({"scope":"desktop","x":1662,"y":0}),
        json!({"scope":"desktop","x":0}),
        json!({"target":{"kind":"desktop","display_id":"other"},"x":0,"y":0}),
    ] {
        assert!(s.arguments("click", &a, None).is_err());
    }
}

async fn wait(engine: &std::sync::Arc<macrun::engine::Engine>, id: &Value) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let task = engine
                .handle("task.get", json!({"task_id":id,"wait_ms":1000}))
                .await
                .unwrap();
            if task.get("ended_at").is_some() {
                return task;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn worker_optimizes_before_artifacts_and_maps_actions_in_sequences() {
    use macrun::{
        config::{Backend, WorkerConfig},
        engine::Engine,
        model::id,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut config = WorkerConfig {
        screenshots: Config {
            max_width: 120,
            max_height: 60,
            cua_backends: vec!["fixture".into()],
            ..Config::default()
        },
        ..WorkerConfig::default()
    };
    config.mcp.insert(
        "fixture".into(),
        Backend {
            command: "python3".into(),
            args: vec![format!(
                "{}/tests/fixtures/screenshot.py",
                env!("CARGO_MANIFEST_DIR")
            )],
            env: Default::default(),
            cwd: None,
        },
    );
    let engine = Engine::open(dir.path().into(), config).unwrap();
    let discovery = engine
        .handle("mcp.tools", json!({"server":"fixture"}))
        .await
        .unwrap();
    let session = &discovery["session"];
    assert!(
        discovery["result"]["tools"][0]["description"]
            .as_str()
            .unwrap()
            .contains("120x60")
    );
    let call = |tool: &str, args: Value| json!({"server":"fixture","session":session,"tool":tool,"arguments":args,"request_id":id()});
    let req = call("get_window_state", json!({"pid":10,"window_id":20}));
    let capture = engine.handle("mcp.call", req).await.unwrap();
    let task = wait(&engine, &capture["task_id"]).await;
    assert_eq!(task["status"], "succeeded", "{task}");
    let result = &task["result"]["result"];
    assert_eq!(decode(result).dimensions(), (90, 60));
    assert_eq!(result["macrun_screenshot"]["source_width"], 120);
    assert_eq!(
        result["structuredContent"]["arguments_received"]["max_dimension"],
        120
    );
    let refs = engine
        .handle(
            "task.get",
            json!({"task_id":capture["task_id"],"artifact_refs":true}),
        )
        .await
        .unwrap();
    assert!(refs["result"]["result"]["content"][1].get("data").is_none());
    assert_eq!(
        refs["artifacts"][0]["bytes"],
        result["macrun_screenshot"]["bytes"]
    );
    // The frontend must expose a separate image, never its base64 in text.
    let content = macrun::frontend::content(&task);
    let blocks = content["content"].as_array().unwrap();
    let data = result["content"][1]["data"].as_str().unwrap();
    assert!(blocks.iter().any(|b| b["type"] == "image"));
    assert!(
        blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .all(|b| !b["text"].as_str().unwrap().contains(data))
    );
    let mut click = call("click", json!({"pid":10,"window_id":20,"x":45,"y":30}));
    click["coordinate_task_id"] = capture["task_id"].clone();
    let click = engine.handle("mcp.call", click).await.unwrap();
    let done = wait(&engine, &click["task_id"]).await;
    assert_eq!(done["result"]["result"]["structuredContent"]["x"], 60.0);
    assert_eq!(done["result"]["result"]["structuredContent"]["y"], 40.0);
    let sequence = engine.handle("desktop.sequence",json!({"server":"fixture","session":session,"request_id":id(),"steps":[{"tool":"get_window_state","arguments":{"pid":10,"window_id":20},"screenshot_mode":"original"},{"tool":"click","arguments":{"pid":10,"window_id":20,"x":120,"y":80}}]})).await.unwrap();
    let done = wait(&engine, &sequence["task_id"]).await;
    assert_eq!(done["status"], "succeeded", "{done}");
    assert_eq!(done["result"]["result"]["structuredContent"]["x"], 120.0);
    // Unknown image-returning tools are byte-for-byte passthrough.
    let other = engine
        .handle("mcp.call", call("observe", json!({})))
        .await
        .unwrap();
    let done = wait(&engine, &other["task_id"]).await;
    assert!(done["result"]["result"].get("macrun_screenshot").is_none());
    assert_eq!(decode(&done["result"]["result"]).dimensions(), (240, 160));
    assert!(
        engine
            .handle("mcp.call", call("get_window_state", json!(null)))
            .await
            .is_err()
    );
    let mut stale = call("click", json!({"pid":10,"window_id":20,"x":45,"y":30}));
    stale["coordinate_task_id"] = capture["task_id"].clone();
    let stale = engine.handle("mcp.call", stale).await.unwrap();
    let rejected = wait(&engine, &stale["task_id"]).await;
    assert_eq!(rejected["status"], "unknown");
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("superseded")
    );
    let refreshed = engine
        .handle("mcp.tools", json!({"server":"fixture"}))
        .await
        .unwrap();
    assert_ne!(&refreshed["session"], session);
    engine.shutdown().await.unwrap();
}
