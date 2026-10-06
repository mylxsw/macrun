//! Negotiated, bounded delta/pack path. Installation still verifies the whole file.
use crate::{
    model::Event,
    sync::{self, Entry, Manifest, Receiver},
    wire,
    workspace::Lease,
};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};

pub const BLOCK: usize = 1024 * 1024;
pub const MAX_BLOCKS: usize = 8192;
const SMALL: u64 = 64 * 1024;
const PACK: usize = 4 * 1024 * 1024;
const MAX_FILES: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Offer {
    pub path: String,
    pub blocks: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Packet {
    File {
        path: String,
        size: u64,
    },
    Delta {
        path: String,
        size: u64,
        changed: Vec<usize>,
    },
    Pack {
        paths: Vec<String>,
        size: usize,
        raw_size: usize,
    },
}
#[derive(Default)]
pub struct Stats {
    pub files: usize,
    pub bytes: u64,
    pub reused_bytes: u64,
    pub packs: usize,
}

pub fn hashes(path: &Path) -> Result<Vec<String>> {
    let mut file = std::fs::File::open(path)?;
    ensure!(file.metadata()?.is_file(), "delta base is not a file");
    let mut out = vec![];
    let mut buffer = vec![0; BLOCK];
    loop {
        let mut n = 0;
        while n < BLOCK {
            let k = file.read(&mut buffer[n..])?;
            if k == 0 {
                break;
            }
            n += k;
        }
        if n == 0 {
            break;
        }
        ensure!(out.len() < MAX_BLOCKS, "too many delta blocks");
        out.push(blake3::hash(&buffer[..n]).to_hex().to_string());
    }
    Ok(out)
}
pub fn offers(receiver: &Receiver) -> Result<Vec<Offer>> {
    let mut budget = 32 * 1024 * 1024;
    receiver
        .needed
        .iter()
        .map(|p| {
            let path = receiver.root.join(sync::relative(p)?);
            let blocks = match std::fs::symlink_metadata(&path) {
                Ok(m)
                    if m.is_file()
                        && m.len() >= 4 * BLOCK as u64
                        && m.len() <= (BLOCK * MAX_BLOCKS) as u64
                        && m.len().div_ceil(BLOCK as u64) as usize * 64 <= budget =>
                {
                    let h = hashes(&path)?;
                    budget -= h.len() * 64;
                    h
                }
                _ => vec![],
            };
            Ok(Offer {
                path: p.clone(),
                blocks,
            })
        })
        .collect()
}
fn size(manifest: &Manifest, path: &str) -> Result<u64> {
    sync::relative(path)?;
    match manifest.entries.get(path) {
        Some(Entry::File { size, .. }) => Ok(*size),
        _ => bail!("unexpected sync file"),
    }
}

pub async fn send<W: AsyncWrite + Unpin>(
    root: &Path,
    manifest: &Manifest,
    offers: &[Offer],
    out: &mut W,
) -> Result<()> {
    let _phase = crate::metrics::phase("send_payload");
    let mut seen = BTreeSet::new();
    let mut at = 0;
    while at < offers.len() {
        let offer = &offers[at];
        ensure!(seen.insert(offer.path.clone()), "duplicate needed file");
        let length = size(manifest, &offer.path)?;
        if length <= SMALL {
            let mut paths = vec![offer.path.clone()];
            let mut raw_size = length as usize;
            at += 1;
            while at < offers.len() && paths.len() < MAX_FILES {
                let n = size(manifest, &offers[at].path)?;
                if n > SMALL || raw_size + n as usize > PACK {
                    break;
                }
                ensure!(
                    seen.insert(offers[at].path.clone()),
                    "duplicate needed file"
                );
                paths.push(offers[at].path.clone());
                raw_size += n as usize;
                at += 1;
            }
            let batch = paths.clone();
            let root = root.to_owned();
            let sizes: Vec<_> = batch
                .iter()
                .map(|p| size(manifest, p))
                .collect::<Result<_>>()?;
            let payload = wire::blocking(move || -> Result<Vec<u8>> {
                let _phase = crate::metrics::phase("pack_encode");
                let mut encoder =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
                for (path, expected) in batch.into_iter().zip(sizes) {
                    let mut bytes = Vec::new();
                    std::fs::File::open(root.join(&path))?
                        .take(SMALL + 1)
                        .read_to_end(&mut bytes)?;
                    ensure!(
                        bytes.len() as u64 == expected,
                        "source_changed: pack length"
                    );
                    encoder.write_all(&bytes)?;
                }
                Ok(encoder.finish()?)
            })
            .await??;
            ensure!(payload.len() <= PACK + 65536, "pack exceeds limit");
            wire::send(
                out,
                &Packet::Pack {
                    paths,
                    size: payload.len(),
                    raw_size,
                },
            )
            .await?;
            crate::metrics::add("bytes", "pack_raw", raw_size as u64);
            crate::metrics::add("bytes", "pack_compressed", payload.len() as u64);
            crate::metrics::add("files", "packs", 1);
            crate::metrics::Metered::new(&mut *out, "payload")
                .write_all(&payload)
                .await?;
            continue;
        }
        let path = root.join(&offer.path);
        let blocks = if !offer.blocks.is_empty() && length <= (BLOCK * MAX_BLOCKS) as u64 {
            let p = path.clone();
            wire::blocking(move || hashes(&p)).await??
        } else {
            vec![]
        };
        let changed: Vec<_> = blocks
            .iter()
            .enumerate()
            .filter(|(i, h)| offer.blocks.get(*i) != Some(h))
            .map(|(i, _)| i)
            .collect();
        let mut file = tokio::fs::File::open(&path).await?;
        if !blocks.is_empty() && changed.len() * 4 < blocks.len() * 3 {
            wire::send(
                out,
                &Packet::Delta {
                    path: offer.path.clone(),
                    size: length,
                    changed: changed.clone(),
                },
            )
            .await?;
            for i in changed {
                let offset = (i * BLOCK) as u64;
                let n = (length - offset).min(BLOCK as u64);
                file.seek(std::io::SeekFrom::Start(offset)).await?;
                ensure!(
                    tokio::io::copy(
                        &mut (&mut file).take(n),
                        &mut crate::metrics::Metered::new(&mut *out, "payload")
                    )
                    .await?
                        == n,
                    "source_changed: delta length"
                );
            }
        } else {
            wire::send(
                out,
                &Packet::File {
                    path: offer.path.clone(),
                    size: length,
                },
            )
            .await?;
            ensure!(
                tokio::io::copy(
                    &mut file.take(length),
                    &mut crate::metrics::Metered::new(&mut *out, "payload")
                )
                .await?
                    == length,
                "source_changed: file length"
            );
        }
        at += 1;
    }
    Ok(())
}

async fn install(
    receiver: Arc<Receiver>,
    lease: Arc<Lease>,
    path: String,
    temp: PathBuf,
) -> Result<()> {
    wire::blocking(move || {
        let _guard = lease;
        receiver.install(&path, &temp)
    })
    .await??;
    Ok(())
}
pub async fn receive<R: AsyncRead + Unpin>(
    input: &mut R,
    receiver: Arc<Receiver>,
    offers: &[Offer],
    lease: Arc<Lease>,
) -> Result<Stats> {
    let incoming = receiver.root.join(".macrun/incoming");
    tokio::fs::create_dir_all(&incoming).await?;
    let mut remaining: BTreeSet<_> = receiver.needed.iter().cloned().collect();
    let mut stats = Stats::default();
    while !remaining.is_empty() {
        let packet: Packet = wire::recv(input).await?;
        match packet {
            Packet::Pack {
                paths,
                size: bytes,
                raw_size,
            } => {
                ensure!(
                    !paths.is_empty()
                        && paths.len() <= MAX_FILES
                        && bytes <= PACK + 65536
                        && raw_size <= PACK,
                    "invalid pack limits"
                );
                let mut total = 0;
                for path in &paths {
                    ensure!(remaining.remove(path), "unexpected pack path");
                    let n = size(&receiver.manifest, path)?;
                    ensure!(n <= SMALL, "oversized pack file");
                    total += n as usize;
                }
                ensure!(total == raw_size, "pack size mismatch");
                let mut payload = vec![0; bytes];
                crate::metrics::Metered::new(&mut *input, "payload")
                    .read_exact(&mut payload)
                    .await?;
                crate::metrics::add("bytes", "pack_raw", raw_size as u64);
                crate::metrics::add("bytes", "pack_compressed", bytes as u64);
                crate::metrics::add("files", "packs", 1);
                let r = receiver.clone();
                let guard = lease.clone();
                let dir = incoming.clone();
                let count = paths.len();
                wire::blocking(move || -> Result<()> {
                    let _guard = guard;
                    let mut decoder = flate2::read::ZlibDecoder::new(payload.as_slice());
                    let decoding = crate::metrics::phase("pack_decode");
                    let mut raw = Vec::new();
                    (&mut decoder)
                        .take((PACK + 1) as u64)
                        .read_to_end(&mut raw)?;
                    ensure!(
                        raw.len() == raw_size && decoder.total_in() == payload.len() as u64,
                        "invalid compressed pack"
                    );
                    drop(decoding);
                    let mut offset = 0;
                    for path in paths {
                        let n = size(&r.manifest, &path)? as usize;
                        let temp = dir.join(crate::model::id());
                        wire::private_write(&temp, &raw[offset..offset + n])?;
                        r.install(&path, &temp)?;
                        offset += n;
                    }
                    Ok(())
                })
                .await??;
                stats.files += count;
                stats.bytes += bytes as u64;
                stats.packs += 1;
            }
            Packet::File { path, size: length } => {
                ensure!(
                    remaining.remove(&path) && size(&receiver.manifest, &path)? == length,
                    "unexpected file"
                );
                let temp = incoming.join(crate::model::id());
                let mut file = tokio::fs::File::create(&temp).await?;
                ensure!(
                    tokio::io::copy(
                        &mut crate::metrics::Metered::new(&mut *input, "payload").take(length),
                        &mut file
                    )
                    .await?
                        == length,
                    "truncated file"
                );
                file.sync_all().await?;
                drop(file);
                install(receiver.clone(), lease.clone(), path, temp).await?;
                stats.bytes += length;
                stats.files += 1;
            }
            Packet::Delta {
                path,
                size: length,
                changed,
            } => {
                ensure!(
                    remaining.remove(&path)
                        && size(&receiver.manifest, &path)? == length
                        && length <= (MAX_BLOCKS * BLOCK) as u64,
                    "invalid delta"
                );
                let offer = offers
                    .iter()
                    .find(|o| o.path == path)
                    .ok_or_else(|| anyhow::anyhow!("missing delta base"))?;
                ensure!(!offer.blocks.is_empty(), "unoffered delta");
                let count = length.div_ceil(BLOCK as u64) as usize;
                let updates: BTreeSet<_> = changed.iter().copied().collect();
                ensure!(
                    updates.len() == changed.len() && updates.iter().all(|i| *i < count),
                    "invalid delta indices"
                );
                let mut old = tokio::fs::File::open(receiver.root.join(&path)).await?;
                let temp = incoming.join(crate::model::id());
                let mut file = tokio::fs::File::create(&temp).await?;
                let mut buffer = vec![0; BLOCK];
                for i in 0..count {
                    let n = (length - (i * BLOCK) as u64).min(BLOCK as u64) as usize;
                    if updates.contains(&i) {
                        crate::metrics::Metered::new(&mut *input, "payload")
                            .read_exact(&mut buffer[..n])
                            .await?;
                        stats.bytes += n as u64;
                    } else {
                        old.seek(std::io::SeekFrom::Start((i * BLOCK) as u64))
                            .await?;
                        old.read_exact(&mut buffer[..n]).await?;
                        ensure!(offer.blocks.get(i).is_some_and(|h|*h==blake3::hash(&buffer[..n]).to_hex().as_str()),"delta base changed");
                        stats.reused_bytes += n as u64;
                        crate::metrics::add("bytes", "reused", n as u64);
                    }
                    file.write_all(&buffer[..n]).await?;
                }
                file.sync_all().await?;
                drop(file);
                install(receiver.clone(), lease.clone(), path, temp).await?;
                stats.files += 1;
            }
        }
    }
    Ok(stats)
}

pub async fn send_offers(tx: &tokio::sync::mpsc::Sender<Event>, offers: &[Offer]) -> Result<()> {
    let mut page = Vec::new();
    let mut bytes = 0;
    for offer in offers {
        let length = serde_json::to_vec(offer)?.len();
        ensure!(length < wire::MAX_FRAME / 2, "oversized delta offer");
        if !page.is_empty() && (bytes + length > 1024 * 1024 || page.len() == 256) {
            tx.send(Event::NeedObjects {
                objects: std::mem::take(&mut page),
                last: false,
            })
            .await?;
            bytes = 0;
        }
        bytes += length;
        page.push(offer.clone());
    }
    tx.send(Event::NeedObjects {
        objects: page,
        last: true,
    })
    .await?;
    Ok(())
}
