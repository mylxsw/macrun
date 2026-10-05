//! Physical resources are shared across all connection profiles in one process.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};
pub fn resource(group: &str) -> Arc<tokio::sync::Mutex<()>> {
    static GROUPS: OnceLock<Mutex<BTreeMap<String, Weak<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let mut groups = GROUPS.get_or_init(Default::default).lock().unwrap();
    groups.retain(|_, g| g.strong_count() > 0);
    if let Some(lock) = groups.get(group).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    groups.insert(group.into(), Arc::downgrade(&lock));
    lock
}

pub struct Reservation {
    pub group: String,
    pub guard: tokio::sync::OwnedMutexGuard<()>,
}
