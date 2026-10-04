#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
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

#[derive(Clone, Default, Serialize, Deserialize)]
struct Settings {
    server: String,
    cert: String,
    token_file: String,
    backend_config: String,
}
struct Runtime {
    data: PathBuf,
    socket: PathBuf,
    child: Mutex<Option<Child>>,
    snapshot: Mutex<Value>,
    settings: Mutex<Settings>,
    exiting: AtomicBool,
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
#[tauri::command]
fn app_state(app: tauri::AppHandle, rt: tauri::State<Runtime>) -> Value {
    let mut child = rt.child.lock().unwrap();
    let running = child
        .as_mut()
        .is_some_and(|c| c.try_wait().ok().flatten().is_none());
    json!({"worker_running":running,"snapshot":*rt.snapshot.lock().unwrap(),"settings":*rt.settings.lock().unwrap(),"data_dir":rt.data,"legacy_running":legacy_running(),"autostart":app.autolaunch().is_enabled().unwrap_or(false),"platform":std::env::consts::OS})
}
#[tauri::command]
fn save_settings(settings: Settings, rt: tauri::State<Runtime>) -> std::result::Result<(), String> {
    let result: Result<()> = (|| {
        anyhow::ensure!(!settings.server.trim().is_empty(), "请填写服务器地址");
        anyhow::ensure!(
            PathBuf::from(&settings.cert).is_file(),
            "服务器证书文件不存在"
        );
        anyhow::ensure!(
            PathBuf::from(&settings.token_file).is_file(),
            "令牌文件不存在"
        );
        if !settings.backend_config.is_empty() {
            let _: macrun::config::WorkerConfig = toml_config(&settings.backend_config)?;
        }
        wire::atomic_json(&rt.data.join("connection.json"), &settings)?;
        *rt.settings.lock().unwrap() = settings;
        Ok(())
    })();
    result.map_err(|e| e.to_string())
}
fn toml_config(path: &str) -> Result<macrun::config::WorkerConfig> {
    Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
}
#[tauri::command]
async fn start_worker(
    app: tauri::AppHandle,
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<(), String> {
    let cfg = rt.settings.lock().unwrap().clone();
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
    let result: Result<()> = (|| {
        let mut slot = rt.child.lock().unwrap();
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
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(rt.data.join("worker.log"))?;
        let mut command = Command::new(binary);
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
        *slot = Some(command.spawn().context("无法启动随应用分发的 worker")?);
        Ok(())
    })();
    result.map_err(|e| e.to_string())?;
    let _ = app.emit("worker-starting", ());
    Ok(())
}
#[tauri::command]
async fn control(
    action: String,
    args: Value,
    rt: tauri::State<'_, Runtime>,
) -> std::result::Result<Value, String> {
    if ![
        "pause", "desktop", "stop_all", "cancel", "tools", "snapshot",
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
        if let Err(e) = local::request(&rt.socket, "shutdown", json!({})).await {
            // Closing the owned stdin pipe also requests a graceful worker shutdown.
            if let Some(c) = rt.child.lock().unwrap().as_mut() {
                c.stdin.take();
            }
            return Err(format!("已请求执行器停止，请稍后再退出：{e}"));
        }
    }
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
    show(app, None);
    let _ = app.emit("exit-requested", ());
}
fn main() {
    tauri::Builder::default()
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
            save_settings,
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
            app.manage(Runtime {
                data,
                socket,
                child: Mutex::new(None),
                snapshot: Mutex::new(Value::Null),
                settings: Mutex::new(settings),
                exiting: AtomicBool::new(false),
            });
            let handle = app.handle().clone();
            app.global_shortcut().register("Control+Alt+Super+Period")?;
            let mut rgba = vec![0u8; 22 * 22 * 4];
            for y in 5..17 {
                for x in 4..18 {
                    if (x < 11 && (y == x + 1 || y == 21 - x)) || (y == 16 && x > 11) {
                        let i = (y * 22 + x) * 4;
                        rgba[i + 3] = 255;
                    }
                }
            }
            tauri::tray::TrayIconBuilder::with_id("macrun")
                .icon(tauri::image::Image::new_owned(rgba, 22, 22))
                .icon_as_template(true)
                .tooltip("Macrun · 本机执行器")
                .on_tray_icon_event(|icon, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button_state: tauri::tray::MouseButtonState::Up,
                        position,
                        ..
                    } = event
                        && let Some(w) = icon.app_handle().get_webview_window("tray")
                    {
                        if w.is_visible().unwrap_or(false) {
                            let _ = w.hide();
                        } else {
                            let scale = w.scale_factor().unwrap_or(1.0);
                            let _ = w.set_position(tauri::PhysicalPosition::new(
                                position.x - 340. * scale,
                                position.y + 14. * scale,
                            ));
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;
            tauri::WebviewWindowBuilder::new(
                app,
                "tray",
                tauri::WebviewUrl::App("index.html?tray=1".into()),
            )
            .title("Macrun 快捷面板")
            .inner_size(360., 560.)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()?;
            tauri::async_runtime::spawn(async move {
                loop {
                    let sock = handle.state::<Runtime>().socket.clone();
                    let session: Result<()> = async {
                        let mut stream = tokio::net::UnixStream::connect(sock).await?;
                        wire::send(&mut stream, &json!({"action":"subscribe"})).await?;
                        loop {
                            let v: Value = wire::recv(&mut stream).await?;
                            *handle.state::<Runtime>().snapshot.lock().unwrap() = v.clone();
                            let _ = handle.emit("worker-state", v);
                        }
                    }
                    .await;
                    if session.is_err() {
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
