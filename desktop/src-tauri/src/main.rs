#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod migration;
mod native;
mod supervisor;
mod tray;
use anyhow::{Context, Result};
use macrun::{local, wire};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Settings {
    server: String,
    cert: String,
    token_file: String,
    backend_config: String,
    #[serde(default)]
    keychain_account: String,
    #[serde(default)]
    certificate_fingerprint: String,
    #[serde(default)]
    worker_path: String,
}
impl Settings {
    fn public_value(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("settings serialize");
        value.as_object_mut().unwrap().remove("worker_path");
        value
    }

    fn apply_worker_environment(&self, command: &mut Command) {
        if !self.worker_path.is_empty() {
            command.env("PATH", &self.worker_path);
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    show_overlay: bool,
    yield_input: bool,
    notifications: bool,
    keep_awake: bool,
    auto_connect: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            show_overlay: true,
            yield_input: true,
            notifications: true,
            keep_awake: true,
            auto_connect: true,
        }
    }
}
#[tauri::command]
fn save_preferences(
    preferences: Preferences,
    rt: tauri::State<Runtime>,
) -> std::result::Result<(), String> {
    wire::atomic_json(&rt.data.join("preferences.json"), &preferences)
        .map_err(|e| e.to_string())?;
    *rt.preferences.lock().unwrap() = preferences;
    Ok(())
}
struct Runtime {
    data: PathBuf,
    socket: PathBuf,
    child: Mutex<Option<Child>>,
    snapshot: Mutex<Value>,
    settings: Mutex<Settings>,
    preferences: Mutex<Preferences>,
    exiting: AtomicBool,
    starting: AtomicBool,
    desired_running: AtomicBool,
    migrating: AtomicBool,
    window_layout: Mutex<WindowLayout>,
}
#[derive(Default)]
struct WindowLayout {
    pairing: bool,
    normal_size: Option<tauri::PhysicalSize<u32>>,
}

#[tauri::command]
fn set_main_mode(
    pairing: bool,
    app: tauri::AppHandle,
    rt: tauri::State<Runtime>,
) -> std::result::Result<(), String> {
    let window = app.get_webview_window("main").ok_or("主窗口不可用")?;
    let mut layout = rt.window_layout.lock().unwrap();
    if layout.pairing == pairing {
        return Ok(());
    }
    if pairing {
        layout.normal_size = Some(window.inner_size().map_err(|e| e.to_string())?);
        window.set_decorations(false).map_err(|e| e.to_string())?;
        window
            .set_size(tauri::LogicalSize::new(800., 580.))
            .map_err(|e| e.to_string())?;
    } else {
        window.set_decorations(true).map_err(|e| e.to_string())?;
        if let Some(size) = layout.normal_size {
            window.set_size(size).map_err(|e| e.to_string())?;
        }
    }
    layout.pairing = pairing;
    Ok(())
}

#[tauri::command]
fn resize_panel(window: tauri::WebviewWindow, height: f64) -> std::result::Result<(), String> {
    if window.label() != "tray" || !height.is_finite() {
        return Err("仅快捷面板支持内容高度调整".into());
    }
    window
        .set_size(tauri::LogicalSize::new(
            352.,
            height.ceil().clamp(120., 720.),
        ))
        .map_err(|e| e.to_string())
}
fn tray_image(state: tray::TrayState) -> tauri::image::Image<'static> {
    tauri::image::Image::new_owned(tray::pixels(state), tray::WIDTH, tray::HEIGHT)
}

fn tray_status(app: &tauri::AppHandle, v: &Value) {
    let next = tray::Presentation::from_snapshot(v);
    let cache = app.state::<Mutex<Option<tray::Presentation>>>();
    let mut previous = cache.lock().unwrap();
    let Some(icon) = app.tray_by_id("macrun") else {
        return;
    };
    if previous.as_ref().map(|p| p.state) != Some(next.state) {
        // A separate set_icon + set_icon_as_template briefly displays a black
        // non-template image. Update both together, only when the state changes.
        if icon
            .set_icon_with_as_template(Some(tray_image(next.state)), next.state.template())
            .is_err()
        {
            return;
        }
        let _ = icon.set_tooltip(Some(format!("Macrun · {}", next.state.label())));
    }
    if previous.as_ref().map(|p| p.approvals) != Some(next.approvals) {
        let _ = icon.set_title((next.approvals > 0).then(|| next.approvals.to_string()));
    }
    *previous = Some(next);
}

