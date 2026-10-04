use macrun::{
    config::WorkerConfig,
    engine::Engine,
    safety::{AllowRule, DesktopTiers, Safety, classify_tool, program},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

async fn status(e: &Arc<Engine>, id: &str) -> Value {
    e.handle("task.get", json!({"task_id":id})).await.unwrap()
}
async fn wait_status(e: &Arc<Engine>, id: &str, want: &str) -> Value {
    for _ in 0..150 {
        let v = status(e, id).await;
        if v["status"] == want {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("task {id} never reached {want}: {}", status(e, id).await)
}
fn fixture_engine(dir: &std::path::Path) -> Arc<Engine> {
    let mut config = WorkerConfig::default();
    config.mcp.insert(
        "fixture".into(),
        macrun::config::Backend {
            command: "python3".into(),
            args: vec![format!(
                "{}/tests/fixtures/mcp.py",
                env!("CARGO_MANIFEST_DIR")
            )],
            env: [("MACRUN_TEST_TOOLS_TIERS".into(), "1".into())].into(),
            cwd: None,
        },
    );
    Engine::open(dir.into(), config).unwrap()
}
async fn confirm_all(e: &Arc<Engine>, desktop: DesktopTiers) {
    e.set_safety(Safety {
        approval: "all".into(),
        desktop,
        ..Default::default()
    })
    .await
    .unwrap();
}

#[test]
fn defaults_are_permissive_for_phase_one() {
    let s = Safety::default();
    assert_eq!(s.approval, "direct");
    for tier in macrun::safety::TIERS {
        assert_eq!(s.desktop.policy(tier), "allow");
    }
    // Older safety.json files without desktop tiers keep working.
    let old: Safety = serde_json::from_value(json!({"approval":"risk"})).unwrap();
    assert_eq!(old.desktop, DesktopTiers::default());
    let mut bad = Safety {
        desktop: DesktopTiers {
            high: "sometimes".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(bad.validate().is_err());
}

#[test]
fn tools_are_classified_from_declared_metadata() {
    assert_eq!(
        classify_tool(
            &json!({"name":"shot","annotations":{"readOnlyHint":true},"risk":{"class":"r3"}})
        ),
        "observe"
    );
    assert_eq!(
        classify_tool(
            &json!({"name":"kill_app","annotations":{"destructiveHint":true},"risk":{"class":"r3"}})
        ),
        "high"
    );
    assert_eq!(
        classify_tool(
            &json!({"name":"click","annotations":{"destructiveHint":true},"risk":{"class":"r1"}})
        ),
        "control"
    );
    assert_eq!(classify_tool(&json!({"name":"bare"})), "control");
}

#[test]
fn rules_match_program_directory_and_expiry() {
    let args = json!({"command":"/usr/bin/git status","cwd":"/work/app"});
    assert_eq!(program("/usr/bin/git status"), "git");
    let similar = AllowRule::from_request("exec.start", "similar", &args, None, 1_000).unwrap();
    assert!(similar.matches(
        "exec.start",
        &json!({"command":"git log","cwd":"/work/app/sub"}),
        None,
        2_000
    ));
    assert!(!similar.matches(
        "exec.start",
        &json!({"command":"rm -rf x","cwd":"/work/app"}),
        None,
        2_000
    ));
    assert!(!similar.matches(
        "exec.start",
        &json!({"command":"git log","cwd":"/work/other"}),
        None,
        2_000
    ));
    assert!(!similar.matches(
        "exec.start",
        &json!({"command":"git log","cwd":"/work/app/../other"}),
        None,
        2_000
    ));
    assert!(!similar.matches(
        "exec.start",
        &json!({"command":"git log","cwd":"/work/app"}),
        None,
        1_000 + 15 * 60 * 1000
    ));
    assert!(!similar.matches("mcp.call", &json!({"server":"x","tool":"git"}), None, 2_000));

    let session = AllowRule::from_request("exec.start", "session", &args, None, 1_000).unwrap();
    assert!(session.expires_at.is_none());
    assert!(session.matches(
        "exec.start",
        &json!({"command":"make test","cwd":"/work/app"}),
        None,
        u64::MAX - 1
    ));

    let call = json!({"server":"computer","tool":"click"});
    let tool = AllowRule::from_request("mcp.call", "similar", &call, Some("control"), 0).unwrap();
    assert!(tool.matches("mcp.call", &call, Some("control"), 1));
    assert!(!tool.matches(
        "mcp.call",
        &json!({"server":"computer","tool":"drag"}),
        Some("control"),
        1
    ));
    let tier = AllowRule::from_request("mcp.call", "session", &call, Some("control"), 0).unwrap();
    assert!(tier.matches(
        "mcp.call",
        &json!({"server":"computer","tool":"drag"}),
        Some("control"),
        1
    ));
    assert!(!tier.matches(
        "mcp.call",
        &json!({"server":"computer","tool":"kill_app"}),
        Some("high"),
        1
    ));

    assert!(AllowRule::from_request("exec.start", "forever", &args, None, 0).is_err());
    assert!(
        AllowRule::from_request(
            "exec.start",
            "session",
            &json!({"command":"ls","cwd":"relative"}),
            None,
            0
        )
        .is_err()
    );
    assert!(AllowRule::from_request("file.write", "session", &args, None, 0).is_err());
}

#[tokio::test]
async fn rejection_reports_its_own_code() {
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().into(), WorkerConfig::default()).unwrap();
    confirm_all(&e, DesktopTiers::default()).await;
    let v = e
        .handle("exec.start", json!({"cwd":d.path(),"command":"touch ran"}))
        .await
        .unwrap();
    let id = v["task_id"].as_str().unwrap();
    wait_status(&e, id, "awaiting_approval").await;
    assert!(e.approve_scoped(id, false, "everything").await.is_err());
    e.approve_scoped(id, false, "similar").await.unwrap();
    let v = wait_status(&e, id, "denied").await;
    assert_eq!(v["error"]["code"], "approval_rejected");
    // A rejection never grants a rule.
    assert!(e.allow_rules().await.is_empty());
    assert!(!d.path().join("ran").exists());
}

#[tokio::test(start_paused = true)]
async fn expiry_reports_its_own_code() {
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().into(), WorkerConfig::default()).unwrap();
    confirm_all(&e, DesktopTiers::default()).await;
    let v = e
        .handle("exec.start", json!({"cwd":d.path(),"command":"touch ran"}))
        .await
        .unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::task::yield_now().await;
    let v = status(&e, v["task_id"].as_str().unwrap()).await;
    assert_eq!(v["status"], "denied");
    assert_eq!(v["error"]["code"], "approval_expired");
    assert!(v["error"]["message"].as_str().unwrap().contains("expired"));
}

#[tokio::test]
async fn similar_rule_skips_later_prompts_until_revoked() {
    let d = tempfile::tempdir().unwrap();
    let e = Engine::open(d.path().into(), WorkerConfig::default()).unwrap();
    confirm_all(&e, DesktopTiers::default()).await;
    let first = e
        .handle("exec.start", json!({"cwd":d.path(),"command":"touch one"}))
        .await
        .unwrap();
    let first = first["task_id"].as_str().unwrap();
    wait_status(&e, first, "awaiting_approval").await;
    let granted = e.approve_scoped(first, true, "similar").await.unwrap();
    wait_status(&e, first, "succeeded").await;
    let rule_id = granted["rule_id"].as_str().unwrap().to_owned();
    assert_eq!(e.allow_rules().await.len(), 1);
    assert_eq!(
        e.local_snapshot().await.unwrap()["allow_rules"][0]["program"],
        "touch"
    );

    let second = e
        .handle("exec.start", json!({"cwd":d.path(),"command":"touch two"}))
        .await
        .unwrap();
    let v = wait_status(&e, second["task_id"].as_str().unwrap(), "succeeded").await;
    assert_eq!(v["approved_by_rule"], rule_id.as_str());

    let other = e
        .handle(
            "exec.start",
            json!({"cwd":d.path(),"command":"mkdir three"}),
        )
        .await
        .unwrap();
    let other = other["task_id"].as_str().unwrap();
    wait_status(&e, other, "awaiting_approval").await;
    e.approve(other, false).await.unwrap();

    assert_eq!(e.revoke_rule(&rule_id).await.unwrap()["removed"], true);
    assert_eq!(e.revoke_rule(&rule_id).await.unwrap()["removed"], false);
    let again = e
        .handle("exec.start", json!({"cwd":d.path(),"command":"touch four"}))
        .await
        .unwrap();
    let again = again["task_id"].as_str().unwrap();
    wait_status(&e, again, "awaiting_approval").await;
    e.approve(again, false).await.unwrap();
    assert!(!d.path().join("four").exists());
}

#[tokio::test]
async fn desktop_tiers_allow_confirm_and_deny() {
    let d = tempfile::tempdir().unwrap();
    let e = fixture_engine(d.path());
    let tools = e
        .handle("mcp.tools", json!({"server":"fixture"}))
        .await
        .unwrap();
    let session = tools["session"].as_str().unwrap().to_owned();
    let tiers = e.local_snapshot().await.unwrap()["backends"][0]["tiers"].clone();
    assert_eq!(
        tiers,
        json!({"observe":"observe","click":"control","kill":"high"})
    );
    let call = |tool: &str| json!({"server":"fixture","tool":tool,"session":session});

    // Default: every tier runs without a prompt.
    let v = e.handle("mcp.call", call("observe")).await.unwrap();
    let v = wait_status(&e, v["task_id"].as_str().unwrap(), "succeeded").await;
    assert_eq!(v["desktop_tier"], "observe");

    e.set_safety(Safety {
        desktop: DesktopTiers {
            observe: "allow".into(),
            control: "confirm".into(),
            high: "deny".into(),
        },
        ..Default::default()
    })
    .await
    .unwrap();
    let err = e.handle("mcp.call", call("kill")).await.unwrap_err();
    assert_eq!(
        err.downcast_ref::<macrun::model::Fault>().unwrap().code,
        "desktop_denied"
    );
    let v = e.handle("mcp.call", call("click")).await.unwrap();
    let id = v["task_id"].as_str().unwrap();
    let waiting = wait_status(&e, id, "awaiting_approval").await;
    assert_eq!(waiting["desktop_tier"], "control");
    e.approve_scoped(id, true, "session").await.unwrap();
    wait_status(&e, id, "succeeded").await;
    // The session rule covers the whole control tier on this backend.
    let v = e.handle("mcp.call", call("click")).await.unwrap();
    wait_status(&e, v["task_id"].as_str().unwrap(), "succeeded").await;
    // The local app's own observation never waits for itself.
    e.set_safety(Safety {
        desktop: DesktopTiers {
            observe: "confirm".into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .await
    .unwrap();
    let v = e.observe_locally(call("observe")).await.unwrap();
    let v = wait_status(&e, v["task_id"].as_str().unwrap(), "succeeded").await;
    assert_eq!(v["arguments"]["local_observation"], true);
    // A remote request cannot claim to be the local app to skip approval.
    let mut forged = call("observe");
    forged["local_observation"] = json!(true);
    assert!(e.handle("mcp.call", forged).await.is_err());
    e.shutdown().await.unwrap();
}
