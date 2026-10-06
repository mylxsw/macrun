//! Optional local Cua Driver setup. Commands and the installer URL are fixed here,
//! never supplied by the webview. Existing MCP entries are preserved.
use super::*;
use crate::localization::{interpolate as tr_format, text as tr};
use std::{os::unix::fs::PermissionsExt, path::Path};
use tokio::process::Command as AsyncCommand;

static SETUP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static GRANT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
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
    let mut child = command.spawn().context(tr("无法启动 Cua Driver 操作"))?;
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
    .context(tr("操作超时，已停止操作进程；请检查网络或系统授权后重试"))??;
    anyhow::ensure!(
        status.success(),
        tr_format(
            "操作失败：{0}",
            &[if stderr.trim().is_empty() {
                &stdout
            } else {
                &stderr
            }
            .to_string()]
        )
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
            json!({"state":"unsupported","detail":tr("Cua Driver 需要 macOS 14 或更新版本；可跳过桌面控制。")}),
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
            Ok(_) => failure = Some(tr("Cua Driver 没有返回版本信息").to_owned()),
            Err(e) => failure = Some(e.to_string()),
        }
    }
    Ok(match failure {
        Some(error) => json!({"state":"broken","detail":error}),
        None => {
            json!({"state":"missing","detail":tr("尚未安装 Cua Driver")})
        }
    })
}
#[tauri::command]
pub async fn cua_status(rt: tauri::State<'_, Runtime>) -> std::result::Result<Value, String> {
    let mut status = {
        let _guard = match SETUP.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                return Ok(
                    json!({"state":"busy","detail":tr("正在处理 Cua Driver，请稍后重新检测")}),
                );
            }
        };
        inspect(&rt).await.map_err(|e| e.to_string())?
    };
    if let Some(path) = status["path"].as_str() {
        status["permissions"] = permission_status(Path::new(path), Duration::from_secs(5)).await;
    }
    Ok(status)
}
async fn permission_status(path: &Path, timeout: Duration) -> Value {
    // Query the app-owned daemon without prompting or capturing the screen.
    // A restarting/unavailable daemon must never be reported as authorized.
    let result: Result<Value> = async {
        let text = output(
            AsyncCommand::new(path).args(["permissions", "status", "--json"]),
            timeout,
        )
        .await?;
        Ok(serde_json::from_str(&text)?)
    }
    .await;
    match result {
        Ok(value) => json!({
            "accessibility": value["accessibility"].as_bool(),
            "screen_recording": value["screen_recording"].as_bool(),
        }),
        Err(_) => json!({
            "accessibility": null,
            "screen_recording": null,
            "detail": tr("暂无法读取 CuaDriver 权限，重启后将自动重新检测。"),
        }),
    }
}
#[tauri::command]
pub async fn install_cua_driver(
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<Value, String> {
    let _guard = SETUP
        .try_lock()
        .map_err(|_| tr("Cua Driver 操作正在进行"))?;
    let result: Result<Value> = async {
        let status = inspect(&rt).await?;
        anyhow::ensure!(
            status["state"] != "unsupported",
            tr("Cua Driver 需要 macOS 14 或更新版本")
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
            tr("安装结束但未检测到可用的 Cua Driver，请重新检测或重试安装")
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
    let _guard = SETUP
        .try_lock()
        .map_err(|_| tr("Cua Driver 操作正在进行"))?;
    let status = inspect(&rt).await.map_err(|e| e.to_string())?;
    let path = status["path"]
        .as_str()
        .ok_or(tr("请先安装可用的 Cua Driver"))?;
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
    grant_permissions(&rt, Duration::from_secs(120)).await
}
async fn grant_permissions(rt: &Runtime, timeout: Duration) -> std::result::Result<String, String> {
    let _grant = GRANT
        .try_lock()
        .map_err(|_| tr("CuaDriver 授权操作正在进行"))?;
    // Human approval and the driver's restart can outlive this inspection.
    // Keep duplicate grants serialized, but release setup so status can refresh.
    let path = {
        // A periodic status read may be in flight when the user clicks grant.
        let _guard = SETUP.lock().await;
        let status = inspect(rt).await.map_err(|e| e.to_string())?;
        status["path"]
            .as_str()
            .ok_or(tr("请先安装可用的 Cua Driver"))?
            .to_owned()
    };
    // Official grant command uses LaunchServices for the app's TCC identity and
    // verifies capture. A denied grant is an error; opening Settings isn't success.
    output(
        AsyncCommand::new(path).args(["permissions", "grant"]),
        timeout,
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
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn grant_keeps_status_available_rejects_duplicates_and_releases_on_timeout() {
        let temp = tempfile::tempdir().unwrap();
        let driver = temp.path().join("cua-driver");
        let started = temp.path().join("started");
        let finished = temp.path().join("finished");
        std::fs::write(
            &driver,
            r#"#!/bin/sh
case "$*" in
  --version) echo 'cua-driver fixture';;
  'permissions grant')
    touch "$(dirname "$0")/started"
    while [ ! -f "$(dirname "$0")/finished" ]; do sleep 0.01; done
    echo verified;;
  'permissions status --json') echo '{"accessibility":true,"screen_recording":true}';;
  *) exit 2;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&driver, std::fs::Permissions::from_mode(0o700)).unwrap();
        let backend_config = temp.path().join("worker.toml");
        std::fs::write(
            &backend_config,
            with_driver(Default::default(), driver.to_str().unwrap()).unwrap(),
        )
        .unwrap();
        let rt = Runtime {
            data: temp.path().to_path_buf(),
            socket: temp.path().join("control.sock"),
            child: Mutex::new(None),
            snapshot: Mutex::new(Value::Null),
            settings: Mutex::new(Settings {
                backend_config: backend_config.to_str().unwrap().to_owned(),
                ..Default::default()
            }),
            preferences: Mutex::new(Preferences::default()),
            exiting: AtomicBool::new(false),
            starting: AtomicBool::new(false),
            desired_running: AtomicBool::new(false),
            migrating: AtomicBool::new(false),
            window_layout: Mutex::new(WindowLayout::default()),
            system_state: Mutex::new(None),
        };
        let inspection = SETUP.lock().await;
        let grant = grant_permissions(&rt, Duration::from_secs(30));
        let check = async {
            drop(inspection);
            tokio::time::timeout(Duration::from_secs(15), async {
                while !started.exists() {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .map_err(|e| e.to_string())?;
            let status = {
                let _guard = SETUP
                    .try_lock()
                    .expect("grant must not hold the setup lock");
                inspect(&rt).await.unwrap()
            };
            assert_eq!(status["state"], "ready");
            assert_eq!(
                permission_status(&driver, Duration::from_secs(1)).await["screen_recording"],
                true
            );
            assert!(
                grant_permissions(&rt, Duration::from_millis(10))
                    .await
                    .unwrap_err()
                    .contains("授权操作正在进行")
            );
            std::fs::write(&finished, "").unwrap();
            Ok::<_, String>(())
        };
        let (result, check) = tokio::join!(grant, check);
        assert!(check.is_ok(), "check: {check:?}, grant: {result:?}");
        assert_eq!(result.unwrap(), "verified");
        assert!(GRANT.try_lock().is_ok());
        std::fs::remove_file(&finished).unwrap();
        assert!(
            grant_permissions(&rt, Duration::from_millis(10))
                .await
                .unwrap_err()
                .contains("超时")
        );
        assert!(GRANT.try_lock().is_ok());
        assert!(SETUP.try_lock().is_ok());
    }
    #[tokio::test]
    async fn reads_driver_permissions_and_recovers_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let driver = temp.path().join("cua-driver");
        std::fs::write(&driver, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&driver, std::fs::Permissions::from_mode(0o700)).unwrap();
        let timeout = Duration::from_secs(1);
        let unknown = permission_status(&driver, timeout).await;
        assert!(unknown["accessibility"].is_null());
        assert!(unknown["screen_recording"].is_null());
        assert!(unknown["detail"].is_string());
        for (accessibility, recording) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            std::fs::write(&driver, format!(
                "#!/bin/sh\n[ \"$*\" = 'permissions status --json' ] || exit 2\necho '{{\"accessibility\":{accessibility},\"screen_recording\":{recording}}}'\n"
            )).unwrap();
            let value = permission_status(&driver, timeout).await;
            assert_eq!(value["accessibility"], accessibility);
            assert_eq!(value["screen_recording"], recording);
        }
        for payload in [
            "not-json",
            "{}",
            r#"{"accessibility":"unknown","screen_recording":null}"#,
        ] {
            std::fs::write(&driver, format!("#!/bin/sh\necho '{payload}'\n")).unwrap();
            let value = permission_status(&driver, timeout).await;
            assert!(value["accessibility"].is_null());
            assert!(value["screen_recording"].is_null());
        }
        std::fs::write(&driver, "#!/bin/sh\nsleep 10\n").unwrap();
        let value = permission_status(&driver, Duration::from_millis(10)).await;
        assert!(value["accessibility"].is_null());
        assert!(value["screen_recording"].is_null());
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
