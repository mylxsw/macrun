//! One-use invitations. The URI transports the public certificate, never a durable token.
use crate::{
    model::{Control, PROTOCOL, id, now},
    wire,
};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};
#[derive(Serialize, Deserialize)]
pub struct Invite {
    pub server: String,
    pub certificate: String,
    pub fingerprint: String,
    pub code: String,
    pub expires_at: u64,
    pub protocol: u32,
}
pub fn create(data: &Path, server: String) -> Result<String> {
    ensure!(!server.trim().is_empty(), "server address required");
    let cert = std::fs::read(data.join("cert.der"))?;
    let code = format!("{}{}", id(), id());
    let expires_at = now() + 600_000;
    let dir = data.join("invitations");
    let _lock = wire::lock(&dir)?;
    wire::atomic_json(
        &dir.join(format!("{}.json", blake3::hash(code.as_bytes()))),
        &serde_json::json!({"expires_at":expires_at}),
    )?;
    let invite = Invite {
        server,
        certificate: URL_SAFE_NO_PAD.encode(&cert),
        fingerprint: blake3::hash(&cert).to_hex().to_string(),
        code,
        expires_at,
        protocol: PROTOCOL,
    };
    Ok(format!(
        "macrun://pair/{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&invite)?)
    ))
}
pub fn parse(uri: &str) -> Result<(Invite, Vec<u8>)> {
    ensure!(uri.len() < 32768, "invitation too large");
    let encoded = uri
        .trim()
        .strip_prefix("macrun://pair/")
        .ok_or_else(|| anyhow::anyhow!("invalid pairing URI"))?;
    let invite: Invite = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded)?)?;
    ensure!(invite.protocol == PROTOCOL, "protocol mismatch");
    ensure!(invite.expires_at > now(), "invitation expired");
    let cert = URL_SAFE_NO_PAD.decode(&invite.certificate)?;
    ensure!(
        blake3::hash(&cert).to_hex().as_str() == invite.fingerprint,
        "certificate fingerprint mismatch"
    );
    Ok((invite, cert))
}
pub fn redeem(data: &Path, code: &str) -> Result<String> {
    ensure!(code.len() <= 256, "invalid invitation");
    let dir = data.join("invitations");
    let _lock = wire::lock(&dir)?;
    let path = dir.join(format!("{}.json", blake3::hash(code.as_bytes())));
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&path).map_err(|_| anyhow::anyhow!("invitation invalid or already used"))?,
    )?;
    ensure!(
        record["expires_at"].as_u64().unwrap_or(0) > now(),
        "invitation expired"
    );
    // Consume before issuing credentials: interrupted exchanges require a fresh invite.
    std::fs::remove_file(path)?;
    let token = format!("{}{}", id(), id());
    wire::atomic_json(
        &data
            .join("devices")
            .join(format!("{}.json", blake3::hash(token.as_bytes()))),
        &serde_json::json!({"created_at":now()}),
    )?;
    Ok(token)
}
pub fn authorized(data: &Path, token: &str) -> bool {
    token.len() <= 256
        && data
            .join("devices")
            .join(format!("{}.json", blake3::hash(token.as_bytes())))
            .is_file()
}
pub async fn exchange(uri: &str, certificate_path: &Path) -> Result<(Invite, String)> {
    let (invite, cert) = parse(uri)?;
    wire::private_write(certificate_path, &cert)?;
    let address = tokio::net::lookup_host(&invite.server)
        .await?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| anyhow::anyhow!("no IPv4 server address"))?;
    let endpoint = wire::client(certificate_path)?;
    let token = tokio::time::timeout(Duration::from_secs(12), async {
        let conn = endpoint.connect(address, "macrun")?.await?;
        let (mut out, mut input) = conn.open_bi().await?;
        wire::send(
            &mut out,
            &Control::Pair {
                protocol: PROTOCOL,
                code: invite.code.clone(),
            },
        )
        .await?;
        match wire::recv(&mut input).await? {
            Control::Paired { token, protocol } if protocol == PROTOCOL => Ok(token),
            Control::Reject { error } => Err(error.into()),
            _ => anyhow::bail!("unexpected pairing response"),
        }
    })
    .await??;
    endpoint.close(0u32.into(), b"paired");
    Ok((invite, token))
}
