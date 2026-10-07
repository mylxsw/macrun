//! CuaDriver screenshot adapter. The backend's pixel frame stays private and
//! actions are mapped back to it under the same lock as capture and dispatch.
use anyhow::{Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{
    DynamicImage, ImageReader,
    codecs::{
        jpeg::JpegEncoder,
        png::{CompressionType, FilterType, PngEncoder},
    },
    imageops,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Cursor, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub max_width: u32,
    pub max_height: u32,
    pub jpeg_quality: u8,
    /// Explicit opt-in for CuaDriver launched through a wrapper command.
    pub cua_backends: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            max_width: 1920,
            max_height: 1080,
            jpeg_quality: 90,
            cua_backends: vec![],
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=8192).contains(&self.max_width) && (1..=8192).contains(&self.max_height),
            "screenshot bounds must be between 1 and 8192"
        );
        ensure!(
            (1..=100).contains(&self.jpeg_quality),
            "screenshot JPEG quality must be between 1 and 100"
        );
        Ok(())
    }
    pub fn applies(&self, name: &str, command: &str) -> bool {
        self.enabled
            && (Path::new(command)
                .file_name()
                .is_some_and(|n| n == "cua-driver" || n == "cua-driver.exe")
                || self.cua_backends.iter().any(|n| n == name))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Auto,
    Png,
    Jpeg,
    Original,
}
pub fn mode(request: &Value) -> Result<Mode> {
    Ok(request
        .get("screenshot_mode")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?
        .unwrap_or_default())
}
pub fn is_capture(tool: &str) -> bool {
    matches!(tool, "get_window_state" | "get_desktop_state")
}

/// Integer arithmetic keeps both edges within bounds, including portrait images.
pub fn fit(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    if width <= max_width && height <= max_height {
        return (width, height);
    }
    if u64::from(width) * u64::from(max_height) > u64::from(height) * u64::from(max_width) {
        (
            max_width,
            (u64::from(height) * u64::from(max_width) / u64::from(width)).max(1) as u32,
        )
    } else {
        (
            (u64::from(width) * u64::from(max_height) / u64::from(height)).max(1) as u32,
            max_height,
        )
    }
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub source_width: u32,
    pub source_height: u32,
    pub width: u32,
    pub height: u32,
    pub task_id: String,
}
impl Frame {
    fn metadata(&self) -> Value {
        json!({"source_width":self.source_width,"source_height":self.source_height,"width":self.width,"height":self.height,
            "to_backend_x":self.source_width as f64 / self.width as f64,"to_backend_y":self.source_height as f64 / self.height as f64,
            "task_id":self.task_id,"coordinate_space":"returned_image_pixels","actions":"Macrun maps pixel actions to the latest screenshot for this backend session and target. Pass coordinate_task_id to reject a superseded screenshot. AX frames remain in backend coordinates."})
    }
}

pub struct Optimized {
    pub result: Value,
    pub frame: Option<Frame>,
}

