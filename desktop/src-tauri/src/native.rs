use super::*;
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn CGPreflightScreenCaptureAccess() -> bool;
}
#[tauri::command]
pub fn permissions() -> Value {
    json!({"accessibility":unsafe {AXIsProcessTrusted()},"screen_recording":unsafe{CGPreflightScreenCaptureAccess()},"graphical_session":unsafe{macrun_graphical_session()},"keep_awake":unsafe{macrun_awake_active()},"scope":"desktop_app","note":"后端有独立权限；请通过后端实拍验证。"})
}
#[tauri::command]
pub fn open_permission(kind: String) -> std::result::Result<(), String> {
    let pane = match kind.as_str() {
        "accessibility" => "Privacy_Accessibility",
        "screen" => "Privacy_ScreenCapture",
        "input" => "Privacy_ListenEvent",
        _ => return Err("未知权限".into()),
    };
    Command::new("open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{pane}"
        ))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn open_workspace(
    root: String,
    terminal: bool,
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<(), String> {
    let snapshot = local::request(&rt.socket, "snapshot", json!({}))
        .await
        .map_err(|e| e.to_string())?;
    if !snapshot["workspaces"]
        .as_array()
        .is_some_and(|w| w.iter().any(|w| w["root"] == root))
    {
        return Err("未知工作区".into());
    }
    let p = macrun::config::expand(&root);
    if !p.is_dir() {
        return Err("工作区目录不存在".into());
    }
    let mut cmd = Command::new("open");
    if terminal {
        cmd.args(["-a", "Terminal"]);
    }
    cmd.arg(p).spawn().map_err(|e| e.to_string())?;
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
        anyhow::ensure!(
            !rt.child.lock().unwrap().as_mut().is_some_and(|c| c
                .try_wait()
                .ok()
                .flatten()
                .is_none()),
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
        let mut settings = rt.settings.lock().unwrap();
        settings.backend_config = path.to_string_lossy().into();
        wire::atomic_json(&rt.data.join("connection.json"), &*settings)?;
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
#[tauri::command]
pub fn migrate_legacy(rt: tauri::State<Runtime>) -> std::result::Result<Value, String> {
    let result: Result<Value> = (|| {
        anyhow::ensure!(
            !rt.child.lock().unwrap().as_mut().is_some_and(|c| c
                .try_wait()
                .ok()
                .flatten()
                .is_none()),
            "请先停止桌面执行器"
        );
        let home = std::env::var("HOME")?;
        let plist = PathBuf::from(home).join("Library/LaunchAgents/dev.macrun.worker.plist");
        anyhow::ensure!(plist.is_file(), "未找到旧 LaunchAgent 配置");
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
        let arg = |key: &str| legacy_argument(&value, key);
        let server = arg("--server")?;
        let cert = arg("--cert")?;
        let token_path = arg("--token-file")?;
        let backend_config = arg("--config").unwrap_or_default();
        if !backend_config.is_empty() {
            toml_config(&backend_config)?;
        }
        if let Ok(old_data) = arg("--data")
            && let Ok(entries) = std::fs::read_dir(PathBuf::from(old_data).join("tasks"))
        {
            for entry in entries {
                if let Ok(bytes) = std::fs::read(entry?.path().join("result.json")) {
                    let task: Value = serde_json::from_slice(&bytes)?;
                    anyhow::ensure!(
                        task.get("ended_at").is_some(),
                        "旧执行器有未结束任务，请先停止任务再迁移"
                    );
                }
            }
        }
        let token = std::fs::read_to_string(token_path)?;
        anyhow::ensure!(!token.trim().is_empty(), "旧令牌为空");
        let account = uuid::Uuid::new_v4().to_string();
        security_framework::passwords::set_generic_password(
            "dev.macrun.desktop",
            &account,
            token.trim().as_bytes(),
        )?;
        let copied_cert = rt.data.join(format!("migrated-{account}.der"));
        std::fs::copy(&cert, &copied_cert)?;
        let backup = rt
            .data
            .join(format!("launchagent-{}.plist.bak", macrun::model::now()));
        std::fs::copy(&plist, &backup)?;
        let uid = Command::new("id").arg("-u").output()?;
        let uid = String::from_utf8_lossy(&uid.stdout).trim().to_owned();
        let disabled = plist.with_extension("plist.macrun-disabled");
        anyhow::ensure!(!disabled.exists(), "存在旧迁移文件，请先检查它再迁移");
        let settings = Settings {
            server,
            cert: copied_cert.to_string_lossy().into(),
            token_file: "/dev/null".into(),
            backend_config,
            keychain_account: account,
            certificate_fingerprint: blake3::hash(&std::fs::read(&copied_cert)?)
                .to_hex()
                .to_string(),
        };
        let candidate = rt.data.join("connection-migration.json");
        wire::atomic_json(&candidate, &settings)?;
        let was_running = legacy_running();
        if was_running {
            anyhow::ensure!(
                Command::new("launchctl")
                    .arg("bootout")
                    .arg(format!("gui/{uid}"))
                    .arg(&plist)
                    .status()?
                    .success(),
                "无法停止旧服务；备份已保留"
            );
        }
        // Preserve the original autoload entry for rollback.
        if let Err(e) = std::fs::rename(&plist, &disabled)
            .and_then(|_| std::fs::rename(&candidate, rt.data.join("connection.json")))
        {
            if disabled.exists() {
                let _ = std::fs::rename(&disabled, &plist);
            }
            if was_running {
                let _ = Command::new("launchctl")
                    .arg("bootstrap")
                    .arg(format!("gui/{uid}"))
                    .arg(&plist)
                    .status();
            }
            return Err(e.into());
        }
        *rt.settings.lock().unwrap() = settings;
        Ok(
            json!({"backup":backup,"disabled":disabled,"note":"旧任务记录仍保留在原数据目录；确认无任务运行后迁移。"}),
        )
    })();
    result.map_err(|e| e.to_string())
}

unsafe extern "C" {
    pub fn macrun_monitor_start();
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
