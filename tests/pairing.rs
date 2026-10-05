use macrun::{pairing, wire};
#[test]
fn invitations_pin_certificates_and_are_single_use() {
    let d = tempfile::tempdir().unwrap();
    wire::init_tls(d.path()).unwrap();
    let uri = pairing::create(d.path(), "127.0.0.1:7443".into()).unwrap();
    let (invite, cert) = pairing::parse(&uri).unwrap();
    assert_eq!(cert, std::fs::read(d.path().join("cert.der")).unwrap());
    let token = pairing::redeem(d.path(), &invite.code).unwrap();
    assert!(pairing::authorized(d.path(), &token));
    assert!(!pairing::authorized(d.path(), "unknown"));
    assert!(pairing::redeem(d.path(), &invite.code).is_err());
    assert!(!uri.contains(&token));
}
#[test]
fn expired_or_tampered_invitations_fail() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let d = tempfile::tempdir().unwrap();
    wire::init_tls(d.path()).unwrap();
    let uri = pairing::create(d.path(), "127.0.0.1:7443".into()).unwrap();
    let (mut invite, _) = pairing::parse(&uri).unwrap();
    invite.expires_at = 0;
    let encode = |i: &pairing::Invite| {
        format!(
            "macrun://pair/{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(i).unwrap())
        )
    };
    assert!(pairing::parse(&encode(&invite)).is_err());
    invite.expires_at = u64::MAX;
    invite.fingerprint = "wrong".into();
    assert!(pairing::parse(&encode(&invite)).is_err());
    assert!(pairing::redeem(d.path(), "../token").is_err());
}
#[tokio::test]
async fn real_quic_exchange_redeems_only_once() {
    let d = tempfile::tempdir().unwrap();
    wire::init_tls(d.path()).unwrap();
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    drop(socket);
    let data = d.path().to_path_buf();
    let server = tokio::spawn(macrun::server::serve(addr, d.path().join("cli.sock"), data));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let uri = pairing::create(d.path(), addr.to_string()).unwrap();
    let (_, token) = pairing::exchange(&uri, &d.path().join("client.der"))
        .await
        .unwrap();
    assert!(pairing::authorized(d.path(), &token));
    assert!(
        pairing::exchange(&uri, &d.path().join("client.der"))
            .await
            .is_err()
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn real_tcp_exchange_keeps_authentication_and_single_use() {
    let d = tempfile::tempdir().unwrap();
    wire::init_tls(d.path()).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let server = tokio::spawn(macrun::server::serve_with_tcp(
        "127.0.0.1:0".parse().unwrap(),
        d.path().join("cli.sock"),
        d.path().into(),
        Some(addr),
    ));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let uri = pairing::create(d.path(), format!("tcp://{addr}")).unwrap();
    let (_, token) = pairing::exchange(&uri, &d.path().join("client.der"))
        .await
        .unwrap();
    assert!(pairing::authorized(d.path(), &token));
    assert!(
        pairing::exchange(&uri, &d.path().join("client.der"))
            .await
            .is_err()
    );
    let conn = macrun::transport::connect_tcp(&addr.to_string(), &d.path().join("cert.der"))
        .await
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    wire::send(
        &mut send,
        &serde_json::json!({"type":"hello","token":"wrong","protocol":macrun::model::PROTOCOL,"instance":"test","status":{}}),
    )
    .await
    .unwrap();
    // Reject followed by immediate connection close may arrive as EOF. Neither
    // transport may admit the unauthorized Hello or return Welcome.
    if let Ok(reply) = wire::recv::<_, serde_json::Value>(&mut recv).await {
        assert_eq!(reply["type"], "reject");
    }
    server.abort();
    let _ = server.await;
}
#[test]
fn expired_server_record_and_concurrent_redemption_are_rejected() {
    let d = tempfile::tempdir().unwrap();
    wire::init_tls(d.path()).unwrap();
    let uri = pairing::create(d.path(), "127.0.0.1:7443".into()).unwrap();
    let (invite, _) = pairing::parse(&uri).unwrap();
    wire::atomic_json(
        &d.path()
            .join("invitations")
            .join(format!("{}.json", blake3::hash(invite.code.as_bytes()))),
        &serde_json::json!({"expires_at":0}),
    )
    .unwrap();
    assert!(pairing::redeem(d.path(), &invite.code).is_err());
    let uri = pairing::create(d.path(), "127.0.0.1:7443".into()).unwrap();
    let (invite, _) = pairing::parse(&uri).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = d.path().to_path_buf();
            let code = invite.code.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                pairing::redeem(&path, &code).is_ok()
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter(|ok| *ok)
            .count(),
        1
    );
}
