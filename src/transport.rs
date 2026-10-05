//! QUIC remains the default. Explicit TCP/TLS uses bounded Yamux streams.
use anyhow::{Result, bail};
use std::{
    io,
    path::Path,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf},
    sync::{Mutex, mpsc, oneshot},
};
use tokio_util::{
    compat::{Compat, FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt},
    sync::CancellationToken,
};
type MuxStream = Compat<yamux::Stream>;
type Reader = tokio::io::ReadHalf<MuxStream>;
type Writer = tokio::io::WriteHalf<MuxStream>;
type Open = oneshot::Sender<Result<yamux::Stream>>;

#[derive(Clone)]
pub enum Connection {
    Quic(quinn::Connection),
    Tcp(Arc<Tcp>),
}
pub struct Tcp {
    open: mpsc::Sender<Open>,
    incoming: Mutex<mpsc::Receiver<yamux::Stream>>,
    stop: CancellationToken,
    closed: CancellationToken,
}
impl Drop for Tcp {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
pub enum SendStream {
    Quic(quinn::SendStream),
    Tcp {
        writer: Option<Writer>,
        done: CancellationToken,
    },
}
pub enum RecvStream {
    Quic(quinn::RecvStream),
    Tcp(Option<Reader>),
}
fn split(stream: yamux::Stream) -> (SendStream, RecvStream) {
    let (r, w) = tokio::io::split(stream.compat());
    (
        SendStream::Tcp {
            writer: Some(w),
            done: CancellationToken::new(),
        },
        RecvStream::Tcp(Some(r)),
    )
}
impl Connection {
    pub async fn open_bi(&self) -> Result<(SendStream, RecvStream)> {
        match self {
            Self::Quic(c) => {
                let (s, r) = c.open_bi().await?;
                Ok((SendStream::Quic(s), RecvStream::Quic(r)))
            }
            Self::Tcp(c) => {
                let (tx, rx) = oneshot::channel();
                c.open.send(tx).await?;
                Ok(split(rx.await??))
            }
        }
    }
    pub async fn accept_bi(&self) -> Result<(SendStream, RecvStream)> {
        match self {
            Self::Quic(c) => {
                let (s, r) = c.accept_bi().await?;
                Ok((SendStream::Quic(s), RecvStream::Quic(r)))
            }
            Self::Tcp(c) => Ok(split(
                c.incoming
                    .lock()
                    .await
                    .recv()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("TCP connection closed"))?,
            )),
        }
    }
    pub async fn open_uni(&self) -> Result<SendStream> {
        match self {
            Self::Quic(c) => Ok(SendStream::Quic(c.open_uni().await?)),
            Self::Tcp(_) => bail!("TCP requires negotiated bidirectional sync"),
        }
    }
    pub async fn accept_uni(&self) -> Result<RecvStream> {
        match self {
            Self::Quic(c) => Ok(RecvStream::Quic(c.accept_uni().await?)),
            Self::Tcp(_) => bail!("TCP requires negotiated bidirectional sync"),
        }
    }
    pub fn close(&self, code: quinn::VarInt, reason: &[u8]) {
        match self {
            Self::Quic(c) => c.close(code, reason),
            Self::Tcp(c) => c.stop.cancel(),
        }
    }
    pub async fn closed(&self) {
        match self {
            Self::Quic(c) => {
                c.closed().await;
            }
            Self::Tcp(c) => c.closed.cancelled().await,
        }
    }
    pub fn rtt(&self) -> Option<Duration> {
        match self {
            Self::Quic(c) => Some(c.rtt()),
            Self::Tcp(_) => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Quic(_) => "quic",
            Self::Tcp(_) => "tcp_tls",
        }
    }
}
impl SendStream {
    pub fn set_priority(&self, p: i32) -> Result<()> {
        if let Self::Quic(s) = self {
            s.set_priority(p)?;
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<()> {
        match self {
            Self::Quic(s) => {
                s.finish()?;
            }
            Self::Tcp { writer, done } => {
                if let Some(mut w) = writer.take() {
                    let done = done.clone();
                    tokio::spawn(async move {
                        let _ = w.shutdown().await;
                        done.cancel();
                    });
                }
            }
        }
        Ok(())
    }
    pub fn reset(&mut self, code: quinn::VarInt) -> Result<()> {
        match self {
            Self::Quic(s) => s.reset(code)?,
            Self::Tcp { writer, done } => {
                writer.take();
                done.cancel();
            }
        }
        Ok(())
    }
    pub async fn stopped(&mut self) -> Result<()> {
        match self {
            Self::Quic(s) => {
                s.stopped().await?;
            }
            Self::Tcp { done, .. } => done.cancelled().await,
        }
        Ok(())
    }
}
impl RecvStream {
    pub fn stop(&mut self, code: quinn::VarInt) -> Result<()> {
        match self {
            Self::Quic(s) => s.stop(code)?,
            Self::Tcp(r) => {
                r.take();
            }
        }
        Ok(())
    }
}
impl AsyncRead for RecvStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Quic(s) => Pin::new(s).poll_read(cx, b),
            Self::Tcp(Some(r)) => Pin::new(r).poll_read(cx, b),
            Self::Tcp(None) => Poll::Ready(Ok(())),
        }
    }
}
impl AsyncWrite for SendStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, b: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Quic(s) => AsyncWrite::poll_write(Pin::new(s), cx, b),
            Self::Tcp {
                writer: Some(w), ..
            } => Pin::new(w).poll_write(cx, b),
            _ => Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Quic(s) => Pin::new(s).poll_flush(cx),
            Self::Tcp {
                writer: Some(w), ..
            } => Pin::new(w).poll_flush(cx),
            _ => Poll::Ready(Ok(())),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Quic(s) => Pin::new(s).poll_shutdown(cx),
            Self::Tcp {
                writer: Some(w), ..
            } => Pin::new(w).poll_shutdown(cx),
            _ => Poll::Ready(Ok(())),
        }
    }
}
fn multiplex<T: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    stream: T,
    mode: yamux::Mode,
) -> Connection {
    let mut config = yamux::Config::default();
    config
        .set_max_num_streams(64)
        .set_max_connection_receive_window(Some(16 * 1024 * 1024));
    let mut connection = yamux::Connection::new(stream.compat(), config, mode);
    let (open, mut requests) = mpsc::channel::<Open>(64);
    let (inbound, incoming) = mpsc::channel(64);
    let stop = CancellationToken::new();
    let closed = CancellationToken::new();
    let (cancel, ended) = (stop.clone(), closed.clone());
    tokio::spawn(async move {
        let mut waiting = None;
        let work = futures_util::future::poll_fn(move |cx| {
            if waiting.is_none() {
                match requests.poll_recv(cx) {
                    Poll::Ready(Some(request)) => waiting = Some(request),
                    Poll::Ready(None) => return Poll::Ready(()),
                    Poll::Pending => {}
                }
            }
            if waiting.is_some() {
                match connection.poll_new_outbound(cx) {
                    Poll::Ready(result) => {
                        let _ = waiting.take().unwrap().send(result.map_err(Into::into));
                        cx.waker().wake_by_ref();
                    }
                    Poll::Pending => {}
                }
            }
            match connection.poll_next_inbound(cx) {
                Poll::Ready(Some(Ok(stream))) => {
                    if inbound.try_send(stream).is_err() {
                        return Poll::Ready(());
                    }
                    cx.waker().wake_by_ref();
                }
                Poll::Ready(_) => return Poll::Ready(()),
                Poll::Pending => {}
            }
            Poll::Pending
        });
        tokio::select! {_=work=>{},_=cancel.cancelled()=>{}}
        ended.cancel();
    });
    Connection::Tcp(Arc::new(Tcp {
        open,
        incoming: Mutex::new(incoming),
        stop,
        closed,
    }))
}
pub fn acceptor(data: &Path) -> Result<tokio_rustls::TlsAcceptor> {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    let cert = CertificateDer::from(std::fs::read(data.join("cert.der"))?);
    let key = PrivatePkcs8KeyDer::from(std::fs::read(data.join("key.der"))?);
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key.into())?;
    config.alpn_protocols = vec![b"macrun-yamux/1".to_vec()];
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}
pub async fn accept(
    socket: tokio::net::TcpStream,
    acceptor: tokio_rustls::TlsAcceptor,
) -> Result<Connection> {
    socket.set_nodelay(true)?;
    let stream = tokio::time::timeout(Duration::from_secs(5), acceptor.accept(socket)).await??;
    anyhow::ensure!(
        stream.get_ref().1.alpn_protocol() == Some(b"macrun-yamux/1"),
        "unsupported TCP protocol"
    );
    Ok(multiplex(stream, yamux::Mode::Server))
}
pub async fn connect_tcp(address: &str, certificate: &Path) -> Result<Connection> {
    use rustls::pki_types::{CertificateDer, ServerName};
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(std::fs::read(certificate)?))?;
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"macrun-yamux/1".to_vec()];
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let stream = tokio::time::timeout(Duration::from_secs(5), async {
        let socket = tokio::net::TcpStream::connect(address).await?;
        socket.set_nodelay(true)?;
        let stream = connector
            .connect(ServerName::try_from("macrun")?, socket)
            .await?;
        anyhow::ensure!(
            stream.get_ref().1.alpn_protocol() == Some(b"macrun-yamux/1"),
            "unsupported TCP protocol"
        );
        Ok::<_, anyhow::Error>(stream)
    })
    .await??;
    Ok(multiplex(stream, yamux::Mode::Client))
}
pub async fn connect(
    endpoint: &quinn::Endpoint,
    address: &str,
    certificate: &Path,
) -> Result<Connection> {
    if let Some(tcp) = address.strip_prefix("tcp://") {
        return connect_tcp(tcp, certificate).await;
    }
    let (address, fallback) = address
        .strip_prefix("auto://")
        .map_or((address, false), |a| (a, true));
    let quic = async {
        let socket = tokio::net::lookup_host(address)
            .await?
            .find(|a| a.is_ipv4())
            .ok_or_else(|| anyhow::anyhow!("no IPv4 address"))?;
        Ok::<_, anyhow::Error>(Connection::Quic(endpoint.connect(socket, "macrun")?.await?))
    };
    match tokio::time::timeout(Duration::from_secs(5), quic).await {
        Ok(Ok(c)) => Ok(c),
        result if fallback => {
            let _ = result;
            connect_tcp(address, certificate).await
        }
        Ok(Err(e)) => Err(e),
        Err(e) => Err(e.into()),
    }
}
