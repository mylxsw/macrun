//! Optional local Cua Driver setup. Commands and the installer URL are fixed here,
//! never supplied by the webview. Existing MCP entries are preserved.
use super::*;
use std::{os::unix::fs::PermissionsExt, path::Path};
use tokio::process::Command as AsyncCommand;

static SETUP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const INSTALLER: &str = "https://cua.ai/driver/install.sh";

fn driver_command(command: &str) -> bool {
    Path::new(command)
        .file_name()
        .is_some_and(|v| v == "cua-driver")
}
fn config(rt: &Runtime) -> Result<macrun::config::WorkerConfig> {
    let path = rt.settings.lock().unwrap().backend_config.clone();
    if path.is_empty() {
        Ok(Default::default())
    } else {
        Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
    }
}
fn candidates(
    config: &macrun::config::WorkerConfig,
    home: &Path,
    path: &std::ffi::OsStr,
) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = config
        .mcp
        .values()
        .filter(|b| driver_command(&b.command))
        .map(|b| macrun::config::expand(&b.command))
        .filter(|p| p.is_absolute())
        .collect();
    paths.extend([
        PathBuf::from("/Applications/CuaDriver.app/Contents/MacOS/cua-driver"),
        home.join("Applications/CuaDriver.app/Contents/MacOS/cua-driver"),
        home.join(".local/bin/cua-driver"),
    ]);
    paths.extend(
        std::env::split_paths(path)
            .filter(|p| p.is_absolute())
            .map(|p| p.join("cua-driver")),
    );
    paths.dedup();
    paths
}
fn executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
fn supported(version: &str) -> bool {
    version
        .trim()
        .split('.')
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .is_some_and(|v| v >= 14)
}

