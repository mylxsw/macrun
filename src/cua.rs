use crate::{
    model::{Fault, Outcome},
    process::Identity,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub run_id: String,
    pub identity: Identity,
    pub app_path: String,
    pub bundle_id: String,
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub id: String,
    pub run_id: String,
    pub window: u64,
    pub bounds: Value,
    pub raw_width: f64,
    pub raw_height: f64,
    pub tokens: Vec<String>,
}
pub struct Cua {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    sequence: u64,
    pub schemas: BTreeMap<String, Value>,
}
impl Cua {
    pub async fn connect(binary: &str, socket: Option<&str>) -> Outcome<Self> {
        let mut cmd = Command::new(binary);
        cmd.arg("mcp");
        if let Some(s) = socket {
            cmd.args(["--socket", s]);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(unavailable)?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut c = Self {
            child,
            input,
            output,
            sequence: 0,
            schemas: BTreeMap::new(),
        };
        c.rpc("initialize",json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"macrun","version":"0.1.0"}})).await?;
        c.input
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .await
            .map_err(unavailable)?;
        let tools = c.rpc("tools/list", json!({})).await?;
        for t in tools["tools"]
            .as_array()
            .ok_or_else(|| unavailable("missing tool schemas"))?
        {
            if let Some(n) = t["name"].as_str() {
                c.schemas.insert(n.into(), t["inputSchema"].clone());
            }
        }
        for name in [
            "list_windows",
            "get_window_state",
            "click",
            "type_text",
            "press_key",
            "scroll",
        ] {
            if !c.schemas.contains_key(name) {
                return Err(unavailable(format!("driver does not advertise {name}")));
            }
        }
        Ok(c)
    }
    async fn rpc(&mut self, method: &str, params: Value) -> Outcome<Value> {
        self.sequence += 1;
        let id = self.sequence;
        let mut data =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                .unwrap();
        data.push(b'\n');
        self.input.write_all(&data).await.map_err(unavailable)?;
        self.input.flush().await.map_err(unavailable)?;
        let response = tokio::time::timeout(Duration::from_secs(25), async {
            loop {
                let mut line = String::new();
                if self
                    .output
                    .read_line(&mut line)
                    .await
                    .map_err(unavailable)?
                    == 0
                {
                    return Err(unavailable(
                        "Cua MCP exited; check daemon and GUI permissions",
                    ));
                }
                let v: Value = serde_json::from_str(&line).map_err(unavailable)?;
                if v["id"] == id {
                    if !v["error"].is_null() {
                        return Err(unavailable(v["error"].to_string()));
                    }
                    return Ok(v["result"].clone());
                }
            }
        })
        .await;
        match response {
            Ok(r) => r,
            Err(_) => {
                let _ = self.child.kill().await;
                Err(unavailable("Cua response timed out; reconnect required"))
            }
        }
    }
    pub async fn call(&mut self, name: &str, mut args: Value) -> Outcome<Value> {
        if self
            .schemas
            .get(name)
            .and_then(|s| s["properties"].get("session"))
            .is_some()
        {
            args["session"] = json!("macrun-demo");
        }
        let v = self
            .rpc("tools/call", json!({"name":name,"arguments":args}))
            .await?;
        if v["isError"] == true {
            let code = v
                .pointer("/structuredContent/code")
                .and_then(Value::as_str)
                .unwrap_or("cua_refused");
            return Err(Fault::new(code, v.to_string()));
        }
        Ok(v)
    }
    pub async fn windows(&mut self, pid: u32) -> Outcome<Vec<Value>> {
        let v = self.call("list_windows", json!({"pid":pid})).await?;
        structured(&v)["windows"]
            .as_array()
            .cloned()
            .ok_or_else(|| unavailable("list_windows response has no windows"))
    }
    pub async fn observe(
        &mut self,
        run: &Run,
        window: u64,
        max_edge: u32,
        directory: &Path,
    ) -> Outcome<(Snapshot, Value, Vec<u8>)> {
        let v = self
            .call(
                "get_window_state",
                json!({"pid":run.identity.pid,"window_id":window}),
            )
            .await?;
        let state = structured(&v);
        if !state["screenshot_error"].is_null() || state["screenshot_frame_valid"] == false {
            return Err(Fault::new("capture_permission_denied", state.to_string()));
        }
        use base64::Engine;
        let image = v["content"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["type"] == "image"))
            .and_then(|v| v["data"].as_str())
            .ok_or_else(|| Fault::new("observation_failed", "Cua returned no inline image"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image)
            .map_err(unavailable)?;
        let raw = image::load_from_memory(&bytes).map_err(unavailable)?;
        let bounds = state["window_bounds"].clone();
        if bounds["width"].as_f64().unwrap_or(0.) <= 0.
            || bounds["height"].as_f64().unwrap_or(0.) <= 0.
        {
            return Err(Fault::new(
                "observation_failed",
                "missing window point geometry",
            ));
        }
        let resized = if raw.width().max(raw.height()) > max_edge {
            raw.thumbnail(max_edge, max_edge)
        } else {
            raw.clone()
        };
        let mut png = std::io::Cursor::new(Vec::new());
        resized
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(unavailable)?;
        let tokens = state["elements"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| e["element_token"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let snap = Snapshot {
            id: crate::model::id(),
            run_id: run.run_id.clone(),
            window,
            bounds: bounds.clone(),
            raw_width: raw.width() as f64,
            raw_height: raw.height() as f64,
            tokens,
        };
        let detail = json!({"run":run,"run_id":run.run_id,"window_id":window,"snapshot_id":snap.id,"observation_status":"succeeded",
            "screenshot":{"path":"screenshots/001.png","point_bounds":bounds,"origin":"top-left","width":resized.width(),"height":resized.height(),
            "pixels_per_point_x":resized.width() as f64/bounds["width"].as_f64().unwrap(),"pixels_per_point_y":resized.height() as f64/bounds["height"].as_f64().unwrap()},
            "accessibility_path":"accessibility.json"});
        std::fs::create_dir_all(directory).map_err(unavailable)?;
        Ok((
            snap,
            json!({"detail":detail,"accessibility":state}),
            png.into_inner(),
        ))
    }
    pub fn action_args(
        &self,
        tool: &str,
        run: &Run,
        snapshot: &Snapshot,
        args: &Value,
        foreground: bool,
    ) -> Outcome<Value> {
        let schema = &self.schemas[tool]["properties"];
        let mut a = if schema.get("target").is_some() {
            json!({"target":{"kind":"window","pid":run.identity.pid,"window_id":snapshot.window}})
        } else {
            json!({"pid":run.identity.pid,"window_id":snapshot.window})
        };
        if schema.get("delivery_mode").is_none() {
            return Err(unavailable(format!(
                "{tool} has no explicit delivery_mode; unsupported driver version"
            )));
        }
        a["delivery_mode"] = json!(if foreground {
            "foreground"
        } else {
            "background"
        });
        if let Some(e) = args["element"].as_str() {
            if !snapshot.tokens.iter().any(|t| t == e) {
                return Err(Fault::new("stale_snapshot", "element not in this snapshot"));
            }
            a["element_token"] = json!(e);
        }
        if let (Some(x), Some(y)) = (args["x"].as_f64(), args["y"].as_f64()) {
            let w = snapshot.bounds["width"].as_f64().unwrap();
            let h = snapshot.bounds["height"].as_f64().unwrap();
            if x < 0. || y < 0. || x >= w || y >= h {
                return Err(Fault::new(
                    "target_mismatch",
                    "coordinates outside observed window",
                ));
            }
            a["x"] = json!(x * snapshot.raw_width / w);
            a["y"] = json!(y * snapshot.raw_height / h);
        }
        for name in ["text", "key"] {
            if let Some(v) = args.get(name) {
                a[name] = v.clone();
            }
        }
        if tool == "scroll" {
            let dy = args["dy"].as_i64().unwrap_or(0);
            if dy == 0 || dy.unsigned_abs() > 100 {
                return Err(Fault::new(
                    "invalid_config",
                    "scroll --dy is signed wheel steps, 1..100",
                ));
            }
            a["direction"] = json!(if dy > 0 { "down" } else { "up" });
            a["amount"] = json!(dy.unsigned_abs());
            a["by"] = json!("line");
        }
        Ok(a)
    }
}
pub fn structured(v: &Value) -> Value {
    if let Some(s) = v.get("structuredContent") {
        return s.clone();
    }
    v["content"]
        .as_array()
        .and_then(|a| {
            a.iter().find_map(|x| {
                x["text"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
            })
        })
        .unwrap_or(Value::Null)
}
fn unavailable(e: impl ToString) -> Fault {
    Fault::new("cua_unavailable", e)
}