fn overlay_visible(app: &tauri::AppHandle, visible: bool) {
    for (label, w) in app.webview_windows() {
        if label == "overlay" || label.starts_with("border-") {
            if visible {
                let _ = w.show();
            } else {
                let _ = w.hide();
            }
        }
    }
}
fn show(app: &tauri::AppHandle, route: Option<&str>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    if let Some(r) = route {
        let _ = app.emit("navigate", r);
    }
}
fn show_tray(app: &tauri::AppHandle, toggle: bool) {
    let Some(window) = app.get_webview_window("tray") else {
        return;
    };
    if toggle && window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    if let Some(icon) = app.tray_by_id("macrun")
        && let Ok(Some(rect)) = icon.rect()
    {
        let scale = window.scale_factor().unwrap_or(1.0);
        let anchor = rect.position.to_physical::<f64>(scale);
        let anchor_size = rect.size.to_physical::<f64>(scale);
        let monitor = window.available_monitors().ok().and_then(|monitors| {
            monitors.into_iter().find(|m| {
                let p = m.position();
                let size = m.size();
                anchor.x >= p.x as f64
                    && anchor.x < p.x as f64 + size.width as f64
                    && anchor.y >= p.y as f64
                    && anchor.y < p.y as f64 + size.height as f64
            })
        });
        let scale = monitor.as_ref().map_or(scale, |m| m.scale_factor());
        let mut x = anchor.x + anchor_size.width - 352. * scale;
        if let Some(monitor) = monitor {
            let left = monitor.position().x as f64 + 8. * scale;
            let right = monitor.position().x as f64 + monitor.size().width as f64 - 360. * scale;
            x = x.clamp(left, right.max(left));
        }
        let _ = window.set_position(tauri::PhysicalPosition::new(
            x,
            anchor.y + anchor_size.height + 6. * scale,
        ));
    }
    let _ = window.show();
    let _ = window.set_focus();
}

fn native_menu(app: &tauri::AppHandle) -> Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, WINDOW_SUBMENU_ID};
    let menu = Menu::default(app)?;
    if let Some(window) = menu
        .get(WINDOW_SUBMENU_ID)
        .and_then(|item| item.as_submenu().cloned())
    {
        window.insert(
            &MenuItem::with_id(app, "open-main", "打开主窗口", true, Some("CmdOrCtrl+O"))?,
            0,
        )?;
        window.insert(
            &MenuItem::with_id(
                app,
                "open-panel",
                "菜单栏面板",
                true,
                Some("CmdOrCtrl+Shift+M"),
            )?,
            1,
        )?;
        window.insert(&PredefinedMenuItem::separator(app)?, 2)?;
    }
    #[cfg(target_os = "macos")]
    if let Some(application) = menu
        .items()?
        .first()
        .and_then(|item| item.as_submenu().cloned())
    {
        application.insert(
            &MenuItem::with_id(app, "open-settings", "设置…", true, Some("CmdOrCtrl+,"))?,
            2,
        )?;
        // The native Quit item can terminate before the webview confirmation.
        // Route Cmd+Q through the same guarded flow as the tray's Quit action.
        if let Some(quit) = application.items()?.last() {
            application.remove(quit)?;
        }
        application.append(&MenuItem::with_id(
            app,
            "request-quit",
            "退出 Macrun",
            true,
            Some("CmdOrCtrl+Q"),
        )?)?;
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| match event.id().as_ref() {
        "open-main" => show(app, None),
        "open-panel" => show_tray(app, false),
        "open-settings" => show(app, Some("settings")),
        "request-quit" => request_exit(app),
        _ => {}
    });
    Ok(())
}
fn legacy_running() -> bool {
    #[cfg(target_os = "macos")]
    {
        let uid = Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .map(|v| String::from_utf8_lossy(&v.stdout).trim().to_string())
            .unwrap_or_default();
        Command::new("launchctl")
            .args(["print", &format!("gui/{uid}/dev.macrun.worker")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

fn legacy_plist_exists(launch_agents: &std::path::Path) -> bool {
    // Match the migration reader, which accepts a valid link to a plist file.
    std::fs::metadata(launch_agents.join("dev.macrun.worker.plist"))
        .is_ok_and(|metadata| metadata.is_file())
}

#[tauri::command]
fn app_state(app: tauri::AppHandle, rt: tauri::State<Runtime>) -> Value {
    let legacy_loaded = legacy_running();
    let legacy_detected = legacy_loaded
        || std::env::var_os("HOME").is_some_and(|home| {
            legacy_plist_exists(&PathBuf::from(home).join("Library/LaunchAgents"))
        });
    let mut child = rt.child.lock().unwrap();
    let running = child
        .as_mut()
        .is_some_and(|c| c.try_wait().ok().flatten().is_none());
    json!({"preferences":*rt.preferences.lock().unwrap(),"worker_running":running,"worker_starting":rt.starting.load(Ordering::SeqCst),"snapshot":*rt.snapshot.lock().unwrap(),"settings":rt.settings.lock().unwrap().public_value(),"data_dir":rt.data,"legacy_running":legacy_loaded,"legacy_detected":legacy_detected,"autostart":app.autolaunch().is_enabled().unwrap_or(false),"platform":std::env::consts::OS})
}

#[cfg(test)]
mod legacy_detection_tests {
    use super::legacy_plist_exists;

    #[test]
    fn detects_only_the_original_legacy_plist() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!legacy_plist_exists(directory.path()));
        std::fs::write(
            directory
                .path()
                .join("dev.macrun.worker.plist.macrun-disabled"),
            b"disabled backup",
        )
        .unwrap();
        assert!(!legacy_plist_exists(directory.path()));
        std::fs::write(
            directory.path().join("dev.macrun.worker.plist"),
            b"original configuration",
        )
        .unwrap();
        assert!(legacy_plist_exists(directory.path()));
    }

    #[test]
    fn accepts_valid_plist_links_but_rejects_directories_and_broken_links() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("dev.macrun.worker.plist")).unwrap();
        assert!(!legacy_plist_exists(directory.path()));

        let linked = tempfile::tempdir().unwrap();
        let target = linked.path().join("saved.plist");
        std::fs::write(&target, b"saved configuration").unwrap();
        std::os::unix::fs::symlink(&target, linked.path().join("dev.macrun.worker.plist")).unwrap();
        assert!(legacy_plist_exists(linked.path()));

        let broken = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(
            broken.path().join("missing.plist"),
            broken.path().join("dev.macrun.worker.plist"),
        )
        .unwrap();
        assert!(!legacy_plist_exists(broken.path()));
    }
}

