//! Private, same-user worker control. No control mutations are exposed over QUIC.
use crate::{engine::Engine, model::now, wire};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{UnixListener, UnixStream},
    sync::watch,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionState {
    pub state: String,
    pub server: String,
    pub since: u64,
    pub rtt_ms: Option<u64>,
    pub error: Option<String>,
}
impl ConnectionState {
    pub fn initial(server: String) -> Self {
        Self {
            state: "connecting".into(),
            server,
            since: now(),
            rtt_ms: None,
            error: None,
        }
    }
}
pub async fn request(socket: &Path, action: &str, args: Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut stream = UnixStream::connect(socket).await?;
        wire::send(&mut stream, &json!({"action":action,"args":args})).await?;
        let v: Value = wire::recv(&mut stream).await?;
        if let Some(error) = v.get("error") {
            bail!("{}", error.as_str().unwrap_or("local control failed"));
        }
        Ok(v)
    })
    .await?
}
pub async fn serve(
    socket: PathBuf,
    engine: Arc<Engine>,
    connection: watch::Receiver<ConnectionState>,
    shutdown: CancellationToken,
) -> Result<()> {
    let parent = socket
        .parent()
        .ok_or_else(|| anyhow::anyhow!("socket needs parent directory"))?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let meta = std::fs::symlink_metadata(parent)?;
    if meta.file_type().is_symlink() {
        bail!("control directory cannot be a symlink");
    }
    if meta.uid() != unsafe { libc::geteuid() } || meta.permissions().mode() & 0o077 != 0 {
        bail!("control socket requires a dedicated, user-owned 0700 directory");
    }
    let _socket_lock = wire::lock(parent)?;
    if let Ok(meta) = std::fs::symlink_metadata(&socket) {
        if !meta.file_type().is_socket() {
            bail!("control path exists and is not a socket");
        }
        if UnixStream::connect(&socket).await.is_ok() {
            bail!("control socket is already in use");
        }
        std::fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    loop {
        tokio::select! {
            _=shutdown.cancelled()=>break,
            accepted=listener.accept()=>{
                let (stream,_) = accepted?;
                if stream.peer_cred()?.uid() != unsafe { libc::geteuid() } { continue; }
                let e=engine.clone(); let c=connection.clone(); let s=shutdown.clone();
                tokio::spawn(async move { let _=handle(stream,e,c,s).await; });
            }
        }
    }
    let _ = std::fs::remove_file(socket);
    Ok(())
}
async fn snapshot(engine: &Engine, connection: &watch::Receiver<ConnectionState>) -> Result<Value> {
    let mut v = engine.local_snapshot().await?;
    v["connection"] = json!(connection.borrow().clone());
    Ok(v)
}
async fn handle(
    mut stream: UnixStream,
    engine: Arc<Engine>,
    connection: watch::Receiver<ConnectionState>,
    shutdown: CancellationToken,
) -> Result<()> {
    let req: Value =
        tokio::time::timeout(Duration::from_secs(5), wire::recv(&mut stream)).await??;
    let a = &req["args"];
    if req["action"] == "subscribe" {
        let mut changes = engine.subscribe();
        let mut connection = connection;
        let mut tick = tokio::time::interval(Duration::from_millis(750));
        loop {
            tokio::select! {
                _=shutdown.cancelled()=>break,
                _=tick.tick()=>{},
                r=changes.changed()=>{r?;},
                r=connection.changed()=>{r?;},
            }
            let v = snapshot(&engine, &connection).await?;
            tokio::time::timeout(Duration::from_secs(3), wire::send(&mut stream, &v)).await??;
        }
        return Ok(());
    }
    let r: Result<Value> = async {
        match req["action"].as_str().unwrap_or("") {
            "snapshot" => snapshot(&engine, &connection).await,
            "pause" => {
                engine
                    .set_policy(
                        Some(
                            a["paused"]
                                .as_bool()
                                .ok_or_else(|| anyhow::anyhow!("paused must be boolean"))?,
                        ),
                        None,
                        false,
                    )
                    .await
            }
            "desktop" => {
                engine
                    .set_policy(
                        None,
                        Some(
                            a["enabled"]
                                .as_bool()
                                .ok_or_else(|| anyhow::anyhow!("enabled must be boolean"))?,
                        ),
                        false,
                    )
                    .await
            }
            "stop_all" => engine.set_policy(None, None, true).await,
            "cancel" => engine.handle("task.cancel", a.clone()).await,
            "tools" => engine.handle("mcp.tools", a.clone()).await,
            "shutdown" => {
                engine.shutdown().await?;
                shutdown.cancel();
                Ok(json!({"stopped":true}))
            }
            _ => bail!("unsupported local action"),
        }
    }
    .await;
    let reply = r.unwrap_or_else(|e| json!({"error":e.to_string()}));
    wire::send(&mut stream, &reply).await?;
    Ok(())
}
