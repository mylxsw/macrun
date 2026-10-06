use crate::localization::{interpolate as tr_format, text as tr};
use anyhow::Result;
use serde_json::{Value, json};
use std::{future::Future, time::Duration};

fn summarize(state: &Value, target: Option<&str>) -> Value {
    let connections: Vec<&Value> = match state["connections"].as_array() {
        Some(rows) => rows
            .iter()
            .filter(|row| target.is_none_or(|id| row["id"] == id))
            .map(|row| &row["connection"])
            .collect(),
        None if target.is_none_or(|id| id == "primary") && state["connection"].is_object() => {
            vec![&state["connection"]]
        }
        _ => vec![],
    };
    let ready = !connections.is_empty() && connections.iter().all(|c| c["state"] == "connected");
    let checks: Vec<Value> = [
        (tr("UDP / QUIC 可达"), "transport"), (tr("证书校验"), "certificate"),
        (tr("凭据认证"), "authentication"), (tr("协议一致"), "protocol"),
    ].into_iter().map(|(name, key)| json!({"name":name,"ok":ready && connections.iter().all(|c| c["checks"][key] == true)})).collect();
    let connected = ready && checks.iter().all(|c| c["ok"] == true);
    let error = connections
        .iter()
        .filter(|c| c["state"] != "connected")
        .filter_map(|c| c["error"].as_str())
        .collect::<Vec<_>>()
        .join("；");
    json!({"checks":checks,"phase":if connected {"connected"} else {"connecting"},"error":error,"connection_id":target})
}

// Only read readiness; never repeat the pairing exchange or replay a command.
pub async fn wait_for_connection<F, Fut>(
    mut read: F,
    target: Option<&str>,
    timeout: Duration,
    interval: Duration,
) -> Value
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let mut result = summarize(&Value::Null, target);
    loop {
        match tokio::time::timeout_at(deadline, read()).await {
            Ok(Ok(state)) => {
                result = summarize(&state, target);
                if result["phase"] == "connected" {
                    return result;
                }
            }
            Ok(Err(error)) => result["error"] = json!(error.to_string()),
            Err(_) => break,
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep_until((tokio::time::Instant::now() + interval).min(deadline)).await;
    }
    result["phase"] = json!("timed_out");
    result["error"] = json!(tr_format(
        "配对已保存，连接尚未就绪，请重试连接。{0}",
        &[result["error"].as_str().unwrap_or_default().to_string()]
    ));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connected() -> Value {
        json!({"state":"connected","checks":{"transport":true,"certificate":true,"authentication":true,"protocol":true}})
    }
    #[tokio::test]
    async fn waits_for_socket_and_handshake_without_repairing() {
        let mut reads = 0;
        let r = wait_for_connection(
            || {
                reads += 1;
                let n = reads;
                async move {
                    match n {
                        1 => anyhow::bail!("socket missing"),
                        2 => Ok(json!({"connection":{"state":"connecting"}})),
                        _ => Ok(json!({"connection":connected()})),
                    }
                }
            },
            Some("primary"),
            Duration::from_secs(1),
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(reads, 3);
        assert_eq!(r["phase"], "connected");
        assert_eq!(r["error"], "");
    }
    #[tokio::test]
    async fn unreachable_and_hung_reads_have_a_deadline() {
        let r = wait_for_connection(
            || async {
                Ok(json!({"connection":{"state":"disconnected","error":"bad certificate"}}))
            },
            None,
            Duration::from_millis(5),
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(r["phase"], "timed_out");
        assert!(r["error"].as_str().unwrap().contains("bad certificate"));
        let r = wait_for_connection(
            std::future::pending,
            None,
            Duration::from_millis(5),
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(r["phase"], "timed_out");
    }
    #[test]
    fn checks_only_the_paired_connection_and_never_passes_empty_rows() {
        let state = json!({"connections":[{"id":"primary","connection":{"state":"disconnected","error":"offline"}},{"id":"new","connection":connected()}]});
        assert_eq!(summarize(&state, Some("new"))["phase"], "connected");
        for target in [None, Some("missing"), Some("primary")] {
            assert_eq!(summarize(&state, target)["phase"], "connecting");
        }
        assert_eq!(
            summarize(&json!({"connections":[]}), None)["phase"],
            "connecting"
        );
        assert_eq!(
            summarize(&json!({"connection":connected()}), Some("missing"))["phase"],
            "connecting"
        );
    }
}
