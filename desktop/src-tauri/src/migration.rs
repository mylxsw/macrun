//! Filesystem portion of legacy migration. The original worker state is never modified.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

const STATE_ENTRIES: &[&str] = &[
    "tasks",
    "dedup",
    "mirrors",
    "request-key",
    "workspaces.json",
    "safety.json",
    "desktop-policy.json",
];

fn regular(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "迁移状态不能包含符号链接：{}",
        path.display()
    );
    ensure!(
        metadata.is_dir() || metadata.is_file(),
        "迁移状态包含不支持的文件：{}",
        path.display()
    );
    Ok(metadata)
}

fn validate_tree(path: &Path) -> Result<()> {
    if regular(path)?.is_dir() {
        for entry in fs::read_dir(path)? {
            validate_tree(&entry?.path())?;
        }
    } else {
        let _readable = fs::File::open(path)?;
    }
    Ok(())
}

pub fn validate_source(source: &Path) -> Result<()> {
    ensure!(regular(source)?.is_dir(), "旧执行器数据目录不存在");
    let tasks = source.join("tasks");
    let mut needs_key = false;
    if tasks.exists() {
        ensure!(regular(&tasks)?.is_dir(), "旧任务记录不是目录");
        for entry in fs::read_dir(&tasks)? {
            let directory = entry?.path();
            ensure!(regular(&directory)?.is_dir(), "旧任务目录格式不正确");
            let path = directory.join("result.json");
            ensure!(regular(&path)?.is_file(), "旧任务记录缺失");
            let value: Value =
                serde_json::from_slice(&fs::read(&path)?).context("旧任务记录无法解析")?;
            let id = value["task_id"].as_str().context("旧任务缺少编号")?;
            uuid::Uuid::parse_str(id).context("旧任务编号格式不正确")?;
            ensure!(
                directory.file_name().and_then(|n| n.to_str()) == Some(id),
                "旧任务编号与目录不一致"
            );
            ensure!(
                value["ended_at"].as_u64().is_some()
                    && [
                        "succeeded",
                        "failed",
                        "cancelled",
                        "timed_out",
                        "unknown",
                        "denied"
                    ]
                    .contains(&value["status"].as_str().unwrap_or("")),
                "旧执行器有未结束任务，请先停止任务再迁移"
            );
            needs_key |= value.get("request_fingerprint").is_some();
        }
    }
    if source.join("dedup").exists() {
        ensure!(
            regular(&source.join("dedup"))?.is_dir(),
            "旧去重记录不是目录"
        );
        needs_key |= fs::read_dir(source.join("dedup"))?.next().is_some();
    }
    if needs_key {
        ensure!(
            source.join("request-key").is_file(),
            "旧任务去重密钥缺失，无法安全迁移"
        );
    }
    if source.join("request-key").try_exists()? {
        ensure!(
            regular(&source.join("request-key"))?.is_file(),
            "旧任务去重密钥格式不正确"
        );
        ensure!(
            fs::metadata(source.join("request-key"))?.len() == 32,
            "旧任务去重密钥长度不正确"
        );
    }
    for name in ["safety.json", "desktop-policy.json", "workspaces.json"] {
        let path = source.join(name);
        if path.try_exists()? {
            ensure!(regular(&path)?.is_file(), "旧状态文件格式不正确");
            let bytes = fs::read(path)?;
            if name == "safety.json" {
                let _: macrun::safety::Safety =
                    serde_json::from_slice(&bytes).context("旧安全策略无法解析")?;
            } else if name == "desktop-policy.json" {
                let _: macrun::engine::Policy =
                    serde_json::from_slice(&bytes).context("旧桌面策略无法解析")?;
            } else {
                let _: std::collections::BTreeMap<String, Value> =
                    serde_json::from_slice(&bytes).context("旧工作区记录无法解析")?;
            }
        }
    }
    for name in STATE_ENTRIES {
        let path = source.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => validate_tree(&path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let metadata = regular(source)?;
    if metadata.is_dir() {
        fs::create_dir(target)?;
        fs::set_permissions(target, fs::Permissions::from_mode(0o700))?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, target)?;
        fs::set_permissions(target, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub struct MigrationFiles {
    pub stage: PathBuf,
    destination: PathBuf,
    connection: PathBuf,
    worker_installed: bool,
    connection_installed: bool,
    previous_connection: bool,
}

impl MigrationFiles {
    pub fn prepare(data: &Path, source: &Path) -> Result<Self> {
        validate_source(source)?;
        let destination = data.join("worker");
        let source = fs::canonicalize(source)?;
        let target = fs::canonicalize(data)?.join("worker");
        ensure!(
            !source.starts_with(&target) && !target.starts_with(&source),
            "新旧执行器数据目录不能重叠"
        );
        if destination.exists() {
            ensure!(
                regular(&destination)?.is_dir() && fs::read_dir(&destination)?.next().is_none(),
                "新版执行器数据目录非空，已保留现有数据；请先检查再迁移"
            );
        }
        let stage = data.join(format!("migration-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&stage)?;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))?;
        let connection = data.join("connection.json");
        let previous_connection = connection.exists();
        if previous_connection {
            copy_tree(&connection, &stage.join("connection-previous.json"))?;
        }
        Ok(Self {
            stage,
            destination,
            connection,
            worker_installed: false,
            connection_installed: false,
            previous_connection,
        })
    }

    pub fn copy_state(&self, source: &Path) -> Result<()> {
        // Recheck after bootout, so a task received during preparation cannot be lost.
        validate_source(source)?;
        let target = self.stage.join("worker");
        fs::create_dir(&target)?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?;
        for name in STATE_ENTRIES {
            let entry = source.join(name);
            match fs::symlink_metadata(&entry) {
                Ok(_) => copy_tree(&entry, &target.join(name))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub fn install(&mut self) -> Result<()> {
        fs::rename(self.stage.join("worker"), &self.destination)?;
        self.worker_installed = true;
        fs::rename(self.stage.join("connection.json"), &self.connection)?;
        self.connection_installed = true;
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<()> {
        if self.connection_installed {
            fs::rename(&self.connection, self.stage.join("connection.json"))?;
            self.connection_installed = false;
            if self.previous_connection {
                copy_tree(
                    &self.stage.join("connection-previous.json"),
                    &self.connection,
                )?;
            }
        }
        if self.worker_installed {
            fs::rename(&self.destination, self.stage.join("worker"))?;
            self.worker_installed = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn source(root: &Path) -> PathBuf {
        let source = root.join("legacy");
        let id = "12345678-1234-4234-8234-123456789012";
        fs::create_dir_all(source.join("tasks").join(id)).unwrap();
        macrun::wire::atomic_json(
            &source.join("tasks").join(id).join("result.json"),
            &json!({
                "task_id": id, "kind": "exec.start", "status": "succeeded", "started_at": 1,
                "ended_at": 2, "arguments": {"command":"echo migrated", "cwd":"/tmp"}
            }),
        )
        .unwrap();
        fs::create_dir_all(source.join("mirrors/example")).unwrap();
        fs::write(
            source.join("mirrors/example/manifest.json"),
            b"{\"entries\":{},\"skipped\":[]}",
        )
        .unwrap();
        fs::write(source.join("process.lock"), b"do not copy").unwrap();
        source
    }
    #[tokio::test]
    async fn copied_legacy_tasks_keep_dedup_and_mirrors_without_touching_source() {
        let temp = tempfile::tempdir().unwrap();
        let source = source(temp.path());
        let destination = temp.path().join("desktop");
        fs::create_dir(&destination).unwrap();
        let id = "12345678-1234-4234-8234-123456789012";
        let record = source.join("tasks").join(id).join("result.json");
        let original = fs::read(&record).unwrap();
        let mut files = MigrationFiles::prepare(&destination, &source).unwrap();
        files.copy_state(&source).unwrap();
        fs::write(files.stage.join("connection.json"), b"{}").unwrap();
        files.install().unwrap();
        let worker = destination.join("worker");
        assert!(!worker.join("process.lock").exists());
        assert!(worker.join("mirrors/example/manifest.json").is_file());
        let engine = macrun::engine::Engine::open(worker, Default::default()).unwrap();
        let reply = engine
            .handle(
                "exec.start",
                json!({"request_id":id,"command":"echo migrated","cwd":"/tmp"}),
            )
            .await
            .unwrap();
        assert_eq!(reply["duplicate"], true);
        assert_eq!(engine.active_count().await, 0);
        assert_eq!(fs::read(record).unwrap(), original);
        assert!(!source.join("request-key").exists());
    }
    #[test]
    fn install_failure_restores_worker_and_keeps_old_connection() {
        let temp = tempfile::tempdir().unwrap();
        let source = source(temp.path());
        let destination = temp.path().join("desktop");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("connection.json"), b"previous").unwrap();
        let mut files = MigrationFiles::prepare(&destination, &source).unwrap();
        files.copy_state(&source).unwrap();
        assert!(files.install().is_err()); // Candidate config deliberately absent.
        files.rollback().unwrap();
        assert!(!destination.join("worker").exists());
        assert_eq!(
            fs::read(destination.join("connection.json")).unwrap(),
            b"previous"
        );
        assert!(files.stage.join("worker/tasks").exists());
    }
    #[test]
    fn full_rollback_restores_previous_connection_and_preserves_staged_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = source(temp.path());
        let destination = temp.path().join("desktop");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("connection.json"), b"previous").unwrap();
        let mut files = MigrationFiles::prepare(&destination, &source).unwrap();
        files.copy_state(&source).unwrap();
        fs::write(files.stage.join("connection.json"), b"new").unwrap();
        files.install().unwrap();
        files.rollback().unwrap();
        assert_eq!(
            fs::read(destination.join("connection.json")).unwrap(),
            b"previous"
        );
        assert_eq!(
            fs::read(files.stage.join("connection.json")).unwrap(),
            b"new"
        );
        assert!(files.stage.join("worker/tasks").is_dir());
        assert!(!destination.join("worker").exists());
    }
    #[test]
    fn rejects_nonempty_target_broken_records_and_missing_dedup_key() {
        let temp = tempfile::tempdir().unwrap();
        let source = source(temp.path());
        let destination = temp.path().join("desktop");
        fs::create_dir_all(destination.join("worker/tasks")).unwrap();
        assert!(MigrationFiles::prepare(&destination, &source).is_err());
        let record = source.join("tasks/12345678-1234-4234-8234-123456789012/result.json");
        let original = fs::read(&record).unwrap();
        fs::write(&record, b"not json").unwrap();
        assert!(validate_source(&source).is_err());
        fs::write(&record, &original).unwrap();
        let mut value: Value = serde_json::from_slice(&original).unwrap();
        value["request_fingerprint"] = json!("existing-hash");
        macrun::wire::atomic_json(&record, &value).unwrap();
        assert!(validate_source(&source).is_err());
        fs::write(source.join("request-key"), [1u8; 31]).unwrap();
        assert!(validate_source(&source).is_err());
    }
    #[test]
    fn preserves_existing_key_policy_and_refuses_active_or_symlink_state() {
        let temp = tempfile::tempdir().unwrap();
        let source = source(temp.path());
        fs::write(source.join("request-key"), [7u8; 32]).unwrap();
        fs::write(source.join("safety.json"), b"{\"approval\":\"all\"}").unwrap();
        let destination = temp.path().join("desktop");
        fs::create_dir(&destination).unwrap();
        let files = MigrationFiles::prepare(&destination, &source).unwrap();
        files.copy_state(&source).unwrap();
        assert_eq!(
            fs::read(files.stage.join("worker/request-key")).unwrap(),
            [7u8; 32]
        );
        assert_eq!(
            fs::read(files.stage.join("worker/safety.json")).unwrap(),
            fs::read(source.join("safety.json")).unwrap()
        );
        let record = source.join("tasks/12345678-1234-4234-8234-123456789012/result.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
        value["status"] = json!("running");
        value["ended_at"] = Value::Null;
        macrun::wire::atomic_json(&record, &value).unwrap();
        assert!(validate_source(&source).is_err());
        value["status"] = json!("succeeded");
        value["ended_at"] = json!(2);
        macrun::wire::atomic_json(&record, &value).unwrap();
        std::os::unix::fs::symlink(&record, source.join("mirrors/unsafe")).unwrap();
        assert!(MigrationFiles::prepare(&destination, &source).is_err());
    }
}
