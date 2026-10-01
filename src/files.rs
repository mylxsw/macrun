use crate::config::expand;
use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
pub const CHUNK: usize = 512 * 1024;
pub fn string<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str()
        .ok_or_else(|| anyhow::anyhow!("{k} must be a string"))
}
pub async fn handle(kind: &str, a: &Value) -> Result<Value> {
    let path = expand(string(a, "path")?);
    match kind {
        "file.list" => {
            let mut entries = vec![];
            let mut rd = tokio::fs::read_dir(&path).await?;
            let offset = a["offset"].as_u64().unwrap_or(0) as usize;
            while let Some(e) = rd.next_entry().await? {
                let m = e.metadata().await?;
                entries.push(json!({"name":e.file_name().to_string_lossy(),"directory":m.is_dir(),"size":m.len()}));
            }
            entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            let total = entries.len();
            let page: Vec<_> = entries.into_iter().skip(offset).take(1000).collect();
            Ok(json!({"entries":page,"total":total,"next_offset":offset+page.len()}))
        }
        "file.read" | "file.image" => {
            let mut f = tokio::fs::File::open(&path).await?;
            let m = f.metadata().await?;
            if !m.is_file() {
                bail!("path is not a regular file");
            }
            let offset = if kind == "file.image" {
                0
            } else {
                a["offset"].as_u64().unwrap_or(0)
            };
            let max = if kind == "file.image" {
                8 * 1024 * 1024
            } else {
                CHUNK
            };
            if kind == "file.image" && m.len() > max as u64 {
                bail!("image exceeds 8 MiB; download or resize first");
            }
            let len = if kind == "file.image" {
                max
            } else {
                a["length"].as_u64().unwrap_or(max as u64).min(max as u64) as usize
            };
            f.seek(std::io::SeekFrom::Start(offset)).await?;
            let mut b = vec![0; len];
            let mut n = 0;
            while n < len {
                let k = f.read(&mut b[n..]).await?;
                if k == 0 {
                    break;
                }
                n += k;
            }
            b.truncate(n);
            let mut v = json!({"path":path,"size":m.len(),"version":format!("{}:{:?}",m.len(),m.modified()?),"offset":offset,"next_offset":offset+n as u64,"eof":offset+n as u64>=m.len(),"data":STANDARD.encode(&b)});
            if kind == "file.image" {
                let mime = if b.starts_with(b"\x89PNG\r\n\x1a\n") {
                    "image/png"
                } else if b.starts_with(&[255, 216, 255]) {
                    "image/jpeg"
                } else {
                    bail!("image must be PNG or JPEG");
                };
                v["content"] = json!([{"type":"image","mimeType":mime,"data":STANDARD.encode(&b)}]);
                v.as_object_mut().unwrap().remove("data");
            } else if a["text"] == true {
                v["text"] = json!(String::from_utf8_lossy(&b));
            }
            Ok(v)
        }
        "file.move" => {
            let dest = expand(string(a, "destination")?);
            tokio::fs::rename(&path, &dest).await?;
            Ok(json!({"path":dest}))
        }
        "file.write" => {
            let b = if let Some(text) = a["text"].as_str() {
                text.as_bytes().to_vec()
            } else {
                STANDARD.decode(string(a, "data")?)?
            };
            if b.len() > CHUNK {
                bail!("write chunk exceeds 512 KiB");
            }
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let offset = a["offset"].as_u64().unwrap_or(0);
            let truncate = a["truncate"].as_bool().unwrap_or(offset == 0);
            let mut f = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(truncate)
                .open(&path)
                .await?;
            f.seek(std::io::SeekFrom::Start(offset)).await?;
            f.write_all(&b).await?;
            f.sync_all().await?;
            Ok(json!({"path":path,"written":b.len(),"next_offset":offset+b.len() as u64}))
        }
        _ => bail!("unknown file operation"),
    }
}