/// Runs on the blocking pool. Only the two known capture tools call this;
/// file_image, zoom crops and arbitrary MCP images retain their own contracts.
pub fn optimize(
    mut result: Value,
    config: &Config,
    mode: Mode,
    task_id: &str,
) -> Result<Optimized> {
    let indices: Vec<usize> = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, v)| (v["type"] == "image").then_some(i))
        .collect();
    if indices.is_empty() {
        return Ok(Optimized {
            result,
            frame: None,
        });
    }
    ensure!(indices.len() == 1, "ambiguous screenshot frames");
    let index = indices[0];
    let block = &result["content"][index];
    let mime_owned = block["mimeType"].as_str().unwrap_or("").to_owned();
    let mime = mime_owned.as_str();
    ensure!(
        matches!(mime, "image/png" | "image/jpeg"),
        "unsupported screenshot MIME type"
    );
    let bytes = STANDARD.decode(
        block["data"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("screenshot data missing"))?,
    )?;
    let format = image::guess_format(&bytes)?;
    ensure!(
        matches!(
            (mime, format),
            ("image/png", image::ImageFormat::Png) | ("image/jpeg", image::ImageFormat::Jpeg)
        ),
        "screenshot MIME mismatch"
    );
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let (source_width, source_height) =
        ImageReader::with_format(Cursor::new(&bytes), format).into_dimensions()?;
    ensure!(
        u64::from(source_width) * u64::from(source_height) <= 64 * 1024 * 1024,
        "screenshot pixel limit exceeded"
    );
    let (width, height) = if mode == Mode::Original {
        (source_width, source_height)
    } else {
        fit(
            source_width,
            source_height,
            config.max_width,
            config.max_height,
        )
    };
    let frame = Frame {
        source_width,
        source_height,
        width,
        height,
        task_id: task_id.into(),
    };
    let original_metadata = result
        .get("structuredContent")
        .cloned()
        .unwrap_or(Value::Null);
    let mut output_bytes = bytes.len();
    if mode != Mode::Original {
        let source = reader.decode()?;
        let resized = (width, height) != (source_width, source_height);
        let image = if resized {
            source.resize_exact(width, height, imageops::FilterType::Lanczos3)
        } else {
            source
        };
        // Remove an unused alpha channel, but never discard real transparency.
        let rgba = image.to_rgba8();
        let opaque = rgba.pixels().all(|p| p.0[3] == 255);
        let image = if opaque {
            DynamicImage::ImageRgb8(image.to_rgb8())
        } else {
            DynamicImage::ImageRgba8(rgba)
        };
        let mut png = Vec::new();
        image.write_with_encoder(PngEncoder::new_with_quality(
            &mut png,
            CompressionType::Default,
            FilterType::Adaptive,
        ))?;
        let (mut encoded, mut mime_out) = (png, "image/png");
        // Auto is deliberately lossless for UI/text. JPEG is an explicit content
        // choice (photos/video); even then keep PNG if it is smaller.
        if mode == Mode::Jpeg && opaque {
            let mut jpeg = Vec::new();
            JpegEncoder::new_with_quality(&mut jpeg, config.jpeg_quality).encode_image(&image)?;
            if jpeg.len() < encoded.len() {
                encoded = jpeg;
                mime_out = "image/jpeg";
            }
        }
        if !resized && bytes.len() <= encoded.len() && (mode != Mode::Png || mime == "image/png") {
            encoded = bytes.clone();
            mime_out = mime;
        }
        output_bytes = encoded.len();
        result["content"][index]["data"] = json!(STANDARD.encode(&encoded));
        result["content"][index]["mimeType"] = json!(mime_out);
    }
    // CuaDriver repeats metadata as a JSON text block for older consumers.
    // Update only known metadata fields, never AX element frames/window bounds.
    if let Some(structured) = result.get_mut("structuredContent") {
        update_metadata(structured, &frame);
    }
    if let Some(content) = result["content"].as_array_mut() {
        for block in content {
            if block["type"] == "text"
                && let Some(text) = block["text"].as_str()
                && let Ok(mut meta) = serde_json::from_str::<Value>(text)
            {
                update_metadata(&mut meta, &frame);
                block["text"] = json!(serde_json::to_string(&meta)?);
            }
        }
    }
    let mut metadata = frame.metadata();
    for key in [
        "screenshot_scale",
        "window_bounds",
        "screenshot_frame",
        "screenshot_frame_valid",
        "scale_factor",
    ] {
        if let Some(v) = original_metadata.get(key) {
            metadata[format!("source_{key}")] = v.clone();
        }
    }
    // CuaDriver's screenshot_scale is the native backing scale, not necessarily
    // returned pixels / window points (its own max_dimension may already apply).
    for (axis, pixels, dimension) in [("x", width, "width"), ("y", height, "height")] {
        if let Some(points) = original_metadata["window_bounds"][dimension]
            .as_f64()
            .filter(|n| *n > 0.0)
        {
            metadata[format!("pixels_per_point_{axis}")] = json!(pixels as f64 / points);
        }
    }
    metadata["source_bytes"] = json!(bytes.len());
    metadata["bytes"] = json!(output_bytes);
    metadata["mimeType"] = result["content"][index]["mimeType"].clone();
    result["macrun_screenshot"] = metadata;
    Ok(Optimized {
        result,
        frame: Some(frame),
    })
}

fn update_metadata(meta: &mut Value, frame: &Frame) {
    if !meta.is_object() {
        return;
    }
    if meta.get("screenshot_width").is_some() {
        meta["screenshot_width"] = json!(frame.width);
    }
    if meta.get("screenshot_height").is_some() {
        meta["screenshot_height"] = json!(frame.height);
    }
}

