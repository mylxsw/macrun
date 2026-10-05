//! One operation per binary transfer. Frames delimit metadata; bodies are raw bytes.
use crate::{
    engine::Engine,
    files::string,
    model::{Fault, Reply, Request},
    wire,
};
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

pub fn supported(kind: &str) -> bool {
    matches!(kind, "file.download" | "file.upload" | "artifact.download")
}
pub async fn response<R: AsyncRead + Unpin>(r: &mut R) -> Result<Value> {
    match wire::recv::<_, Reply>(r).await? {
        Reply::Status { value } => Ok(value),
        Reply::Error { error } => Err(error.into()),
        _ => bail!("unexpected transfer response"),
    }
}
async fn status<W: AsyncWrite + Unpin>(w: &mut W, value: Value) -> Result<()> {
    wire::send(w, &Reply::Status { value }).await
}

/// No buffer grows with the payload. Idle timeout is renewed only by useful I/O.
pub async fn copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    r: &mut R,
    w: &mut W,
    mut bytes: u64,
    cancel: &CancellationToken,
) -> Result<String> {
    let mut buffer = vec![0u8; 256 * 1024];
    let mut hash = blake3::Hasher::new();
    while bytes > 0 {
        let n = bytes.min(buffer.len() as u64) as usize;
        let read = tokio::select! {
            r=tokio::time::timeout(Duration::from_secs(30),r.read(&mut buffer[..n]))=>r.map_err(|_|anyhow::anyhow!("transfer read idle timeout"))??,
            _=cancel.cancelled()=>bail!("transfer cancelled"),
        };
        ensure!(read > 0, "truncated transfer");
        tokio::select! {
            r=tokio::time::timeout(Duration::from_secs(30),w.write_all(&buffer[..read]))=>r.map_err(|_|anyhow::anyhow!("transfer write idle timeout"))??,
            _=cancel.cancelled()=>bail!("transfer cancelled"),
        }
        hash.update(&buffer[..read]);
        bytes -= read as u64;
    }
    w.flush().await?;
    Ok(hash.finalize().to_hex().to_string())
}

pub async fn serve<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    kind: &str,
    args: &Value,
    input: &mut R,
    out: &mut W,
    engine: &Arc<Engine>,
) -> Result<bool> {
    static SLOTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    let _slot = SLOTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(8)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| Fault::new("busy", "transfer capacity reached"))?;
    // Artifact IDs are scoped to this Engine's tasks, never arbitrary worker paths.
    let started = std::time::Instant::now();
    let artifact = kind == "artifact.download";
    let path = if artifact {
        crate::artifact::path(
            &engine.directory(string(args, "task_id")?)?,
            string(args, "artifact_id")?,
        )?
    } else {
        crate::config::expand(string(args, "path")?)
    };
    let record = if artifact {
        None
    } else {
        Some(engine.begin_external(kind, json!({"path":path})).await?)
    };
    let cancel = record.as_ref().map(|(_, c)| c.clone()).unwrap_or_default();
    let operation = async {
        let _lease = if kind == "file.upload" {
            Some(crate::workspace::Lease::acquire(std::slice::from_ref(
                &path,
            ))?)
        } else {
            None
        };
        if kind == "file.upload" {
            let size = args["size"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("size required"))?;
            let expected = string(args, "hash")?;
            ensure!(expected.len() == 64, "hash required");
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let transfer_id = args["transfer_id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(crate::model::id);
            uuid::Uuid::parse_str(&transfer_id)?;
            let temporary = path.with_file_name(format!(
                "{}.{}.macrun-part",
                path.file_name().unwrap().to_string_lossy(),
                transfer_id
            ));
            let journal = temporary.with_extension("macrun-meta");
            let identity = json!({"path":path,"size":size,"hash":expected});
            if journal.exists() {
                let previous: Value = serde_json::from_slice(&tokio::fs::read(&journal).await?)?;
                ensure!(
                    previous == identity,
                    "transfer_id belongs to different content or destination"
                );
            } else {
                wire::persist(&journal, &identity).await?;
            }
            let result=async {
                if let Ok(meta)=tokio::fs::symlink_metadata(&temporary).await {ensure!(meta.is_file(),"invalid transfer staging file");}
                let mut file=tokio::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&temporary).await?;
                let offset=file.metadata().await?.len();
                ensure!(offset<=size,"staging file exceeds expected size");
                let p=temporary.clone();
                let prefix=crate::wire::blocking(move||crate::sync::hash(&p)).await??;
                use tokio::io::AsyncSeekExt;
                file.seek(std::io::SeekFrom::Start(offset)).await?;
                status(out,json!({"ready":true,"size":size-offset,"offset":offset,"prefix_hash":prefix,"transfer_id":transfer_id})).await?;
                copy(input,&mut file,size-offset,&cancel).await?;
                file.sync_all().await?;
                drop(file);
                let p=temporary.clone();
                let hash=crate::wire::blocking(move||crate::sync::hash(&p)).await??;
                ensure!(hash==expected,"transfer checksum mismatch");
                tokio::fs::rename(&temporary,&path).await?;
                tokio::fs::remove_file(&journal).await?;
                Ok::<_,anyhow::Error>(json!({"bytes":size,"transferred_bytes":size-offset,"hash":hash,"transfer_id":transfer_id}))
            }.await;
            if result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("checksum mismatch"))
                || (result.is_err() && args.get("transfer_id").is_none())
            {
                let _ = tokio::fs::remove_file(&temporary).await;
                let _ = tokio::fs::remove_file(&journal).await;
            }
            result
        } else {
            let mut file = tokio::fs::File::open(&path).await?;
            let before = file.metadata().await?;
            ensure!(before.is_file(), "path is not a regular file");
            let offset = args["offset"].as_u64().unwrap_or(0);
            ensure!(offset <= before.len(), "offset exceeds file length");
            use tokio::io::AsyncSeekExt;
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            let version = format!("{}:{:?}", before.len(), before.modified()?);
            if let Some(expected) = args["version"].as_str() {
                ensure!(expected == version, "source_changed: file version differs");
            }
            status(
                out,
                json!({"ready":true,"size":before.len()-offset,"version":version,"offset":offset}),
            )
            .await?;
            let hash = copy(&mut file, out, before.len() - offset, &cancel).await?;
            let after = file.metadata().await?;
            use std::os::unix::fs::MetadataExt;
            ensure!(
                before.len() == after.len()
                    && before.mtime() == after.mtime()
                    && before.mtime_nsec() == after.mtime_nsec()
                    && before.ctime() == after.ctime()
                    && before.ctime_nsec() == after.ctime_nsec(),
                "source_changed during download"
            );
            Ok(json!({"bytes":before.len()-offset,"hash":hash,"version":version}))
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(3600), operation)
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("transfer overall timeout")));
    crate::logging::event(
        "worker",
        "transfer_finished",
        json!({"operation":kind,"duration_ms":started.elapsed().as_millis(),"status":if result.is_ok(){"succeeded"}else{"failed"},"bytes":result.as_ref().ok().map(|v|&v["bytes"]),"transferred_bytes":result.as_ref().ok().map(|v|&v["transferred_bytes"])}),
    );
    if let Some((ident, _)) = record {
        engine
            .finish_external(&ident, result.as_ref().err().map(ToString::to_string), None)
            .await?;
    }
    match result {
        Ok(value) => {
            status(out, value).await?;
            Ok(true)
        }
        Err(error) => {
            wire::send(
                out,
                &Reply::Error {
                    error: Fault::new("transfer_failed", error),
                },
            )
            .await?;
            Ok(false)
        }
    }
}

