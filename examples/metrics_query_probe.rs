//! Synthetic retained-index benchmark; no worker, network or user data is accessed.
use macrun::{
    metrics::{self, Metrics, Profile, Store},
    model, wire,
};
use rusqlite::{Connection, params};
use serde_json::json;
use std::time::Instant;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let _store = Store::open(dir.path().into())?;
    let mut db = Connection::open(dir.path().join("metrics.sqlite"))?;
    let now = model::now();
    let mut m = Metrics {
        wall_ms: 100.0,
        complete: true,
        ..Default::default()
    };
    m.bytes.insert("payload".into(), 4096);
    let tx = db.transaction()?;
    {
        let mut insert = tx.prepare("INSERT INTO operations VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
        for i in 0..100_000u128 {
            let record = metrics::Record {
                task_id: uuid::Uuid::from_u128(i + 1).to_string(),
                kind: "sync".into(),
                status: "succeeded".into(),
                started_at: now - i as u64,
                ended_at: Some(now),
                metrics: Some(m.clone()),
                server_metrics: None,
                version: "synthetic".into(),
            };
            insert.execute(params![
                record.task_id,
                i64::try_from(record.started_at)?,
                record.ended_at.map(i64::try_from).transpose()?,
                record.kind,
                record.status,
                Option::<String>::None,
                100.0,
                serde_json::to_string(&record)?
            ])?;
        }
    }
    tx.commit()?;
    drop(db);
    let id = uuid::Uuid::from_u128(1).to_string();
    wire::atomic_json(
        &dir.path().join("tasks").join(&id).join("result.json"),
        &json!({"task_id":id,"kind":"sync","status":"succeeded","started_at":now,"ended_at":now,"metrics":m}),
    )?;
    let p = vec![Profile {
        id: "primary".into(),
        name: "synthetic".into(),
        data: dir.path().into(),
    }];
    let mut dashboard = vec![];
    let mut details = vec![];
    for _ in 0..8 {
        let start = Instant::now();
        let v = metrics::query(p.clone(), "metrics_dashboard", json!({})).await?;
        anyhow::ensure!(v["total"] == 100_000, "incorrect aggregate");
        dashboard.push(start.elapsed().as_secs_f64() * 1000.0);
        let start = Instant::now();
        metrics::query(
            p.clone(),
            "metrics_detail",
            json!({"connection_id":"primary","task_id":id}),
        )
        .await?;
        details.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "{}",
        json!({"records":100000,"samples":8,"profile":if cfg!(debug_assertions){"debug"}else{"release"},"dashboard_ms":dashboard,"dashboard_p95_ms":metrics::percentile(&mut dashboard.clone(),0.95),"detail_ms":details,"detail_p95_ms":metrics::percentile(&mut details.clone(),0.95)})
    );
    Ok(())
}
