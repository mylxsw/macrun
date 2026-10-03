//! Small stderr-only operational log. Never pass request payloads here.
use crate::model::Reply;
use serde_json::{Value, json};
use std::time::Instant;

pub fn event(component: &str, event: &str, fields: Value) {
    let mut record = fields;
    record["time"] = json!(timestamp());
    record["component"] = json!(component);
    record["event"] = json!(event);
    eprintln!("{record}");
}
fn timestamp() -> String {
    let millis = crate::model::now();
    let seconds = (millis / 1000) as libc::time_t;
    // gmtime_r writes only to our local buffer and uses no shared tm storage.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::gmtime_r(&seconds, &mut tm) }.is_null() {
        return millis.to_string();
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        millis % 1000
    )
}

pub struct Operation {
    component: &'static str,
    start: Instant,
    fields: Value,
}
impl Operation {
    pub fn new(component: &'static str, operation: &str, request_id: &str) -> Self {
        Self {
            component,
            start: Instant::now(),
            fields: json!({
                "operation": operation.chars().take(80).collect::<String>(),
                "request_id": request_id, "status": "interrupted"
            }),
        }
    }
    pub fn context(&mut self, args: &Value) {
        for key in ["task_id", "job_id", "request_id"] {
            if let Some(id) = args[key]
                .as_str()
                .filter(|s| uuid::Uuid::parse_str(s).is_ok())
            {
                let key = if key == "request_id" { "task_id" } else { key };
                self.fields[key] = json!(id);
            }
        }
        if self.fields["operation"]
            .as_str()
            .is_some_and(|s| s.starts_with("mcp."))
        {
            for key in ["server", "tool"] {
                if let Some(name) = args[key].as_str() {
                    self.fields[key] = json!(name.chars().take(80).collect::<String>());
                }
            }
        }
    }
    pub fn status(&mut self, status: &str) {
        self.fields["status"] = json!(status);
    }
    pub fn reply(&mut self, reply: &Reply) {
        match reply {
            Reply::Error { error } => {
                self.status("failed");
                self.fields["error_code"] = json!(error.code);
            }
            Reply::Accepted { job_id, .. } => {
                self.status("accepted");
                self.fields["job_id"] = json!(job_id);
            }
            Reply::Status { value } => {
                self.status("succeeded");
                for key in ["task_id", "job_id"] {
                    if let Some(v) = value.get(key) {
                        self.fields[key] = v.clone();
                    }
                }
                if let Some(v) = value.get("status") {
                    self.fields["task_status"] = v.clone();
                }
                if let Some(v) = value.get("duplicate") {
                    self.fields["duplicate"] = v.clone();
                }
            }
            _ => {}
        }
    }
    pub async fn send<W: tokio::io::AsyncWrite + Unpin>(
        &mut self,
        writer: &mut W,
        reply: &Reply,
    ) -> anyhow::Result<()> {
        crate::wire::send(writer, reply).await?;
        self.reply(reply);
        Ok(())
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        self.fields["duration_ms"] = json!(self.start.elapsed().as_millis() as u64);
        event(self.component, "operation", self.fields.clone());
    }
}
