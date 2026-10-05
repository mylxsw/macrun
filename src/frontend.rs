use crate::{
    model::{Reply, Request, id},
    wire,
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
pub async fn request(socket: &Path, workspace: &Path, kind: &str, args: Value) -> Result<Value> {
    request_for(
        socket,
        workspace,
        kind,
        args,
        std::env::var("MACRUN_CLIENT").ok().as_deref(),
    )
    .await
}
pub async fn request_for(
    socket: &Path,
    workspace: &Path,
    kind: &str,
    mut args: Value,
    client: Option<&str>,
) -> Result<Value> {
    if !args.is_object() {
        bail!("arguments must be a JSON object");
    }
    if ["exec.start", "mcp.call", "desktop.sequence"].contains(&kind)
        && args.get("request_id").is_none()
    {
        args["request_id"] = json!(id());
    }
    if let Some(client) = client
        && args.get("client_id").is_none()
    {
        args["client_id"] = json!(client);
    }
    let request_id = args.get("request_id").cloned();
    let mut stream = tokio::net::UnixStream::connect(socket).await?;
    wire::send(
        &mut stream,
        &Request {
            kind: kind.into(),
            workspace: workspace.into(),
            args,
        },
    )
    .await?;
    loop {
        let reply = wire::recv::<_, Reply>(&mut stream).await.map_err(|e| {
            anyhow::anyhow!("{e}; request_id={request_id:?}; outcome may be unknown")
        })?;
        match reply {
            Reply::Status { value } => return Ok(value),
            Reply::Accepted { job_id, .. } => {
                return Ok(json!({"job_id":job_id,"status":"accepted"}));
            }
            Reply::Error { error } => bail!("{error}; request_id={request_id:?}"),
            Reply::Done { result, .. } => return Ok(result),
            Reply::Log { .. } => {}
        }
    }
}
fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}
pub fn tools() -> Vec<Value> {
    let string = json!({"type":"string"});
    let number = json!({"type":"integer","minimum":0});
    let mut tools = vec![
        tool(
            "device_status",
            "Inspect the outbound-connected worker and latest directory sync.",
            json!({}),
            &[],
        ),
        tool(
            "exec_start",
            "Start a worker-owned shell command. Optional wait_ms (up to 1000) returns a completed result when ready. Save task_id; poll task_get. Reuse the same UUID request_id after uncertain delivery, never a new ID. No implicit synchronization.",
            json!({"command":string,"cwd":string,"env":{"type":"object","additionalProperties":{"type":"string"}},"timeout_seconds":{"type":"integer","minimum":1},"request_id":string,"wait_ms":{"type":"integer","minimum":0,"maximum":1000},"workspace_root":string,"generation":string,"snapshot":{"type":"boolean"}}),
            &["command", "cwd", "request_id"],
        ),
        tool(
            "task_get",
            "Read task state, result and incremental combined stdout/stderr (byte offset). Completed MCP image content is returned as images. Unknown means inspect effects, do not replay.",
            json!({"task_id":string,"offset":number,"include_result":{"type":"boolean"},"wait_ms":{"type":"integer","minimum":0,"maximum":25000}}),
            &["task_id"],
        ),
        tool(
            "task_cancel",
            "Request cancellation. Commands terminate their process group; MCP cancellation invalidates its session and effects may be unknown.",
            json!({"task_id":string}),
            &["task_id"],
        ),
        tool(
            "file_list",
            "List a remote directory (pages of 1000 entries).",
            json!({"path":string,"offset":number}),
            &["path"],
        ),
        tool(
            "file_read",
            "Read up to 512 KiB of a remote regular file. Base64 bytes, optional text, byte offsets and version for chunked download.",
            json!({"path":string,"offset":number,"length":number,"text":{"type":"boolean"}}),
            &["path"],
        ),
        tool(
            "file_image",
            "Return a remote PNG/JPEG as MCP image content (maximum 8 MiB).",
            json!({"path":string}),
            &["path"],
        ),
        tool(
            "file_write",
            "Write up to 512 KiB using text or base64 data. Offset 0 truncates by default; subsequent chunks use explicit offsets. Stage uploads under a temporary path then file_move.",
            json!({"path":string,"text":string,"data":string,"offset":number,"truncate":{"type":"boolean"}}),
            &["path"],
        ),
        tool(
            "file_move",
            "Rename a remote file, e.g. publish a completed staged upload. Replaces destination.",
            json!({"path":string,"destination":string}),
            &["path", "destination"],
        ),
        tool(
            "mcp_servers",
            "List configured local stdio MCP backends on the worker.",
            json!({}),
            &[],
        ),
        tool(
            "mcp_tools",
            "Discover tools and obtain backend session ID; supports cursor pagination. Same session survives network reconnects, but not backend/worker restart.",
            json!({"server":string,"cursor":string}),
            &["server"],
        ),
        tool(
            "mcp_call",
            "Start a local MCP tool call. Optional wait_ms (up to 1000) waits briefly for completion. Requires session from mcp_tools. Poll task_get. Calls to one backend execute serially. No Cua-specific changes to arguments.",
            json!({"server":string,"tool":string,"arguments":{"type":"object"},"session":string,"timeout_seconds":{"type":"integer","minimum":1},"request_id":string,"wait_ms":{"type":"integer","minimum":0,"maximum":1000}}),
            &["server", "tool", "session", "request_id"],
        ),
        tool(
            "desktop_sequence",
            "Execute up to 16 ordered backend steps with a shared resource lease, per-step approval and stop-on-error/condition. Never replay after unknown. Returns a task ID.",
            json!({"server":string,"session":string,"request_id":string,"steps":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"object","properties":{"tool":string,"arguments":{"type":"object"},"expect":{"type":"object","properties":{"pointer":string,"equals":{}}}},"required":["tool"]}},"timeout_seconds":{"type":"integer","minimum":1,"maximum":600},"wait_ms":{"type":"integer","minimum":0,"maximum":1000}}),
            &["server", "session", "request_id", "steps"],
        ),
        tool(
            "sync_start",
            "Start one directory synchronization from Linux workspace/macrun.toml. Poll sync_get until succeeded before executing commands. Continuous synchronization uses the sync --watch CLI.",
            json!({"workspace":string,"strict":{"type":"boolean"}}),
            &[],
        ),
        tool(
            "sync_get",
            "Read a directory synchronization result by job_id; distinct from worker task_id.",
            json!({"job_id":string}),
            &["job_id"],
        ),
    ];
    for tool in &mut tools {
        tool["inputSchema"]["properties"]["client_id"] = json!({"type":"string","description":"Target client_id from device_status. Required when multiple clients are connected; never broadcast."});
    }
    tools
}
pub fn content(v: &Value) -> Value {
    let mut summary = v.clone();
    let embedded = if v.get("content").is_some() {
        summary.as_object_mut().unwrap().remove("content")
    } else {
        summary
            .pointer_mut("/result/result")
            .and_then(Value::as_object_mut)
            .and_then(|o| o.remove("content"))
    };
    let mut items = vec![json!({"type":"text","text":serde_json::to_string(&summary).unwrap()})];
    if let Some(Value::Array(c)) = embedded {
        items.extend(c);
    }
    json!({"content":items,"isError":matches!(v["status"].as_str(),Some("failed"|"unknown"|"cancelled"|"timed_out"|"denied"))})
}
pub async fn mcp(socket: PathBuf, workspace: PathBuf) -> Result<()> {
    mcp_for(socket, workspace, std::env::var("MACRUN_CLIENT").ok()).await
}
#[derive(Default)]
struct ImageCache {
    values: std::collections::BTreeMap<String, String>,
    bytes: usize,
}
type SharedCache = std::sync::Arc<tokio::sync::Mutex<ImageCache>>;
fn references(value: &Value, out: &mut Vec<Value>) {
    match value {
        Value::Object(v) => {
            if v.get("type").and_then(Value::as_str) == Some("image")
                && let Some(a) = v.get("artifact")
            {
                out.push(a.clone());
            }
            for v in v.values() {
                references(v, out);
            }
        }
        Value::Array(a) => {
            for v in a {
                references(v, out);
            }
        }
        _ => {}
    }
}
fn insert_image(value: &mut Value, id: &str, data: &str) {
    match value {
        Value::Object(v) => {
            if v.get("type").and_then(Value::as_str) == Some("image")
                && v.get("artifact").is_some_and(|r| r["id"] == id)
            {
                v.remove("artifact");
                v.insert("data".into(), json!(data));
            }
            for v in v.values_mut() {
                insert_image(v, id, data);
            }
        }
        Value::Array(a) => {
            for v in a {
                insert_image(v, id, data);
            }
        }
        _ => {}
    }
}
async fn hydrate(
    value: &mut Value,
    socket: &Path,
    root: &Path,
    client: Option<&str>,
    cache: &SharedCache,
) -> Result<()> {
    use base64::Engine;
    let mut refs = Vec::new();
    references(value, &mut refs);
    let task = value["task_id"].as_str().unwrap_or("").to_owned();
    for reference in refs {
        let id = reference["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("invalid artifact reference"))?;
        let key = format!("{}:{task}:{id}", client.unwrap_or(""));
        let mut cached = cache.lock().await;
        let data = if let Some(data) = cached.values.get(&key) {
            data.clone()
        } else {
            let mut bytes = Vec::new();
            crate::transfer::download(
                socket,
                root,
                "artifact.download",
                json!({"task_id":task,"artifact_id":id}),
                client,
                &mut bytes,
            )
            .await?;
            anyhow::ensure!(
                blake3::hash(&bytes).to_hex().as_str() == id,
                "artifact hash mismatch"
            );
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            if cached.bytes + data.len() > 32 * 1024 * 1024 {
                cached.values.clear();
                cached.bytes = 0;
            }
            cached.bytes += data.len();
            cached.values.insert(key, data.clone());
            data
        };
        drop(cached);
        insert_image(value, id, &data);
    }
    Ok(())
}
pub async fn mcp_for(socket: PathBuf, workspace: PathBuf, client: Option<String>) -> Result<()> {
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    let mut initialized = false;
    let mut closed = false;
    let mut pending = tokio::task::JoinSet::new();
    let cache = SharedCache::default();
    loop {
        let response = tokio::select! {
            line=input.next_line(),if !closed && pending.len()<16=>{
                let Some(line)=line? else {closed=true;continue};
                match serde_json::from_str::<Value>(&line) {
                    Ok(v) if v.get("id").is_some()=>{
                        let ready=initialized;
                        if v["method"]=="initialize" {initialized=true;}
                        let (s,w,c,cache)=(socket.clone(),workspace.clone(),client.clone(),cache.clone());
                        pending.spawn(async move {rpc_response(s,w,c,v,ready,cache).await});
                        continue;
                    },
                    Ok(_)=>continue,
                    Err(_)=>json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
                }
            },
            response=pending.join_next(),if !pending.is_empty()=>response.unwrap()?,
            else=>break,
        };
        let mut bytes = serde_json::to_vec(&response)?;
        bytes.push(b'\n');
        out.write_all(&bytes).await?;
        out.flush().await?;
    }
    Ok(())
}
async fn rpc_response(
    socket: PathBuf,
    workspace: PathBuf,
    client: Option<String>,
    v: Value,
    initialized: bool,
    cache: SharedCache,
) -> Value {
    let ident = &v["id"];
    let method = v["method"].as_str().unwrap_or("");
    let r = match method {
        "initialize" => Ok(
            json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"macrun","version":env!("CARGO_PKG_VERSION")},"instructions":"Generic remote worker. Save request IDs, poll tasks, explicitly wait for sync, re-observe UI after session change. No automatic replay."}),
        ),
        "ping" => Ok(json!({})),
        _ if !initialized => Err((-32000, "initialize first".to_string())),
        "tools/list" => Ok(json!({"tools":tools()})),
        "tools/call" => {
            let name = v["params"]["name"].as_str().unwrap_or("");
            let mut args = v["params"].get("arguments").cloned().unwrap_or(json!({}));
            let spec = tools().into_iter().find(|t| t["name"] == name);
            if let Some(spec) = spec {
                let valid = args.is_object()
                    && spec["inputSchema"]["required"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|k| args.get(k.as_str().unwrap()).is_some());
                if !valid {
                    Err((-32602, "missing required arguments".into()))
                } else {
                    let mut kind = match name {
                        "device_status" => "status".into(),
                        "sync_start" => "sync".into(),
                        "task_get" if args["wait_ms"].as_u64().unwrap_or(0) > 0 => {
                            "task.wait".into()
                        }
                        other => other.replacen('_', ".", 1),
                    };
                    let root = args["workspace"]
                        .as_str()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| workspace.clone());
                    if kind == "sync" {
                        args["detach"] = json!(true);
                    }
                    let target = args["client_id"]
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| client.clone());
                    let enhanced = matches!(
                        kind.as_str(),
                        "task.get" | "task.wait" | "exec.start" | "mcp.call" | "desktop.sequence"
                    ) && crate::transfer::capable(&socket, &root, target.as_deref())
                        .await
                        .unwrap_or(false);
                    if enhanced {
                        args["artifact_refs"] = json!(true);
                    } else if kind == "task.wait" {
                        kind = "task.get".into();
                        args.as_object_mut().unwrap().remove("wait_ms");
                    }
                    match request_for(&socket, &root, &kind, args, client.as_deref()).await {
                        Ok(mut value) => {
                            match hydrate(&mut value, &socket, &root, target.as_deref(), &cache)
                                .await
                            {
                                Ok(()) => Ok(content(&value)),
                                Err(e) => Ok(
                                    json!({"content":[{"type":"text","text":e.to_string()}],"isError":true}),
                                ),
                            }
                        }
                        Err(e) => Ok(
                            json!({"content":[{"type":"text","text":e.to_string()}],"isError":true}),
                        ),
                    }
                }
            } else {
                Err((-32602, "unknown tool".into()))
            }
        }
        _ => Err((-32601, "method not supported".into())),
    };
    match r {
        Ok(result) => json!({"jsonrpc":"2.0","id":ident,"result":result}),
        Err((code, message)) => {
            json!({"jsonrpc":"2.0","id":ident,"error":{"code":code,"message":message}})
        }
    }
}