#[tauri::command]
async fn save_settings(
    settings: Settings,
    app: tauri::AppHandle,
) -> std::result::Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let rt = app.state::<Runtime>();
        save_settings_inner(settings, &rt)
    })
    .await
    .map_err(|_| "保存连接配置的后台任务异常，请重试".to_owned())?
}

fn save_settings_inner(mut settings: Settings, rt: &Runtime) -> std::result::Result<(), String> {
    let original = rt.settings.lock().unwrap().clone();
    // This native migration setting is not editable through the webview.
    settings.worker_path = original.worker_path.clone();
    let result: Result<()> = (|| {
        {
            let _guard = configuration_edit_guard(rt)?;
        }
        anyhow::ensure!(!settings.server.trim().is_empty(), "请填写服务器地址");
        anyhow::ensure!(
            PathBuf::from(&settings.cert).is_file(),
            "服务器证书文件不存在"
        );
        anyhow::ensure!(
            !settings.keychain_account.is_empty() || PathBuf::from(&settings.token_file).is_file(),
            "令牌文件不存在"
        );
        if !settings.backend_config.is_empty() {
            let _: macrun::config::WorkerConfig = toml_config(&settings.backend_config)?;
        }
        let cert_bytes = std::fs::read(&settings.cert)?;
        let pinned = rt
            .data
            .join(format!("certificate-{}.der", uuid::Uuid::new_v4()));
        wire::private_write(&pinned, &cert_bytes)?;
        settings.cert = pinned.to_string_lossy().into();
        settings.certificate_fingerprint = blake3::hash(&cert_bytes).to_hex().to_string();
        if settings.keychain_account.is_empty() {
            let token = std::fs::read_to_string(&settings.token_file)?;
            anyhow::ensure!(!token.trim().is_empty(), "令牌文件为空");
            let account = uuid::Uuid::new_v4().to_string();
            security_framework::passwords::set_generic_password(
                "dev.macrun.desktop",
                &account,
                token.trim().as_bytes(),
            )?;
            settings.keychain_account = account;
            settings.token_file = "/dev/null".into();
        }
        // Authorization can take arbitrarily long. Recheck before changing the
        // active connection, keeping this short lock away from Keychain calls.
        let _guard = configuration_edit_guard(rt)?;
        let mut current = rt.settings.lock().unwrap();
        anyhow::ensure!(*current == original, "连接配置已变化，请重新检查后保存");
        wire::atomic_json(&rt.data.join("connection.json"), &settings)?;
        *current = settings;
        Ok(())
    })();
    result.map_err(|e| e.to_string())
}
fn configuration_edit_guard(
    rt: &Runtime,
) -> Result<std::sync::MutexGuard<'_, Option<std::process::Child>>> {
    let mut child = rt.child.lock().unwrap();
    anyhow::ensure!(
        !rt.migrating.load(Ordering::SeqCst),
        "正在迁移，请等待完成后修改连接"
    );
    anyhow::ensure!(
        !rt.starting.load(Ordering::SeqCst),
        "正在启动，请等待完成后修改连接"
    );
    anyhow::ensure!(!rt.exiting.load(Ordering::SeqCst), "应用正在退出");
    anyhow::ensure!(
        !child
            .as_mut()
            .is_some_and(|c| c.try_wait().ok().flatten().is_none()),
        "请先断开连接再修改配置"
    );
    Ok(child)
}
fn toml_config(path: &str) -> Result<macrun::config::WorkerConfig> {
    Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
}

