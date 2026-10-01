use anyhow::{Context, Result, bail};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use serde::{Serialize, de::DeserializeOwned};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
pub const MAX_FRAME: usize = 16 * 1024 * 1024;
pub async fn send<W: AsyncWrite + Unpin, T: Serialize>(w: &mut W, v: &T) -> Result<()> {
    let bytes = serde_json::to_vec(v)?;
    if bytes.len() > MAX_FRAME {
        bail!("message exceeds frame limit");
    }
    w.write_u32(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}
pub async fn recv<R: AsyncRead + Unpin, T: DeserializeOwned>(r: &mut R) -> Result<T> {
    let n = r.read_u32().await? as usize;
    if n > MAX_FRAME {
        bail!("message exceeds frame limit");
    }
    let mut b = vec![0; n];
    r.read_exact(&mut b).await?;
    Ok(serde_json::from_slice(&b)?)
}
fn transport() -> Arc<quinn::TransportConfig> {
    let mut c = quinn::TransportConfig::default();
    c.keep_alive_interval(Some(Duration::from_secs(5)));
    c.max_idle_timeout(Some(Duration::from_secs(15).try_into().unwrap()));
    Arc::new(c)
}
pub fn init_tls(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    if dir.join("cert.der").exists() || dir.join("key.der").exists() {
        bail!("TLS identity already exists; refusing overwrite");
    }
    let cert = rcgen::generate_simple_self_signed(vec!["macrun".into()])?;
    std::fs::write(dir.join("cert.der"), cert.cert.der())?;
    private_write(&dir.join("key.der"), &cert.key_pair.serialize_der())?;
    private_write(
        &dir.join("token"),
        uuid::Uuid::new_v4().to_string().as_bytes(),
    )?;
    Ok(())
}
pub fn private_write(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}
pub fn server(addr: std::net::SocketAddr, dir: &Path) -> Result<quinn::Endpoint> {
    let cert = CertificateDer::from(std::fs::read(dir.join("cert.der"))?);
    let key = PrivatePkcs8KeyDer::from(std::fs::read(dir.join("key.der"))?);
    let mut cfg = quinn::ServerConfig::with_single_cert(vec![cert], key.into())?;
    cfg.transport = transport();
    Ok(quinn::Endpoint::server(cfg, addr)?)
}
pub fn client(cert: &Path) -> Result<quinn::Endpoint> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(std::fs::read(cert)?))?;
    let mut cfg = quinn::ClientConfig::with_root_certificates(Arc::new(roots))?;
    cfg.transport_config(transport());
    let mut ep = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    ep.set_default_client_config(cfg);
    Ok(ep)
}
pub fn lock(dir: &Path) -> Result<std::fs::File> {
    std::fs::create_dir_all(dir)?;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("process.lock"))?;
    fs2::FileExt::try_lock_exclusive(&f).context("another process owns this state directory")?;
    Ok(f)
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let temp = path.with_extension("tmp");
    private_write(&temp, &serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

// A framed read must not be dropped halfway through a header/body by select!.
// One dedicated reader owns framing; receiving complete messages is cancellation-safe.
pub fn read_channel<R, T>(mut reader: R) -> tokio::sync::mpsc::Receiver<Result<T>>
where
    R: AsyncRead + Unpin + Send + 'static,
    T: DeserializeOwned + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    tokio::spawn(async move {
        loop {
            let message = recv(&mut reader).await;
            let failed = message.is_err();
            if tx.send(message).await.is_err() || failed {
                break;
            }
        }
    });
    rx
}
