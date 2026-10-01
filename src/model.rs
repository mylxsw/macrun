use crate::{config::Project, sync::Manifest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub const PROTOCOL: u32 = 1;
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fault {
    pub code: String,
    pub message: String,
}
impl Fault {
    pub fn new(code: &str, message: impl ToString) -> Self {
        Self {
            code: code.into(),
            message: message.to_string(),
        }
    }
    pub fn exit_code(&self) -> i32 {
        match self.code.as_str() {
            "build_failed" | "test_failed" => 1,
            "invalid_config" | "artifact_not_found" => 2,
            "worker_offline" | "worker_disconnected" => 3,
            "busy" => 4,
            "sync_failed" | "source_changed" | "path_conflict" => 5,
            "timed_out" => 6,
            "connection_auth_failed" => 7,
            "no_gui_session" | "no_display" | "not_gui_agent" => 8,
            "window_not_found" | "window_required" => 9,
            "cua_unavailable"
            | "cua_refused"
            | "capture_permission_denied"
            | "observation_failed" => 10,
            "app_not_running"
            | "app_exited"
            | "app_instance_conflict"
            | "target_mismatch"
            | "stale_snapshot" => 11,
            "cancelled" => 12,
            _ => 13,
        }
    }
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Fault {}
pub type Outcome<T> = Result<T, Fault>;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub kind: String,
    pub workspace: PathBuf,
    #[serde(default)]
    pub args: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub job_id: String,
    pub request: Request,
    pub project: Project,
    pub manifest: Option<Manifest>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    Hello {
        protocol: u32,
        token: String,
        instance: String,
        status: Value,
    },
    Welcome {
        protocol: u32,
    },
    Heartbeat {
        status: Value,
    },
    Cancel {
        job_id: String,
    },
    Reject {
        error: Fault,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Need { paths: Vec<String> },
    SyncCommit { manifest: Manifest },
    Log { name: String, text: String },
    Progress { phase: String },
    Detail { value: Value },
    Artifact { path: String, data: String },
    Done { error: Option<Fault> },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHeader {
    pub job_id: String,
    pub path: String,
    pub size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Accepted { job_id: String, directory: PathBuf },
    Log { name: String, text: String },
    Status { value: Value },
    Done { result: Value, directory: PathBuf },
    Error { error: Fault },
}
pub fn initial_result(task: &Task) -> Value {
    json!({"job_id":task.job_id,"kind":task.request.kind,"status":"accepted","exit_code":0,
        "started_at":now(),"project":task.project,"action_status":"not_sent",
        "observation_status":"not_attempted","effect_verified":false,"delivery":"unknown",
        "phases_ms":{},"logs":[],"artifacts":[]})
}
pub fn finish(result: &mut Value, error: Option<Fault>) {
    result["ended_at"] = json!(now());
    result["status"] = json!(match error.as_ref().map(|e| e.code.as_str()) {
        None => "succeeded",
        Some("cancelled") => "cancelled",
        Some("timed_out") => "timed_out",
        _ => "failed",
    });
    result["exit_code"] = json!(error.as_ref().map_or(0, Fault::exit_code));
    if let Some(e) = error {
        result["error"] = json!(e);
    }
}
