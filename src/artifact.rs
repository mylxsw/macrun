//! Images live beside task metadata; old clients can still request inline content.
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn path(task_dir: &Path, id: &str) -> Result<PathBuf> {
    ensure!(
        id.len() == 64
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid artifact id"
    );
    Ok(task_dir.join("artifacts").join(id))
}
pub fn store(task_dir: &Path, value: &mut Value) -> Result<Vec<Value>> {
    let mut refs = Vec::new();
    visit(value, &mut |object| {
        if object.get("type").and_then(Value::as_str) != Some("image") {
            return Ok(());
        }
        let Some(data) = object.get("data").and_then(Value::as_str) else {
            return Ok(());
        };
        let bytes = STANDARD.decode(data)?;
        let id = blake3::hash(&bytes).to_hex().to_string();
        let file = path(task_dir, &id)?;
        std::fs::create_dir_all(file.parent().unwrap())?;
        crate::wire::private_write(&file, &bytes)?;
        let reference = json!({"id":id,"bytes":bytes.len(),"mimeType":object.get("mimeType"),"created_at":crate::model::now()});
        object.remove("data");
        object.insert("artifact".into(), reference.clone());
        if !refs.iter().any(|r: &Value| r["id"] == reference["id"]) {
            refs.push(reference);
        }
        Ok(())
    })?;
    Ok(refs)
}
pub fn hydrate(task_dir: &Path, value: &mut Value) -> Result<()> {
    visit(value, &mut |object| {
        if object.get("type").and_then(Value::as_str) == Some("image")
            && let Some(reference) = object.get("artifact")
        {
            let id = reference["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid artifact"))?;
            let bytes = std::fs::read(path(task_dir, id)?)?;
            ensure!(
                blake3::hash(&bytes).to_hex().as_str() == id,
                "artifact checksum mismatch"
            );
            object.insert("data".into(), json!(STANDARD.encode(bytes)));
            object.remove("artifact");
        }
        Ok(())
    })
}
fn visit(
    value: &mut Value,
    f: &mut impl FnMut(&mut serde_json::Map<String, Value>) -> Result<()>,
) -> Result<()> {
    match value {
        Value::Object(object) => {
            f(object)?;
            for child in object.values_mut() {
                visit(child, f)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                visit(child, f)?;
            }
        }
        _ => {}
    }
    Ok(())
}
