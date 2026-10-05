use super::*;

pub fn set_application_icon() {
    // Embed the same multi-resolution image shipped in Contents/Resources so
    // startup does not depend on LaunchServices retaining the newest icon.
    let icon = include_bytes!("../icons/icon.icns");
    if !unsafe { macrun_set_application_icon(icon.as_ptr(), icon.len()) } {
        eprintln!("Unable to load the Macrun Dock icon");
    }
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn CGPreflightScreenCaptureAccess() -> bool;
}
#[tauri::command]
pub fn permissions() -> Value {
    json!({"accessibility":unsafe {AXIsProcessTrusted()},"screen_recording":unsafe{CGPreflightScreenCaptureAccess()},"graphical_session":unsafe{macrun_graphical_session()},"keep_awake":unsafe{macrun_awake_active()},"scope":"desktop_app","note":"后端有独立权限；请通过后端实拍验证。"})
}
fn permission_url(kind: &str) -> std::result::Result<String, String> {
    let pane = match kind {
        "accessibility" => "Privacy_Accessibility",
        "screen" => "Privacy_ScreenCapture",
        "input" => "Privacy_ListenEvent",
        _ => return Err("未知权限".into()),
    };
    Ok(format!(
        "x-apple.systempreferences:com.apple.preference.security?{pane}"
    ))
}
#[tauri::command]
pub async fn open_permission(kind: String) -> std::result::Result<(), String> {
    let status = tokio::process::Command::new("open")
        .arg(permission_url(&kind)?)
        .status()
        .await
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("无法打开系统权限设置，请在系统设置中手动打开“隐私与安全性”".into());
    }
    Ok(())
}
fn task_workspace(task: &Value, task_id: &str, root: &str) -> std::result::Result<PathBuf, String> {
    if task["task_id"] != task_id {
        return Err("任务记录不匹配".into());
    }
    let key = match task["kind"].as_str() {
        Some("exec.start") => "cwd",
        Some("sync") => "remote_root",
        _ => return Err("该任务没有可打开的工作目录".into()),
    };
    let stored = task["arguments"][key]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("任务工作目录不可用")?;
    if stored != root {
        return Err("目录与任务记录不匹配".into());
    }
    Ok(macrun::config::expand(stored))
}
#[tauri::command]
pub async fn open_workspace(
    root: String,
    terminal: bool,
    task_id: Option<String>,
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<(), String> {
    let p = if let Some(task_id) = task_id {
        let task = local::request(&rt.socket, "task_detail", json!({"task_id":task_id}))
            .await
            .map_err(|e| e.to_string())?;
        task_workspace(&task, &task_id, &root)?
    } else {
        let snapshot = local::request(&rt.socket, "snapshot", json!({}))
            .await
            .map_err(|e| e.to_string())?;
        if !snapshot["workspaces"]
            .as_array()
            .is_some_and(|w| w.iter().any(|w| w["root"] == root))
        {
            return Err("未知工作区".into());
        }
        macrun::config::expand(&root)
    };
    if !p.is_dir() {
        return Err("工作区目录不存在".into());
    }
    let mut cmd = tokio::process::Command::new("open");
    if terminal {
        cmd.args(["-a", "Terminal"]);
    }
    if !cmd
        .arg(p)
        .status()
        .await
        .map_err(|e| e.to_string())?
        .success()
    {
        return Err("无法打开工作目录，请检查目录访问权限".into());
    }
    Ok(())
}
#[tauri::command]
pub fn backend_config(rt: tauri::State<Runtime>) -> std::result::Result<String, String> {
    let cfg = rt.settings.lock().unwrap().clone();
    if cfg.backend_config.is_empty() {
        return Ok("# 添加本机 MCP 后端\n# [mcp.example]\n# command = \"/absolute/path/backend\"\n# args = []\n".into());
    }
    std::fs::read_to_string(cfg.backend_config).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn save_backends(text: String, rt: tauri::State<Runtime>) -> std::result::Result<(), String> {
    let result: Result<()> = (|| {
        let mut child = rt.child.lock().unwrap();
        anyhow::ensure!(
            !rt.starting.load(Ordering::SeqCst),
            "执行器正在启动，请等待后再修改配置"
        );
        anyhow::ensure!(
            !rt.migrating.load(Ordering::SeqCst),
            "正在迁移旧执行器，请稍后修改配置"
        );
        anyhow::ensure!(
            !child
                .as_mut()
                .is_some_and(|c| c.try_wait().ok().flatten().is_none()),
            "请先断开连接，再修改后端配置"
        );
        let config: macrun::config::WorkerConfig = toml::from_str(&text)?;
        for backend in config.mcp.values() {
            anyhow::ensure!(!backend.command.trim().is_empty(), "后端命令不能为空");
        }
        let path = rt.data.join("worker.toml");
        if path.exists() {
            std::fs::copy(
                &path,
                rt.data
                    .join(format!("worker-{}.toml.bak", macrun::model::now())),
            )?;
        }
        wire::private_write(&path, text.as_bytes())?;
        let mut settings = rt.settings.lock().unwrap().clone();
        settings.backend_config = path.to_string_lossy().into();
        wire::atomic_json(&rt.data.join("connection.json"), &settings)?;
        *rt.settings.lock().unwrap() = settings;
        Ok(())
    })();
    result.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn diagnostics(rt: tauri::State<'_, Runtime>) -> std::result::Result<String, String> {
    let state = local::request(&rt.socket, "snapshot", json!({}))
        .await
        .unwrap_or(Value::Null);
    // Explicit allowlist: no paths, hostnames, commands, backend arguments, logs or credentials.
    let value = json!({"desktop_version":env!("CARGO_PKG_VERSION"),"worker_version":state["version"],"protocol":state["protocol"],"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"connection_state":state["connection"]["state"],"active_count":state["active_count"],"permissions":permissions(),"generated_at":macrun::model::now()});
    let path = rt
        .data
        .join(format!("diagnostics-{}.json", macrun::model::now()));
    wire::atomic_json(&path, &value).map_err(|e| e.to_string())?;
    Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into())
}
#[tauri::command]
pub async fn connection_check(rt: tauri::State<'_, Runtime>) -> std::result::Result<Value, String> {
    let state = local::request(&rt.socket, "snapshot", json!({}))
        .await
        .map_err(|e| e.to_string())?;
    let checks = &state["connection"]["checks"];
    Ok(
        json!({"checks":[{"name":"UDP / QUIC 可达","ok":checks["transport"]},{"name":"证书校验","ok":checks["certificate"]},{"name":"凭据认证","ok":checks["authentication"]},{"name":"协议一致","ok":checks["protocol"]}],"error":state["connection"]["error"],"note":"连接成功代表握手校验通过；未连接时请查看具体错误。"}),
    )
}
fn legacy_argument(value: &Value, key: &str) -> Result<String> {
    let args = value["ProgramArguments"]
        .as_array()
        .context("缺少启动参数")?;
    let mut found = Vec::new();
    for (i, a) in args.iter().enumerate() {
        if a == key {
            found.push(
                args.get(i + 1)
                    .and_then(Value::as_str)
                    .filter(|v| !v.starts_with("--"))
                    .context("参数缺少值")?
                    .to_owned(),
            );
        } else if let Some(v) = a.as_str().and_then(|a| a.strip_prefix(&format!("{key}="))) {
            found.push(v.to_owned());
        }
    }
    anyhow::ensure!(found.len() == 1, "缺少或重复参数 {key}");
    Ok(found.remove(0))
}

fn legacy_worker_path(value: &Value) -> Result<String> {
    match &value["EnvironmentVariables"]["PATH"] {
        Value::Null => Ok(String::new()),
        Value::String(path) => {
            anyhow::ensure!(!path.contains('\0'), "旧 PATH 配置无效");
            Ok(path.clone())
        }
        _ => anyhow::bail!("旧 PATH 配置必须为字符串"),
    }
}
#[tauri::command]
pub async fn migrate_legacy(app: tauri::AppHandle) -> std::result::Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let rt = app.state::<Runtime>();
        {
            // Share the short lifecycle lock with worker start and final exit.
            let mut child = rt.child.lock().unwrap();
            if rt.exiting.load(Ordering::SeqCst) {
                return Err("应用正在退出，未开始迁移".into());
            }
            if child
                .as_mut()
                .is_some_and(|c| c.try_wait().ok().flatten().is_none())
            {
                return Err("请先停止桌面执行器".into());
            }
            if rt
                .migrating
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return Err("迁移正在进行，请等待完成".into());
            }
        }
        struct Guard<'a>(&'a AtomicBool);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _guard = Guard(&rt.migrating);
        migrate_legacy_inner(&rt).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "迁移任务异常中断，请检查保留的备份".to_owned())?
}