// Bound subprocess lifetime and output. Killing the process group also cleans up
// curl/bash descendants on timeout or cancellation, including a dropped future.
struct ProcessGroup(u32);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        macrun::process::signal_group(self.0, 9);
    }
}
async fn output(command: &mut AsyncCommand, timeout: Duration) -> Result<String> {
    use tokio::io::AsyncReadExt;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    let mut child = command.spawn().context("无法启动 Cua Driver 操作")?;
    let _group = ProcessGroup(child.id().context("missing process id")?);
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let drain = |stream: Box<dyn tokio::io::AsyncRead + Unpin + Send>| async move {
        let mut stream = stream;
        let mut tail = Vec::new();
        let mut buf = [0; 4096];
        loop {
            let count = stream.read(&mut buf).await?;
            if count == 0 {
                break;
            }
            tail.extend_from_slice(&buf[..count]);
            if tail.len() > 16384 {
                tail.drain(..tail.len() - 16384);
            }
        }
        Ok::<_, std::io::Error>(String::from_utf8_lossy(&tail).into_owned())
    };
    let (status, stdout, stderr) = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            child.wait(),
            drain(Box::new(stdout)),
            drain(Box::new(stderr))
        )
    })
    .await
    .context("操作超时，已停止操作进程；请检查网络或系统授权后重试")??;
    anyhow::ensure!(
        status.success(),
        "操作失败：{}",
        if stderr.trim().is_empty() {
            &stdout
        } else {
            &stderr
        }
    );
    Ok(stdout.trim().to_owned())
}
async fn inspect(rt: &Runtime) -> Result<Value> {
    let os = output(
        AsyncCommand::new("/usr/bin/sw_vers").arg("-productVersion"),
        Duration::from_secs(5),
    )
    .await?;
    if !supported(&os) {
        return Ok(
            json!({"state":"unsupported","detail":"Cua Driver 需要 macOS 14 或更新版本；可跳过桌面控制。"}),
        );
    }
    let cfg = config(rt)?;
    let home = PathBuf::from(std::env::var_os("HOME").context("missing home")?);
    inspect_candidates(
        &cfg,
        candidates(&cfg, &home, &std::env::var_os("PATH").unwrap_or_default()),
    )
    .await
}
async fn inspect_candidates(
    cfg: &macrun::config::WorkerConfig,
    paths: Vec<PathBuf>,
) -> Result<Value> {
    let mut failure = None;
    for path in paths {
        if !executable(&path) {
            continue;
        }
        match output(
            AsyncCommand::new(&path).arg("--version"),
            Duration::from_secs(5),
        )
        .await
        {
            Ok(version) if !version.is_empty() => {
                let resolved = std::fs::canonicalize(&path)?;
                let configured = cfg.mcp.values().any(|b| {
                    driver_command(&b.command)
                        && b.args == ["mcp"]
                        && std::fs::canonicalize(macrun::config::expand(&b.command))
                            .is_ok_and(|p| p == resolved)
                });
                return Ok(
                    json!({"state":"ready","path":resolved,"version":version,"configured":configured}),
                );
            }
            Ok(_) => failure = Some("Cua Driver 没有返回版本信息".to_owned()),
            Err(e) => failure = Some(e.to_string()),
        }
    }
    Ok(match failure {
        Some(error) => json!({"state":"broken","detail":error}),
        None => json!({"state":"missing","detail":"尚未安装 Cua Driver"}),
    })
}
#[tauri::command]
pub async fn cua_status(rt: tauri::State<'_, Runtime>) -> std::result::Result<Value, String> {
    let _guard = match SETUP.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            return Ok(json!({"state":"busy","detail":"正在处理 Cua Driver，请稍后重新检测"}));
        }
    };
    inspect(&rt).await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn install_cua_driver(
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<Value, String> {
    let _guard = SETUP.try_lock().map_err(|_| "Cua Driver 操作正在进行")?;
    let result: Result<Value> = async {
        let status = inspect(&rt).await?;
        anyhow::ensure!(
            status["state"] != "unsupported",
            "Cua Driver 需要 macOS 14 或更新版本"
        );
        if status["state"] == "ready" {
            return Ok(status);
        }
        if status["state"] == "broken" {
            let _idle = configuration_edit_guard(&rt)?;
        }
        // A private unique directory prevents a partial download being executed
        // and keeps the official wrapper from finding an unrelated sibling script.
        let directory = rt
            .data
            .join(format!("cua-install-{}", uuid::Uuid::new_v4()));
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let script = directory.join("install.sh");
        output(
            AsyncCommand::new("/usr/bin/curl")
                .args([
                    "--fail",
                    "--silent",
                    "--show-error",
                    "--location",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
                    "--connect-timeout",
                    "15",
                    "--max-time",
                    "60",
                    "--output",
                ])
                .arg(&script)
                .arg(INSTALLER),
            Duration::from_secs(65),
        )
        .await?;
        output(
            AsyncCommand::new("/bin/bash")
                .arg(&script)
                .arg("--no-modify-path"),
            Duration::from_secs(300),
        )
        .await?;
        let installed = inspect(&rt).await?;
        anyhow::ensure!(
            installed["state"] == "ready",
            "安装结束但未检测到可用的 Cua Driver，请重新检测或重试安装"
        );
        Ok(installed)
    }
    .await;
    result.map_err(|e| e.to_string())
}
fn with_driver(mut config: macrun::config::WorkerConfig, path: &str) -> Result<String> {
    // Idempotent, and never overwrite a custom backend occupying the usual name.
    if !config
        .mcp
        .values()
        .any(|b| b.command == path && b.args == ["mcp"])
    {
        let mut name = "cua".to_owned();
        let mut suffix = 2;
        while config.mcp.contains_key(&name) {
            name = format!("cua-{suffix}");
            suffix += 1;
        }
        config.mcp.insert(
            name,
            macrun::config::Backend {
                command: path.into(),
                args: vec!["mcp".into()],
                env: Default::default(),
                cwd: None,
            },
        );
    }
    Ok(toml::to_string_pretty(&config)?)
}
#[tauri::command]
pub async fn configure_cua_driver(
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<(), String> {
    let _guard = SETUP.try_lock().map_err(|_| "Cua Driver 操作正在进行")?;
    let status = inspect(&rt).await.map_err(|e| e.to_string())?;
    let path = status["path"].as_str().ok_or("请先安装可用的 Cua Driver")?;
    if status["configured"] == true {
        return Ok(());
    }
    let text =
        with_driver(config(&rt).map_err(|e| e.to_string())?, path).map_err(|e| e.to_string())?;
    native::save_backend_text(&text, &rt)
}
#[tauri::command]
pub async fn grant_cua_permissions(
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<String, String> {
    let _guard = SETUP.try_lock().map_err(|_| "Cua Driver 操作正在进行")?;
    let status = inspect(&rt).await.map_err(|e| e.to_string())?;
    let path = status["path"].as_str().ok_or("请先安装可用的 Cua Driver")?;
    // Official grant command uses LaunchServices for the app's TCC identity and
    // verifies capture. A denied grant is an error; opening Settings isn't success.
    output(
        AsyncCommand::new(path).args(["permissions", "grant"]),
        Duration::from_secs(120),
    )
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_custom_backends_and_is_idempotent() {
        let original =
            "[mcp.cua]\ncommand='/custom/backend'\nargs=['serve']\n[mcp.cua.env]\nSETTING='kept'\n";
        let text = with_driver(
            toml::from_str(original).unwrap(),
            "/Applications/CuaDriver.app/Contents/MacOS/cua-driver",
        )
        .unwrap();
        let cfg: macrun::config::WorkerConfig = toml::from_str(&text).unwrap();
        assert_eq!(cfg.mcp["cua"].command, "/custom/backend");
        assert_eq!(cfg.mcp["cua"].env["SETTING"], "kept");
        assert_eq!(cfg.mcp["cua-2"].args, ["mcp"]);
        assert_eq!(
            with_driver(cfg, "/Applications/CuaDriver.app/Contents/MacOS/cua-driver").unwrap(),
            text
        );
    }
    #[test]
    fn detects_os_boundary_and_executable_candidates() {
        for version in ["12.7", "13.6", "", "unknown"] {
            assert!(!supported(version));
        }
        for version in ["14.0", "15.7", "26.0\n"] {
            assert!(supported(version));
        }
        let cfg = toml::from_str("[mcp.custom]\ncommand='/some/cua-driver'\n").unwrap();
        let paths = candidates(
            &cfg,
            Path::new("/test-home"),
            std::ffi::OsStr::new("/opt/bin:.:/usr/bin"),
        );
        assert_eq!(paths[0], Path::new("/some/cua-driver"));
        assert!(paths.contains(&PathBuf::from("/test-home/.local/bin/cua-driver")));
        assert!(paths.contains(&PathBuf::from("/opt/bin/cua-driver")));
        assert!(!paths.contains(&PathBuf::from("./cua-driver")));
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cua-driver");
        assert!(!executable(&path));
        std::fs::write(&path, "fixture").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!executable(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(executable(&path));
    }
    #[tokio::test]
    async fn detects_missing_broken_and_installed_drivers_without_system_changes() {
        let temp = tempfile::tempdir().unwrap();
        let driver = temp.path().join("cua-driver");
        let cfg = macrun::config::WorkerConfig::default();
        assert_eq!(
            inspect_candidates(&cfg, vec![driver.clone()])
                .await
                .unwrap()["state"],
            "missing"
        );
        std::fs::write(&driver, "#!/bin/sh\necho broken >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&driver, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            inspect_candidates(&cfg, vec![driver.clone()])
                .await
                .unwrap()["state"],
            "broken"
        );
        std::fs::write(&driver, "#!/bin/sh\necho 'cua-driver fixture'\n").unwrap();
        let cfg = toml::from_str(&with_driver(cfg, driver.to_str().unwrap()).unwrap()).unwrap();
        let result = inspect_candidates(&cfg, vec![driver.clone()])
            .await
            .unwrap();
        assert_eq!(result["state"], "ready");
        assert_eq!(result["configured"], true);
        std::fs::write(&driver, "#!/bin/sh\nexit 0\n").unwrap();
        assert_eq!(
            inspect_candidates(&cfg, vec![driver]).await.unwrap()["state"],
            "broken"
        );
    }
    #[tokio::test]
    async fn command_failure_timeout_and_bounded_output() {
        assert_eq!(
            output(
                AsyncCommand::new("/bin/echo").arg("version"),
                Duration::from_secs(1)
            )
            .await
            .unwrap(),
            "version"
        );
        let error = output(
            AsyncCommand::new("/bin/sh").args(["-c", "echo denied >&2; exit 1"]),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("denied"));
        assert!(
            output(
                AsyncCommand::new("/bin/sleep").arg("10"),
                Duration::from_millis(10)
            )
            .await
            .is_err()
        );
        let out = output(
            AsyncCommand::new("/bin/sh").args(["-c", "yes x | head -c 30000"]),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert!(out.len() <= 16384);
    }
}