#[derive(Default)]
pub struct Session {
    // None marks a capture that returned no usable image; do not reuse old pixels.
    frames: BTreeMap<String, Option<Frame>>,
}
fn target(tool: &str, a: &Value) -> Result<String> {
    let session = a["session"].as_str().unwrap_or("");
    let t = a.get("target").filter(|v| !v.is_null()).unwrap_or(a);
    ensure!(
        !(t["kind"] == "window" && a["scope"] == "desktop")
            && !(t["kind"] == "desktop" && a["scope"] == "window"),
        "conflicting target and scope"
    );
    if tool == "get_desktop_state" || t["kind"] == "desktop" || a["scope"] == "desktop" {
        ensure!(
            t["display_id"].as_str().is_none_or(|v| v == "primary"),
            "only primary desktop screenshot coordinates supported"
        );
        ensure!(
            a.get("pid").is_none() && a.get("window_id").is_none(),
            "conflicting desktop/window target"
        );
        return Ok(json!([session, "desktop"]).to_string());
    }
    let pid = t["pid"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("pixel mapping requires target pid"))?;
    let window = t["window_id"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("pixel mapping requires target window_id"))?;
    for (field, expected) in [("pid", pid), ("window_id", window)] {
        ensure!(
            a.get(field).is_none_or(|v| v.as_u64() == Some(expected)),
            "conflicting screenshot target"
        );
    }
    Ok(json!([session, "window", pid, window]).to_string())
}
impl Session {
    pub fn record(&mut self, tool: &str, args: &Value, frame: Option<Frame>) -> Result<()> {
        let key = target(tool, args)?;
        // Bound session memory without silently forgetting a still-actionable frame.
        ensure!(
            self.frames.contains_key(&key) || self.frames.len() < 1024,
            "screenshot frame capacity reached; restart backend"
        );
        self.frames.insert(key, frame);
        Ok(())
    }
    pub fn clear(&mut self) {
        self.frames.clear();
    }
    pub fn arguments(
        &self,
        tool: &str,
        args: &Value,
        coordinate_task: Option<&str>,
    ) -> Result<Value> {
        let fields: &[(&str, bool)] = match tool {
            "click" | "double_click" | "right_click" | "move_cursor" | "scroll" | "type_text" => {
                &[("x", true), ("y", false)]
            }
            "drag" => &[
                ("from_x", true),
                ("from_y", false),
                ("to_x", true),
                ("to_y", false),
            ],
            "zoom" => &[("x1", true), ("y1", false), ("x2", true), ("y2", false)],
            _ => {
                ensure!(
                    coordinate_task.is_none(),
                    "coordinate_task_id requires a pixel action"
                );
                return Ok(args.clone());
            }
        };
        if args["from_zoom"] == true
            || args.get("element_token").is_some()
            || args.get("element_index").is_some()
            || !fields.iter().any(|(k, _)| args.get(*k).is_some())
        {
            ensure!(
                coordinate_task.is_none(),
                "coordinate_task_id requires full screenshot pixel coordinates"
            );
            return Ok(args.clone());
        }
        let key = target(tool, args)?;
        let Some(stored) = self.frames.get(&key) else {
            bail!("fresh screenshot required for this backend session and target");
        };
        let Some(frame) = stored else {
            bail!("fresh screenshot required before pixel action");
        };
        ensure!(
            coordinate_task.is_none_or(|id| id == frame.task_id),
            "screenshot task superseded; observe again"
        );
        let mut mapped = args.clone();
        for (key, x_axis) in fields {
            let value = args[*key]
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("pixel action requires numeric {key}"))?;
            let (displayed, source) = if *x_axis {
                (frame.width, frame.source_width)
            } else {
                (frame.height, frame.source_height)
            };
            let upper_ok = if tool == "zoom" {
                value <= displayed as f64
            } else {
                value < displayed as f64
            };
            ensure!(
                value.is_finite() && value >= 0.0 && upper_ok,
                "pixel coordinate outside screenshot bounds"
            );
            mapped[*key] = json!(value * source as f64 / displayed as f64);
        }
        Ok(mapped)
    }
}