struct StartAttempt<'a> {
    starting: &'a AtomicBool,
    app: Option<tauri::AppHandle>,
}

impl<'a> StartAttempt<'a> {
    fn begin(starting: &'a AtomicBool) -> std::result::Result<Self, String> {
        starting
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "执行器正在启动，请先完成系统授权并等待结果".to_owned())?;
        Ok(Self {
            starting,
            app: None,
        })
    }

    fn announce(mut self, app: tauri::AppHandle) -> Self {
        let _ = app.emit("worker-starting", true);
        self.app = Some(app);
        self
    }
}

impl Drop for StartAttempt<'_> {
    fn drop(&mut self) {
        self.starting.store(false, Ordering::SeqCst);
        if let Some(app) = &self.app {
            let _ = app.emit("worker-starting", false);
        }
    }
}

async fn read_credential_in_background(
    read: impl FnOnce() -> std::result::Result<Vec<u8>, String> + Send + 'static,
) -> std::result::Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(read)
        .await
        .map_err(|_| "读取连接凭据的后台任务异常，请重试连接".to_owned())?
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn configuration_edits_reject_start_migration_and_exit_before_any_file_changes() {
        let directory = tempfile::tempdir().unwrap();
        let rt = Runtime {
            data: directory.path().to_path_buf(),
            socket: directory.path().join("control.sock"),
            child: Mutex::new(None),
            snapshot: Mutex::new(Value::Null),
            settings: Mutex::new(Settings::default()),
            preferences: Mutex::new(Preferences::default()),
            exiting: AtomicBool::new(false),
            starting: AtomicBool::new(false),
            desired_running: AtomicBool::new(false),
            migrating: AtomicBool::new(false),
            window_layout: Mutex::new(WindowLayout::default()),
        };
        assert!(configuration_edit_guard(&rt).is_ok());
        for (flag, message) in [
            (&rt.starting, "正在启动"),
            (&rt.migrating, "正在迁移"),
            (&rt.exiting, "正在退出"),
        ] {
            flag.store(true, Ordering::SeqCst);
            let error = save_settings_inner(Settings::default(), &rt).unwrap_err();
            assert!(error.contains(message));
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
            flag.store(false, Ordering::SeqCst);
        }
    }

    #[test]
    fn startup_attempt_blocks_duplicates_and_releases_after_failure() {
        let starting = AtomicBool::new(false);
        let attempt = StartAttempt::begin(&starting).unwrap();
        assert!(StartAttempt::begin(&starting).is_err());
        drop(attempt);
        let failure: std::result::Result<(), String> = (|| {
            let _attempt = StartAttempt::begin(&starting)?;
            Err("test failure".into())
        })();
        assert!(failure.is_err());
        assert!(!starting.load(Ordering::SeqCst));
        assert!(StartAttempt::begin(&starting).is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn credential_read_runs_off_the_calling_thread_and_preserves_errors() {
        let caller = std::thread::current().id();
        let result = read_credential_in_background(move || {
            assert_ne!(std::thread::current().id(), caller);
            Ok(vec![1, 2, 3])
        })
        .await
        .unwrap();
        assert_eq!(result, vec![1, 2, 3]);
        let error = read_credential_in_background(|| Err("test read refused".into()))
            .await
            .unwrap_err();
        assert_eq!(error, "test read refused");
    }
}

#[tauri::command]
async fn start_worker(
    app: tauri::AppHandle,
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<(), String> {
    let _attempt = StartAttempt::begin(&rt.starting)?.announce(app.clone());
    let cfg = rt.settings.lock().unwrap().clone();
    {
        let mut child = rt.child.lock().unwrap();
        if rt.migrating.load(Ordering::SeqCst) {
            return Err("正在迁移旧执行器，请稍后连接".into());
        }
        if rt.exiting.load(Ordering::SeqCst) {
            return Err("应用正在退出，未启动执行器".into());
        }
        if let Some(child) = child.as_mut()
            && child.try_wait().map_err(|e| e.to_string())?.is_none()
        {
            return Err("执行器已经运行".into());
        }
    }
    if !cfg.certificate_fingerprint.is_empty() {
        let bytes = std::fs::read(&cfg.cert).map_err(|e| e.to_string())?;
        if blake3::hash(&bytes).to_hex().as_str() != cfg.certificate_fingerprint {
            return Err("固定证书已变化，请重新配对或导入".into());
        }
    }
    let address = tokio::net::lookup_host(&cfg.server)
        .await
        .map_err(|e| format!("服务器地址无效：{e}"))?
        .find(|a| a.is_ipv4())
        .ok_or("当前版本需要 IPv4 地址")?;
    if legacy_running() {
        return Err(
            "检测到旧 LaunchAgent。请先完成迁移，避免重复运行；本版本不会自动停止旧服务。".into(),
        );
    }
    // Keychain may wait for a system authorization dialog. Do not hold the
    // lifecycle lock or block an async executor thread during that wait.
    let token = if cfg.keychain_account.is_empty() {
        None
    } else {
        let account = cfg.keychain_account.clone();
        Some(
            read_credential_in_background(move || {
                security_framework::passwords::get_generic_password("dev.macrun.desktop", &account)
                    .map_err(|_| "无法从钥匙串读取连接凭据，请重试连接或重新配对".to_owned())
            })
            .await?,
        )
    };
    let result: Result<()> = (|| {
        let mut slot = rt.child.lock().unwrap();
        anyhow::ensure!(
            !rt.migrating.load(Ordering::SeqCst),
            "正在迁移旧执行器，请稍后连接"
        );
        anyhow::ensure!(
            !rt.exiting.load(Ordering::SeqCst),
            "应用正在退出，未启动执行器"
        );
        // DNS and Keychain authorization can outlive a migration or connection
        // edit. Never spawn with credentials captured before that await.
        anyhow::ensure!(
            *rt.settings.lock().unwrap() == cfg,
            "连接配置已变化，请重新连接"
        );
        if let Some(c) = slot.as_mut()
            && c.try_wait()?.is_none()
        {
            anyhow::bail!("执行器已经运行");
        }
        let bundled = std::env::current_exe()?
            .parent()
            .context("missing application directory")?
            .join("macrun");
        let binary = if bundled.is_file() {
            bundled
        } else if cfg!(debug_assertions) {
            std::env::var_os("MACRUN_DESKTOP_WORKER")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/macrun")
                })
        } else {
            anyhow::bail!("bundled worker is missing");
        };
        let worker_data = rt.data.join("worker");
        if !worker_data.join("desktop-policy.json").exists() {
            wire::atomic_json(
                &worker_data.join("desktop-policy.json"),
                // Phase one favours a working setup: desktop control starts enabled
                // and every desktop tier is allowed; users tighten it locally.
                &json!({"paused":false,"desktop_enabled":true}),
            )?;
        }
        if !worker_data.join("safety.json").exists() {
            wire::atomic_json(
                &worker_data.join("safety.json"),
                &macrun::safety::Safety::default(),
            )?;
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(rt.data.join("worker.log"))?;
        let mut command = Command::new(binary);
        cfg.apply_worker_environment(&mut command);
        command
            .args([
                "worker",
                "--server",
                &address.to_string(),
                "--cert",
                &cfg.cert,
                "--token-file",
                &cfg.token_file,
                "--data",
            ])
            .arg(rt.data.join("worker"))
            .arg("--control-socket")
            .arg(&rt.socket)
            .arg("--parent-pipe")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log));
        if !cfg.backend_config.is_empty() {
            command.arg("--config").arg(&cfg.backend_config);
        }
        *rt.snapshot.lock().unwrap() = Value::Null;
        if token.is_some() {
            command.arg("--token-stdin");
        }
        let mut child = command.spawn().context("无法启动随应用分发的 worker")?;
        if let Some(token) = token {
            use std::io::Write;
            let pipe = child.stdin.as_mut().context("missing credential pipe")?;
            pipe.write_all(&(token.len() as u32).to_be_bytes())?;
            pipe.write_all(&token)?;
            pipe.flush()?;
        }
        *slot = Some(child);
        Ok(())
    })();
    result.map_err(|e| e.to_string())?;
    rt.desired_running.store(true, Ordering::SeqCst);
    Ok(())
}
#[tauri::command]
async fn pair(uri: String, rt: tauri::State<'_, Runtime>) -> std::result::Result<Value, String> {
    let result:Result<Value>=async {
        anyhow::ensure!(!rt.child.lock().unwrap().as_mut().is_some_and(|c|c.try_wait().ok().flatten().is_none()),"请先断开现有连接");
        let cert=rt.data.join(format!("paired-{}.der",uuid::Uuid::new_v4()));
        let (invite,token)=macrun::pairing::exchange(&uri,&cert).await?;
        let account=uuid::Uuid::new_v4().to_string();
        security_framework::passwords::set_generic_password("dev.macrun.desktop",&account,token.as_bytes())
            .map_err(|_|anyhow::anyhow!("钥匙串写入失败，请检查系统授权并生成新邀请"))?;
        let mut settings=rt.settings.lock().unwrap().clone();
        settings.server=invite.server;settings.cert=cert.to_string_lossy().into();
        settings.token_file="/dev/null".into();settings.keychain_account=account;
        settings.certificate_fingerprint=invite.fingerprint.clone();
        wire::atomic_json(&rt.data.join("connection.json"),&settings)?;
        *rt.settings.lock().unwrap()=settings;
        Ok(json!({"fingerprint":invite.fingerprint,"protocol":invite.protocol,"credentials":"keychain"}))
    }.await;
    result.map_err(|e| e.to_string())
}
#[tauri::command]
async fn control(
    action: String,
    args: Value,
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<Value, String> {
    if ![
        "pause",
        "desktop",
        "stop_all",
        "cancel",
        "tools",
        "snapshot",
        "safety",
        "approve",
        "yield",
        "restart_backend",
        "task_detail",
        "task_list",
        "prune",
        "observe",
        "self_test",
        "revoke_rule",
    ]
    .contains(&action.as_str())
    {
        return Err("不支持的控制操作".into());
    }
    local::request(&rt.socket, &action, args)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn stop_worker(rt: tauri::State<'_, Runtime>) -> std::result::Result<(), String> {
    rt.desired_running.store(false, Ordering::SeqCst);
    local::request(&rt.socket, "shutdown", json!({}))
        .await
        .map_err(|e| e.to_string())?;
    // Retain the child until it is reaped; a new start checks try_wait.
    Ok(())
}
#[tauri::command]
async fn exit_app(
    force: bool,
    app: tauri::AppHandle,
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<(), String> {
    if rt.migrating.load(Ordering::SeqCst) {
        return Err("正在迁移旧执行器，请等待完成后退出".into());
    }
    let running = rt
        .child
        .lock()
        .unwrap()
        .as_mut()
        .is_some_and(|c| c.try_wait().ok().flatten().is_none());
    if running {
        let state = local::request(&rt.socket, "snapshot", json!({})).await;
        if !force
            && state
                .as_ref()
                .map_or(true, |s| s["active_count"].as_u64().unwrap_or(0) > 0)
        {
            return Err("仍有任务运行或状态未知，请确认退出。".into());
        }
        rt.desired_running.store(false, Ordering::SeqCst);
        if let Err(e) = local::request(&rt.socket, "shutdown", json!({})).await {
            // Closing the owned stdin pipe also requests a graceful worker shutdown.
            if let Some(c) = rt.child.lock().unwrap().as_mut() {
                c.stdin.take();
            }
            return Err(format!("已请求执行器停止，请稍后再退出：{e}"));
        }
    }
    let _lifecycle = rt.child.lock().unwrap();
    if rt.migrating.load(Ordering::SeqCst) {
        return Err("正在迁移旧执行器，请等待完成后退出".into());
    }
    rt.desired_running.store(false, Ordering::SeqCst);
    rt.exiting.store(true, Ordering::SeqCst);
    app.exit(0);
    Ok(())
}
#[tauri::command]
fn open_log(task_id: Option<String>, rt: tauri::State<Runtime>) -> std::result::Result<(), String> {
    let p = if let Some(id) = task_id {
        let id = uuid::Uuid::parse_str(&id).map_err(|_| "无效任务编号")?;
        rt.data
            .join("worker/tasks")
            .join(id.to_string())
            .join("output.log")
    } else {
        rt.data.join("worker.log")
    };
    if !p.is_file() {
        return Err("当前任务没有文本日志".into());
    }
    Command::new("open")
        .arg(p)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
fn autostart(enabled: bool, app: tauri::AppHandle) -> std::result::Result<(), String> {
    if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|e| e.to_string())
}
#[tauri::command]
fn open_main(app: tauri::AppHandle, route: String) {
    show(&app, Some(&route));
    if let Some(w) = app.get_webview_window("tray") {
        let _ = w.hide();
    }
}
#[tauri::command]
fn request_quit(app: tauri::AppHandle) {
    request_exit(&app);
}
fn request_exit(app: &tauri::AppHandle) {
    if app.state::<Runtime>().migrating.load(Ordering::SeqCst) {
        show(app, None);
        let _ = app.emit("control-error", "正在迁移旧执行器，请等待完成后退出");
        return;
    }
    show(app, None);
    let _ = app.emit("exit-requested", ());
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show(app, None)
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let rt = app.state::<Runtime>();
                            if let Err(e) = local::request(&rt.socket, "stop_all", json!({})).await
                            {
                                let _ = app.emit("control-error", e.to_string());
                            }
                        });
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            app_state,
            set_main_mode,
            resize_panel,
            save_settings,
            save_preferences,
            pair,
            native::input_status,
            native::notify_task,
            native::permissions,
            native::open_permission,
            native::open_workspace,
            native::backend_config,
            native::save_backends,
            native::diagnostics,
            native::connection_check,
            native::migrate_legacy,
            start_worker,
            control,
            stop_worker,
            exit_app,
            open_log,
            autostart,
            open_main,
            request_quit
        ])
        .setup(|app| {
            let data = if cfg!(debug_assertions) {
                std::env::var_os("MACRUN_DESKTOP_DATA")
                    .map(PathBuf::from)
                    .unwrap_or(app.path().app_data_dir()?)
            } else {
                app.path().app_data_dir()?
            };
            std::fs::create_dir_all(&data)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700))?;
            let settings = std::fs::read(data.join("connection.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            // macOS Unix socket paths are short; this dedicated 0700 directory is deterministic per data root.
            let hash = blake3::hash(data.to_string_lossy().as_bytes())
                .to_hex()
                .to_string();
            let socket = std::env::temp_dir()
                .join(format!("macrun-desktop-{}", &hash[..16]))
                .join("control.sock");
            let preferences = std::fs::read(data.join("preferences.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            app.manage(Runtime {
                data,
                socket,
                child: Mutex::new(None),
                snapshot: Mutex::new(Value::Null),
                settings: Mutex::new(settings),
                preferences: Mutex::new(preferences),
                exiting: AtomicBool::new(false),
                starting: AtomicBool::new(false),
                desired_running: AtomicBool::new(false),
                migrating: AtomicBool::new(false),
                window_layout: Mutex::new(WindowLayout::default()),
            });
            app.manage(Mutex::new(None::<tray::Presentation>));
            let handle = app.handle().clone();
            app.global_shortcut().register("Control+Alt+Super+Period")?;
            tauri::tray::TrayIconBuilder::with_id("macrun")
                .icon(tray_image(tray::TrayState::Offline))
                .icon_as_template(true)
                .tooltip("Macrun · 本机执行器")
                .on_tray_icon_event(|icon, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_tray(icon.app_handle(), true);
                    }
                })
                .build(app)?;
            tauri::WebviewWindowBuilder::new(
                app,
                "tray",
                tauri::WebviewUrl::App("index.html?tray=1".into()),
            )
            .title("Macrun · 菜单栏")
            .shadow(false)
            .inner_size(352., 510.)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()?;
            tauri::WebviewWindowBuilder::new(
                app,
                "overlay",
                tauri::WebviewUrl::App("index.html?overlay=1".into()),
            )
            .title("Macrun · 屏幕提示")
            .inner_size(600., 64.)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .content_protected(true)
            .build()?;
            for (i, monitor) in app.available_monitors()?.iter().enumerate() {
                let p = monitor.position();
                let size = monitor.size();
                let width = size.width as f64;
                let height = size.height as f64;
                let rects = [
                    (0., 0., width, 3.),
                    (0., height - 3., width, 3.),
                    (0., 0., 3., height),
                    (width - 3., 0., 3., height),
                ];
                for (side, (x, y, w, h)) in rects.iter().enumerate() {
                    let border = tauri::WebviewWindowBuilder::new(
                        app,
                        format!("border-{i}-{side}"),
                        tauri::WebviewUrl::App("index.html?border=1".into()),
                    )
                    .title(format!("Macrun · 屏幕边框 {}-{}", i + 1, side + 1))
                    .decorations(false)
                    .resizable(false)
                    .focusable(false)
                    .focused(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .visible(false)
                    .content_protected(true)
                    .build()?;
                    border.set_size(tauri::PhysicalSize::new(*w as u32, *h as u32))?;
                    border.set_position(tauri::PhysicalPosition::new(
                        p.x as f64 + x,
                        p.y as f64 + y,
                    ))?;
                    border.set_ignore_cursor_events(true)?;
                }
                if i == 0
                    && let Some(w) = app.get_webview_window("overlay")
                {
                    w.set_position(tauri::PhysicalPosition::new(
                        p.x as f64 + (width - 600. * monitor.scale_factor()) / 2.,
                        p.y as f64 + 32. * monitor.scale_factor(),
                    ))?;
                }
            }
            native_menu(app.handle())?;
            unsafe {
                native::macrun_monitor_start();
            }
            let monitor = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let configured = !monitor
                    .state::<Runtime>()
                    .settings
                    .lock()
                    .unwrap()
                    .server
                    .is_empty();
                if configured
                    && monitor
                        .state::<Runtime>()
                        .preferences
                        .lock()
                        .unwrap()
                        .auto_connect
                    && let Err(e) = start_worker(monitor.clone(), monitor.state::<Runtime>()).await
                {
                    let _ = monitor.emit("control-error", e);
                }
                let mut sequence = 0;
                let mut budget = supervisor::RestartBudget::default();
                let mut last_yield = 0;
                loop {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let rt = monitor.state::<Runtime>();
                    if rt.exiting.load(Ordering::SeqCst) {
                        break;
                    }
                    let current = unsafe { native::macrun_input_sequence() };
                    if current != sequence && rt.preferences.lock().unwrap().yield_input {
                        sequence = current;
                        if macrun::model::now() - last_yield > 500 {
                            let _ = local::request(&rt.socket, "yield", json!({})).await;
                            last_yield = macrun::model::now();
                        }
                    }
                    let running = rt
                        .child
                        .lock()
                        .unwrap()
                        .as_mut()
                        .is_some_and(|c| c.try_wait().ok().flatten().is_none());
                    let desired = rt.desired_running.load(Ordering::SeqCst);
                    // A user-initiated start may be waiting on Keychain. Do not
                    // consume recovery attempts or report duplicate-start errors.
                    if rt.starting.load(Ordering::SeqCst) {
                        continue;
                    }
                    if budget.tick(macrun::model::now(), running, desired) {
                        if let Err(e) =
                            start_worker(monitor.clone(), monitor.state::<Runtime>()).await
                        {
                            let _ = monitor.emit("control-error", e);
                        }
                    } else if budget.take_exhausted_notice(running, desired) {
                        let _ = monitor.emit(
                            "control-error",
                            "执行器连续退出，已暂停自动重试。请查看日志后在设置中重新连接。",
                        );
                    }
                }
            });
            tauri::async_runtime::spawn(async move {
                loop {
                    let sock = handle.state::<Runtime>().socket.clone();
                    let session: Result<()> = async {
                        let mut stream = tokio::net::UnixStream::connect(sock).await?;
                        wire::send(&mut stream, &json!({"action":"subscribe"})).await?;
                        loop {
                            let v: Value = wire::recv(&mut stream).await?;
                            *handle.state::<Runtime>().snapshot.lock().unwrap() = v.clone();
                            let active = v["tasks"].as_array().is_some_and(|tasks| {
                                tasks
                                    .iter()
                                    .any(|t| t["kind"] == "mcp.call" && t["status"] == "running")
                            });
                            let preferences = handle
                                .state::<Runtime>()
                                .preferences
                                .lock()
                                .unwrap()
                                .clone();
                            unsafe {
                                native::macrun_keep_awake(active && preferences.keep_awake);
                            }
                            let observing = v["tasks"].as_array().is_some_and(|ts| {
                                ts.iter().any(|t| {
                                    t["status"] == "running"
                                        && (t["arguments"]["local_observation"] == true
                                            || ["screenshot", "observe", "capture"].iter().any(
                                                |s| {
                                                    t["arguments"]["tool"]
                                                        .as_str()
                                                        .unwrap_or("")
                                                        .to_lowercase()
                                                        .contains(s)
                                                },
                                            ))
                                })
                            });
                            overlay_visible(
                                &handle,
                                active && preferences.show_overlay && !observing,
                            );
                            tray_status(&handle, &v);
                            let _ = handle.emit("worker-state", v);
                        }
                    }
                    .await;
                    if session.is_err() {
                        unsafe {
                            native::macrun_keep_awake(false);
                        }
                        overlay_visible(&handle, false);
                        tray_status(&handle, &Value::Null);
                        let _ = handle.emit("worker-unavailable", ());
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
            Ok(())
        })
        .on_window_event(|w, e| match e {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = w.hide();
            }
            tauri::WindowEvent::Focused(false) if w.label() == "tray" => {
                let _ = w.hide();
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("cannot build Macrun Desktop")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event
                && !app.state::<Runtime>().exiting.load(Ordering::SeqCst)
            {
                api.prevent_exit();
                request_exit(app);
            }
        });
}
