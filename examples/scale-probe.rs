//! Reproducible filesystem/history scale probe; all generated data is disposable.
use macrun::{history::TaskHistory, model::id, sync};
use serde_json::json;
use std::{fs, time::Instant};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    let data = temp.path().join("data");
    fs::create_dir(&source)?;
    fs::create_dir_all(data.join("tasks"))?;
    let mut rows = vec![];
    let mut previous = 0;
    for count in [1000, 10_000, 100_000] {
        for i in previous..count {
            fs::write(source.join(format!("{i:06}")), "small fixture\n")?;
            let ident = id();
            let directory = data.join("tasks").join(&ident);
            fs::create_dir(&directory)?;
            fs::write(
                directory.join("result.json"),
                serde_json::to_vec(
                    &json!({"task_id":ident,"kind":"exec.start","status":"succeeded","started_at":i+1,"ended_at":i+2,"arguments":{"command":"true"}}),
                )?,
            )?;
        }
        let t = Instant::now();
        let initial = sync::scan_with(&source, &[], false)?;
        let cold_scan = t.elapsed().as_secs_f64() * 1000.;
        let t = Instant::now();
        assert_eq!(sync::scan_with(&source, &[], false)?, initial);
        let warm_scan = t.elapsed().as_secs_f64() * 1000.;
        fs::write(source.join("000000"), "changed file\n")?;
        let t = Instant::now();
        let changed = sync::scan_with(&source, &[], false)?;
        let edit_scan = t.elapsed().as_secs_f64() * 1000.;
        assert_ne!(initial.entries["000000"], changed.entries["000000"]);
        let history = TaskHistory::new(data.clone());
        let t = Instant::now();
        assert_eq!(history.shared_records().await?.len(), count);
        let cold_history = t.elapsed().as_secs_f64() * 1000.;
        let mut warm = vec![];
        let mut pages = vec![];
        for _ in 0..20 {
            let t = Instant::now();
            assert_eq!(history.shared_records().await?.len(), count);
            warm.push(t.elapsed().as_secs_f64() * 1000.);
            let t = Instant::now();
            let page = history.page(json!({"limit":50})).await?;
            assert_eq!(page["total"], count);
            pages.push(t.elapsed().as_secs_f64() * 1000.);
        }
        history.checkpoint().await?;
        drop(history);
        let history = TaskHistory::new(data.clone());
        let t = Instant::now();
        assert_eq!(history.shared_records().await?.len(), count);
        let restart = t.elapsed().as_secs_f64() * 1000.;
        rows.push(json!({"entries":count,"scan_ms":{"cold":cold_scan,"warm":warm_scan,"one_edit":edit_scan},"history_ms":{"cold":cold_history,"restart_reconcile":restart,"shared_20":warm,"page_20":pages}}));
        previous = count;
        // Ensure the next size's edit changes content too.
        fs::write(source.join("000000"), "small fixture\n")?;
    }
    println!("{}", serde_json::to_string_pretty(&rows)?);
    Ok(())
}