pub async fn relay(
    socket: &mut tokio::net::UnixStream,
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    kind: &str,
) -> Result<bool> {
    let ready = wire::recv::<_, Reply>(recv).await?;
    let size = match &ready {
        Reply::Status { value } if value["ready"] == true => value["size"].as_u64(),
        _ => None,
    };
    wire::send(socket, &ready).await?;
    let Some(size) = size else { return Ok(false) };
    let cancel = CancellationToken::new();
    if kind == "file.upload" {
        copy(socket, send, size, &cancel).await?;
        send.finish()?;
    } else {
        copy(recv, socket, size, &cancel).await?;
    }
    let reply: Reply = wire::recv(recv).await?;
    wire::send(socket, &reply).await?;
    Ok(matches!(reply, Reply::Status { .. }))
}

pub async fn capable(socket: &Path, workspace: &Path, client: Option<&str>) -> Result<bool> {
    let status =
        crate::frontend::request_for(socket, workspace, "status", json!({}), client).await?;
    Ok(status["server_capabilities"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v == "binary_transfer_v1"))
        && status["worker"]["capabilities"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "binary_transfer_v1")))
}
async fn connect(
    socket: &Path,
    workspace: &Path,
    kind: &str,
    mut args: Value,
    client: Option<&str>,
) -> Result<tokio::net::UnixStream> {
    if let Some(client) = client {
        args["client_id"] = json!(client);
    }
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
    Ok(stream)
}
pub async fn download<W: AsyncWrite + Unpin>(
    socket: &Path,
    workspace: &Path,
    kind: &str,
    args: Value,
    client: Option<&str>,
    out: &mut W,
) -> Result<Value> {
    let mut stream = connect(socket, workspace, kind, args, client).await?;
    let ready = response(&mut stream).await?;
    let size = ready["size"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("missing transfer size"))?;
    if kind == "artifact.download" {
        ensure!(
            size <= wire::MAX_FRAME as u64,
            "artifact exceeds memory budget"
        );
    }
    let hash = copy(&mut stream, out, size, &CancellationToken::new()).await?;
    let end = response(&mut stream).await?;
    ensure!(end["hash"] == hash, "download checksum mismatch");
    Ok(end)
}
pub async fn upload(
    socket: &Path,
    workspace: &Path,
    local: &Path,
    remote: &str,
    client: Option<&str>,
    transfer_id: &str,
) -> Result<Value> {
    uuid::Uuid::parse_str(transfer_id)?;
    let p = local.to_path_buf();
    let hash = crate::wire::blocking(move || crate::sync::hash(&p)).await??;
    let mut file = tokio::fs::File::open(local).await?;
    let size = file.metadata().await?.len();
    let mut stream = connect(
        socket,
        workspace,
        "file.upload",
        json!({"path":remote,"size":size,"hash":hash,"transfer_id":transfer_id}),
        client,
    )
    .await?;
    let ready = response(&mut stream).await?;
    let offset = ready["offset"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("missing resume offset"))?;
    ensure!(offset <= size, "invalid resume offset");
    let p = local.to_path_buf();
    let prefix = crate::wire::blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(p)?.take(offset);
        let mut hash = blake3::Hasher::new();
        let mut b = [0u8; 65536];
        loop {
            let n = file.read(&mut b)?;
            if n == 0 {
                break;
            }
            hash.update(&b[..n]);
        }
        Ok::<_, anyhow::Error>(hash.finalize().to_hex().to_string())
    })
    .await??;
    ensure!(
        ready["prefix_hash"] == prefix,
        "resume prefix mismatch; retry with a new transfer ID"
    );
    use tokio::io::AsyncSeekExt;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    copy(
        &mut file,
        &mut stream,
        size - offset,
        &CancellationToken::new(),
    )
    .await?;
    let end = response(&mut stream).await?;
    ensure!(end["hash"] == hash, "upload checksum mismatch");
    Ok(end)
}
