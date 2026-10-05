//! Native menu panel. AppKit owns presentation; all actions use the existing worker socket.
use crate::{Runtime, request_exit, show};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    ffi::{CStr, CString, c_char},
    sync::{Mutex, OnceLock},
};
use tauri::Manager;

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static PENDING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static ERROR: OnceLock<Mutex<String>> = OnceLock::new();
unsafe extern "C" {
    fn macrun_panel_init(callback: extern "C" fn(*const c_char));
    fn macrun_panel_update(json: *const c_char);
    fn macrun_panel_show(x: f64, y: f64, width: f64, height: f64, toggle: bool);
    fn macrun_panel_hide();
}
fn key(action: &str, args: &Value) -> String {
    if action == "approve" {
        format!("approve:{}", args["task_id"].as_str().unwrap_or(""))
    } else {
        action.to_owned()
    }
}
extern "C" fn action(wire: *const c_char) {
    if wire.is_null() {
        return;
    }
    // AppKit passes a valid UTF-8 string for the duration of this callback.
    let Ok(wire) = unsafe { CStr::from_ptr(wire) }.to_str() else {
        return;
    };
    let Ok(request) = serde_json::from_str::<Value>(wire) else {
        return;
    };
    let Some(app) = APP.get() else {
        return;
    };
    let action = request["action"].as_str().unwrap_or("").to_owned();
    match action.as_str() {
        "open" => {
            hide();
            show(app, request["route"].as_str());
            return;
        }
        "quit" => {
            hide();
            request_exit(app);
            return;
        }
        "pause" | "desktop" | "stop_all" | "approve" => {}
        _ => return,
    }
    let args = request.get("args").cloned().unwrap_or_else(|| json!({}));
    let pending_key = key(&action, &args);
    if !PENDING
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .insert(pending_key.clone())
    {
        return;
    }
    ERROR.get().unwrap().lock().unwrap().clear();
    update(app, &app.state::<Runtime>().snapshot.lock().unwrap());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let socket = app.state::<Runtime>().socket.clone();
        let result = macrun::local::request(&socket, &action, args).await;
        if let Err(error) = result {
            *ERROR.get().unwrap().lock().unwrap() = error.to_string();
        } else if let Ok(snapshot) = macrun::local::request(&socket, "snapshot", json!({})).await {
            *app.state::<Runtime>().snapshot.lock().unwrap() = snapshot;
        }
        PENDING.get().unwrap().lock().unwrap().remove(&pending_key);
        update(&app, &app.state::<Runtime>().snapshot.lock().unwrap());
    });
}
pub fn init(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());
    let _ = PENDING.set(Mutex::new(HashSet::new()));
    let _ = ERROR.set(Mutex::new(String::new()));
    unsafe { macrun_panel_init(action) };
    update(app, &Value::Null);
}
pub fn update(app: &tauri::AppHandle, snapshot: &Value) {
    let pending = PENDING
        .get()
        .map(|p| p.lock().unwrap().iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let error = ERROR
        .get()
        .map(|p| p.lock().unwrap().clone())
        .unwrap_or_default();
    let model = presentation(snapshot, pending, error);
    let Ok(wire) = CString::new(model.to_string()) else {
        return;
    };
    let _ = app.run_on_main_thread(move || unsafe { macrun_panel_update(wire.as_ptr()) });
}
fn presentation(v: &Value, pending: Vec<String>, error: String) -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    crate::panel_model::presentation(v, pending, error, now)
}
pub fn show_panel(app: &tauri::AppHandle, toggle: bool) {
    update(app, &app.state::<Runtime>().snapshot.lock().unwrap());
    let Some(icon) = app.tray_by_id("macrun") else {
        return;
    };
    let Ok(Some(rect)) = icon.rect() else {
        return;
    };
    let pos = rect.position.to_physical::<f64>(1.0);
    let size = rect.size.to_physical::<f64>(1.0);
    unsafe { macrun_panel_show(pos.x, pos.y, size.width, size.height, toggle) };
}
pub fn hide() {
    unsafe { macrun_panel_hide() };
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_panel_keeps_confirmed_policy_while_a_request_is_pending() {
        let v = json!({"connection":{"state":"connected"},"policy":{"paused":true,"desktop_enabled":false},"tasks":[{"task_id":"approval","status":"awaiting_approval","started_at":0,"arguments":{"command":"echo waiting","cwd":"/w/app"}}]});
        let panel = presentation(&v, vec!["pause".into()], "".into());
        assert_eq!(panel["pause"], false);
        assert_eq!(panel["desktop"], false);
        assert_eq!(panel["approval"]["id"], "approval");
        assert_eq!(panel["pending"][0], "pause");
        assert_eq!(
            presentation(&Value::Null, vec![], "offline".into())["available"],
            false
        );
    }
}
