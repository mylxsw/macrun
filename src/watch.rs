use anyhow::Result;
use notify::{RecursiveMode, Watcher};
use std::path::Path;

/// Coalesced notifications; dropped/overflowed events request a fresh scan too.
pub fn source(
    root: &Path,
) -> Result<(
    notify::RecommendedWatcher,
    tokio::sync::watch::Receiver<u64>,
)> {
    let (tx, rx) = tokio::sync::watch::channel(0u64);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event
            .as_ref()
            .is_ok_and(|e| matches!(e.kind, notify::EventKind::Access(_)))
        {
            return;
        }
        tx.send_modify(|n| *n = n.wrapping_add(1));
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok((watcher, rx))
}
