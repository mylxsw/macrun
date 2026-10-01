//! Persistent stdio MCP client. A backend is serialized, but network reconnects do not recreate it.
use crate::{config::Backend, model::id, wire::MAX_FRAME};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};
pub struct Client {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    sequence: u64,
    pub generation: String,
}
impl Client {
    pub async fn connect(config: &Backend) -> Result<Self> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }
        let mut child = cmd.spawn()?;
        let mut c = Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            sequence: 0,
            generation: id(),
        };
        let r=c.rpc("initialize",json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"macrun","version":env!("CARGO_PKG_VERSION")}})).await?;
        if !["2024-11-05", "2025-03-26", "2025-06-18"]
            .contains(&r["protocolVersion"].as_str().unwrap_or(""))
        {
            bail!("unsupported backend MCP protocol");
        }
        c.write(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await?;
        Ok(c)
    }
    async fn write(&mut self, v: Value) -> Result<()> {
        let mut b = serde_json::to_vec(&v)?;
        if b.len() > MAX_FRAME {
            bail!("MCP message exceeds limit");
        }
        b.push(b'\n');
        self.input.write_all(&b).await?;
        self.input.flush().await?;
        Ok(())
    }
    pub async fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        self.sequence += 1;
        let seq = self.sequence;
        self.write(json!({"jsonrpc":"2.0","id":seq,"method":method,"params":params}))
            .await?;
        loop {
            let mut bytes = Vec::new();
            loop {
                let buffer = self.output.fill_buf().await?;
                if buffer.is_empty() {
                    bail!("backend exited; session lost");
                }
                let end = buffer.iter().position(|b| *b == b'\n').map(|i| i + 1);
                let n = end.unwrap_or(buffer.len());
                if bytes.len() + n > MAX_FRAME {
                    bail!("backend message exceeds limit; use file download for large artifacts");
                }
                bytes.extend_from_slice(&buffer[..n]);
                self.output.consume(n);
                if end.is_some() {
                    break;
                }
            }
            let v: Value = serde_json::from_slice(&bytes)?;
            if v.get("method").is_some() {
                if let Some(id) = v.get("id") {
                    self.write(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"client callbacks not supported"}})).await?;
                }
                continue;
            }
            if v["id"] == seq {
                if v.get("error").is_some() {
                    bail!("backend RPC error: {}", v["error"]);
                }
                return Ok(v["result"].clone());
            }
        }
    }
    pub async fn stop(&mut self) {
        let _ = self.child.kill().await;
    }
}