fn migrate_legacy_inner(rt: &Runtime) -> Result<Value> {
    use crate::migration::MigrationFiles;
    anyhow::ensure!(
        !rt.child
            .lock()
            .unwrap()
            .as_mut()
            .is_some_and(|c| c.try_wait().ok().flatten().is_none()),
        "请先停止桌面执行器"
    );
    let home = std::env::var("HOME")?;
    let plist = PathBuf::from(home).join("Library/LaunchAgents/dev.macrun.worker.plist");
    anyhow::ensure!(plist.is_file(), "未找到旧 LaunchAgent 配置");
    let disabled = plist.with_extension("plist.macrun-disabled");
    anyhow::ensure!(!disabled.exists(), "存在旧迁移文件，请先检查它再迁移");
    let out = Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(&plist)
        .output()?;
    anyhow::ensure!(out.status.success(), "无法解析 LaunchAgent");
    let value: Value = serde_json::from_slice(&out.stdout)?;
    anyhow::ensure!(
        value["Label"] == "dev.macrun.worker",
        "LaunchAgent 标签不匹配"
    );
    anyhow::ensure!(
        value["ProgramArguments"][1] == "worker",
        "暂不支持经 shell 包装的旧执行器，请先检查启动配置"
    );
    let arg = |key: &str| legacy_argument(&value, key);
    let server = arg("--server")?;
    let cert = PathBuf::from(arg("--cert")?);
    let token_path = PathBuf::from(arg("--token-file")?);
    let old_data = PathBuf::from(arg("--data")?);
    let worker_path = legacy_worker_path(&value)?;
    anyhow::ensure!(
        cert.is_absolute() && token_path.is_absolute() && old_data.is_absolute(),
        "旧配置的证书、令牌和数据目录必须为绝对路径"
    );
    let has_config = value["ProgramArguments"].as_array().is_some_and(|args| {
        args.iter()
            .any(|v| v == "--config" || v.as_str().is_some_and(|s| s.starts_with("--config=")))
    });
    let backend_text = if has_config {
        let path = PathBuf::from(arg("--config")?);
        anyhow::ensure!(path.is_absolute(), "旧后端配置必须为绝对路径");
        let text = std::fs::read_to_string(path).context("无法读取旧后端配置")?;
        let _: macrun::config::WorkerConfig = toml::from_str(&text)
            .map_err(|_| anyhow::anyhow!("旧后端配置格式不兼容，请检查 worker.toml"))?;
        Some(text)
    } else {
        None
    };
    let cert_bytes = std::fs::read(&cert).context("无法读取旧服务器证书")?;
    let token = std::fs::read_to_string(token_path).context("无法读取旧令牌文件")?;
    anyhow::ensure!(!token.trim().is_empty(), "旧令牌为空");
    let mut files = MigrationFiles::prepare(&rt.data, &old_data)?;
    let backup = files.stage.join("launchagent.plist.bak");
    wire::private_write(&backup, &std::fs::read(&plist)?)?;
    let copied_cert = files.stage.join("certificate.der");
    wire::private_write(&copied_cert, &cert_bytes)?;
    let backend_config = if let Some(text) = backend_text {
        let path = files.stage.join("worker.toml");
        wire::private_write(&path, text.as_bytes())?;
        path.to_string_lossy().into_owned()
    } else {
        String::new()
    };
    let account = uuid::Uuid::new_v4().to_string();
    security_framework::passwords::set_generic_password(
        "dev.macrun.desktop",
        &account,
        token.trim().as_bytes(),
    )
    .map_err(|_| anyhow::anyhow!("无法将旧连接凭据保存到钥匙串；旧服务未停止"))?;
    let settings = Settings {
        server,
        cert: copied_cert.to_string_lossy().into(),
        token_file: "/dev/null".into(),
        backend_config,
        keychain_account: account,
        certificate_fingerprint: blake3::hash(&cert_bytes).to_hex().to_string(),
        worker_path,
    };
    wire::atomic_json(&files.stage.join("connection.json"), &settings)?;
    let uid = Command::new("id").arg("-u").output()?;
    anyhow::ensure!(uid.status.success(), "无法确定当前用户，旧服务未停止");
    let domain = format!("gui/{}", String::from_utf8_lossy(&uid.stdout).trim());
    let was_running = legacy_running();
    let mut source_lock = None;
    let result: Result<()> = (|| {
        if was_running {
            anyhow::ensure!(
                Command::new("launchctl")
                    .arg("bootout")
                    .arg(&domain)
                    .arg(&plist)
                    .status()?
                    .success(),
                "无法停止旧服务"
            );
            anyhow::ensure!(!legacy_running(), "旧服务尚未卸载");
        }
        // Holding the original worker lock prevents another instance from changing
        // task results between the second task check and the completed copy.
        for attempt in 0..50 {
            match wire::lock(&old_data) {
                Ok(lock) => {
                    source_lock = Some(lock);
                    break;
                }
                Err(error) if attempt == 49 => return Err(error.context("旧执行器仍占用数据目录")),
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        files.copy_state(&old_data)?;
        std::fs::rename(&plist, &disabled)?;
        files.install()?;
        Ok(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        if let Err(e) = files.rollback() {
            failures.push(format!("恢复桌面数据失败：{e}"));
        }
        if disabled.exists() {
            if plist.exists() {
                failures.push("原服务入口已变化，未覆盖".into());
            } else if let Err(e) = std::fs::rename(&disabled, &plist) {
                failures.push(format!("恢复旧服务入口失败：{e}"));
            }
        }
        drop(source_lock.take());
        if was_running && !legacy_running() {
            match Command::new("launchctl")
                .arg("bootstrap")
                .arg(&domain)
                .arg(&plist)
                .status()
            {
                Ok(status) if status.success() && legacy_running() => {}
                _ => failures.push("旧服务恢复启动失败，请使用保留的备份恢复".into()),
            }
        }
        if failures.is_empty() {
            anyhow::bail!("迁移未完成，原配置已保留，旧服务已恢复原状态：{error}");
        }
        anyhow::bail!(
            "迁移失败：{error}；{}。备份：{}",
            failures.join("；"),
            backup.display()
        );
    }
    *rt.settings.lock().unwrap() = settings;
    Ok(
        json!({"backup":backup,"disabled":disabled,"note":"连接、任务去重与同步状态已复制到新版；旧数据目录保持原样。"}),
    )
}

unsafe extern "C" {
    pub fn macrun_monitor_start();
    fn macrun_set_application_icon(bytes: *const u8, length: usize) -> bool;
    pub fn macrun_graphical_session() -> bool;
    pub fn macrun_awake_active() -> bool;
    pub fn macrun_input_sequence() -> u64;
    pub fn macrun_input_available() -> bool;
    pub fn macrun_keep_awake(enabled: bool) -> bool;
}
#[tauri::command]
pub fn input_status() -> Value {
    json!({"available":unsafe{macrun_input_available()}})
}
#[tauri::command]
pub fn notify_task(status: String, app: tauri::AppHandle) -> std::result::Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    let title = match status.as_str() {
        "failed" => "任务执行失败",
        "timed_out" => "任务已超时",
        "unknown" => "任务结果需要核对",
        "awaiting_approval" => "有请求等待你确认，60 秒内未处理将过期",
        _ => return Ok(()),
    };
    app.notification()
        .builder()
        .title("Macrun")
        .body(title)
        .show()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_workspace_requires_the_recorded_kind_id_and_path() {
        let task =
            json!({"task_id":"command-1","kind":"exec.start","arguments":{"cwd":"/tmp/work"}});
        assert_eq!(
            task_workspace(&task, "command-1", "/tmp/work").unwrap(),
            PathBuf::from("/tmp/work")
        );
        assert!(task_workspace(&task, "other-task", "/tmp/work").is_err());
        assert!(task_workspace(&task, "command-1", "/tmp/unrelated").is_err());
        let sync =
            json!({"task_id":"sync-1","kind":"sync","arguments":{"remote_root":"/tmp/sync"}});
        assert_eq!(
            task_workspace(&sync, "sync-1", "/tmp/sync").unwrap(),
            PathBuf::from("/tmp/sync")
        );
        let backend = json!({"task_id":"call-1","kind":"mcp.call","arguments":{"cwd":"/tmp/work"}});
        assert!(task_workspace(&backend, "call-1", "/tmp/work").is_err());
        assert!(
            task_workspace(
                &json!({"task_id":"gone","kind":"exec.start"}),
                "gone",
                "/tmp/work"
            )
            .is_err()
        );
    }
    #[test]
    fn permission_links_only_allow_known_system_panes() {
        assert!(
            permission_url("accessibility")
                .unwrap()
                .ends_with("?Privacy_Accessibility")
        );
        assert!(
            permission_url("screen")
                .unwrap()
                .ends_with("?Privacy_ScreenCapture")
        );
        assert!(
            permission_url("input")
                .unwrap()
                .ends_with("?Privacy_ListenEvent")
        );
        assert!(permission_url("https://example.com").is_err());
    }
    #[test]
    fn migration_preserves_only_worker_path_without_exposing_it_to_ui() {
        let plist = json!({"EnvironmentVariables": {"PATH": "/opt/homebrew/bin:/usr/bin", "UNRELATED_SECRET": "test-only"}});
        let settings = Settings {
            worker_path: legacy_worker_path(&plist).unwrap(),
            ..Default::default()
        };
        let stored = serde_json::to_value(&settings).unwrap();
        assert_eq!(stored["worker_path"], "/opt/homebrew/bin:/usr/bin");
        assert!(settings.public_value().get("worker_path").is_none());
        let mut old = stored;
        old.as_object_mut().unwrap().remove("worker_path");
        assert!(
            serde_json::from_value::<Settings>(old)
                .unwrap()
                .worker_path
                .is_empty()
        );
        let mut command = Command::new("unused-test-worker");
        settings.apply_worker_environment(&mut command);
        let env: Vec<_> = command.get_envs().collect();
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].0, "PATH");
        assert_eq!(env[0].1.unwrap(), "/opt/homebrew/bin:/usr/bin");
        assert!(legacy_worker_path(&json!({})).unwrap().is_empty());
        assert!(legacy_worker_path(&json!({"EnvironmentVariables":{"PATH":true}})).is_err());
    }
    #[test]
    fn migration_arguments_support_both_forms_and_reject_ambiguity() {
        let v = json!({"ProgramArguments":["macrun","worker","--server","127.0.0.1:7443","--cert=/tmp/cert.der"]});
        assert_eq!(legacy_argument(&v, "--server").unwrap(), "127.0.0.1:7443");
        assert_eq!(legacy_argument(&v, "--cert").unwrap(), "/tmp/cert.der");
        assert!(legacy_argument(&v, "--missing").is_err());
        assert!(
            legacy_argument(
                &json!({"ProgramArguments":["--server","--cert","file"]}),
                "--server"
            )
            .is_err()
        );
        assert!(
            legacy_argument(
                &json!({"ProgramArguments":["--server=a","--server=b"]}),
                "--server"
            )
            .is_err()
        );
    }
}
