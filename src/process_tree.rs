//! Own only descendants of one command, including helpers that create a new session.
//! The inherited nonce also identifies children reparented between process scans.
use super::process::Identity;
use crate::{
    model::{Fault, Outcome},
    wire,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

pub(crate) const OWNER_ENV: &str = "MACRUN_PROCESS_OWNER";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OwnedProcess {
    pid: u32,
    birth: String,
    executable: PathBuf,
}

// Unlike second-resolution start_time, these distinguish rapid PID reuse. An exec
// changes the executable but not this identity; do not lose ownership on exec.
#[cfg(target_os = "macos")]
fn birth(pid: u32) -> Option<String> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (read == size).then(|| format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec))
}
#[cfg(target_os = "linux")]
fn birth(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ticks = stat.rsplit_once(") ")?.1.split_whitespace().nth(19)?;
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    Some(format!("{}:{ticks}", boot.trim()))
}

// Poll only the known ancestry at high frequency; full environment scans are
// reserved for cleanup/recovery. This catches platform helpers with hidden envs
// without repeatedly reading every process's arguments on the machine.
#[cfg(target_os = "macos")]
fn children(pid: u32) -> Vec<u32> {
    // This convenience API returns PID counts, unlike proc_listpids's byte count.
    let count = unsafe { libc::proc_listchildpids(pid as i32, std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    let mut pids = vec![0i32; count as usize + 16];
    let count = unsafe {
        libc::proc_listchildpids(
            pid as i32,
            pids.as_mut_ptr().cast(),
            std::mem::size_of_val(pids.as_slice()) as i32,
        )
    };
    if count <= 0 {
        return Vec::new();
    }
    pids.into_iter()
        .take(count as usize)
        .filter(|p| *p > 0)
        .map(|p| p as u32)
        .collect()
}
#[cfg(target_os = "linux")]
fn children(pid: u32) -> Vec<u32> {
    // A subprocess can be created by any thread, not just the thread-group leader.
    let Ok(threads) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return Vec::new();
    };
    threads
        .flatten()
        .filter_map(|t| std::fs::read_to_string(t.path().join("children")).ok())
        .flat_map(|text| {
            text.split_whitespace()
                .filter_map(|p| p.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect()
}

impl OwnedProcess {
    fn matches(&self) -> bool {
        birth(self.pid).as_ref() == Some(&self.birth)
    }
    fn inaccessible_but_present(&self) -> bool {
        birth(self.pid).is_none()
            && (unsafe { libc::kill(self.pid as i32, 0) } == 0
                || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH))
    }
    fn signal(&self, signal: i32) {
        if self.matches() {
            unsafe {
                libc::kill(self.pid as i32, signal);
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Tree {
    version: u8,
    owner: String,
    #[serde(default)]
    processes: BTreeMap<u32, OwnedProcess>,
    #[serde(default)]
    group: Option<OwnedProcess>,
    #[serde(skip)]
    armed: bool,
}
impl Tree {
    pub(crate) fn new() -> Self {
        Self {
            version: 2,
            owner: uuid::Uuid::new_v4().to_string(),
            processes: BTreeMap::new(),
            group: None,
            armed: true,
        }
    }
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }
    pub(crate) fn add_root(&mut self, pid: u32) {
        if let Some(birth) = birth(pid) {
            self.group = Some(OwnedProcess {
                pid,
                birth: birth.clone(),
                executable: PathBuf::new(),
            });
            self.processes.insert(
                pid,
                OwnedProcess {
                    pid,
                    birth,
                    executable: PathBuf::new(),
                },
            );
        }
    }
    pub(crate) fn refresh_children(&mut self) -> bool {
        let previous = self.processes.clone();
        self.processes
            .retain(|_, p| p.matches() || p.inaccessible_but_present());
        let mut pending: Vec<u32> = self.processes.keys().copied().collect();
        let mut visited = BTreeSet::new();
        while let Some(pid) = pending.pop() {
            if !visited.insert(pid) || !self.processes[&pid].matches() {
                continue;
            }
            for child in children(pid) {
                if !self.processes.contains_key(&child)
                    && let Some(birth) = birth(child)
                {
                    self.processes.insert(
                        child,
                        OwnedProcess {
                            pid: child,
                            birth,
                            executable: crate::process::identity(child)
                                .map(|i| i.executable)
                                .unwrap_or_default(),
                        },
                    );
                }
                if self.processes.contains_key(&child) {
                    pending.push(child);
                }
            }
        }
        self.processes != previous
    }
    pub(crate) fn refresh(&mut self) {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_exe(UpdateKind::Always)
                .with_environ(UpdateKind::Always),
        );
        let marker = format!("{OWNER_ENV}={}", self.owner);
        // Keep the original process-group guarantee for children whose environment
        // macOS hides (e.g. platform binaries). During recovery an absent leader
        // needs a surviving recorded member before its group can be attributed.
        let group = self
            .group
            .as_ref()
            .filter(|root| {
                root.matches()
                    || (birth(root.pid).is_none()
                        && (self.armed
                            || self.processes.values().any(|p| {
                                p.matches()
                                    && unsafe { libc::getpgid(p.pid as i32) } == root.pid as i32
                            })))
            })
            .map(|root| root.pid);
        // Accept known PIDs only while the exact birth identity still matches.
        let mut live = BTreeMap::new();
        for (pid, p) in system.processes() {
            if p.status() == sysinfo::ProcessStatus::Zombie {
                continue;
            }
            let pid = pid.as_u32();
            let marked = p.environ().iter().any(|e| e == marker.as_str());
            let Some(birth) = birth(pid) else {
                continue;
            };
            let known = self.processes.get(&pid).is_some_and(|p| p.birth == birth);
            let grouped =
                group.is_some_and(|group| unsafe { libc::getpgid(pid as i32) } == group as i32);
            if marked || known || grouped {
                live.insert(
                    pid,
                    OwnedProcess {
                        pid,
                        birth,
                        executable: p.exe().unwrap_or(Path::new("")).into(),
                    },
                );
            }
        }
        // A child may clear its environment. While its ancestry is visible, bind
        // it to the same precise birth identity and continue tracking after exec.
        loop {
            let count = live.len();
            for (pid, p) in system.processes() {
                let pid = pid.as_u32();
                if !live.contains_key(&pid)
                    && p.status() != sysinfo::ProcessStatus::Zombie
                    && p.parent()
                        .is_some_and(|parent| live.contains_key(&parent.as_u32()))
                    && let Some(birth) = birth(pid)
                {
                    live.insert(
                        pid,
                        OwnedProcess {
                            pid,
                            birth,
                            executable: p.exe().unwrap_or(Path::new("")).into(),
                        },
                    );
                }
            }
            if live.len() == count {
                break;
            }
        }
        // Losing inspection permission is not proof that a recorded child exited.
        // Keep it unresolved; signalling still requires a matching birth identity.
        for (pid, p) in &self.processes {
            if p.inaccessible_but_present() {
                live.entry(*pid).or_insert_with(|| p.clone());
            }
        }
        self.processes = live;
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.processes.is_empty()
            && !self.group.as_ref().is_some_and(|root| {
                // A leaderless group without an attributable member is ambiguous
                // after recovery. Keep its journal for inspection, never signal it.
                birth(root.pid).is_none() && unsafe { libc::kill(-(root.pid as i32), 0) } == 0
            })
    }
    pub(crate) fn signal(&self, signal: i32) {
        for p in self.processes.values().rev() {
            p.signal(signal);
        }
    }
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
    pub(crate) async fn persist(&self, path: &Path, root: &Option<Identity>) -> Outcome<()> {
        wire::persist(
            path,
            &serde_json::json!({"state":"running", "identity":root,"tree":self}),
        )
        .await
        .map_err(|e| Fault::new("recovery_required", e))
    }
    pub(crate) fn recover(value: &serde_json::Value) -> Outcome<Self> {
        let mut tree: Self = serde_json::from_value(value.clone())
            .map_err(|e| Fault::new("recovery_required", e))?;
        if tree.version != 2 || uuid::Uuid::parse_str(&tree.owner).is_err() {
            return Err(Fault::new(
                "recovery_required",
                "invalid process ownership journal",
            ));
        }
        tree.armed = false;
        tree.refresh();
        Ok(tree)
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        if self.armed {
            self.refresh();
            self.signal(libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};

    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn spawn(owner: Option<&str>) -> ChildGuard {
        let mut command = Command::new("python3");
        command.args(["-c", "import time; time.sleep(30)"]);
        if let Some(owner) = owner {
            command.env(OWNER_ENV, owner);
        }
        let child = ChildGuard(command.spawn().unwrap());
        std::thread::sleep(std::time::Duration::from_millis(100));
        child
    }

    #[test]
    fn markers_isolate_tasks_and_stale_birth_cannot_kill_another_process() {
        let mut first = Tree::new();
        let mut second = Tree::new();
        let mut a = spawn(Some(first.owner()));
        let mut b = spawn(Some(second.owner()));
        let mut unrelated = spawn(None);
        first.processes.insert(
            unrelated.0.id(),
            OwnedProcess {
                pid: unrelated.0.id(),
                birth: "stale".into(),
                executable: PathBuf::new(),
            },
        );
        first.refresh();
        assert!(first.processes.contains_key(&a.0.id()));
        assert!(!first.processes.contains_key(&b.0.id()));
        assert!(!first.processes.contains_key(&unrelated.0.id()));
        first.signal(libc::SIGKILL);
        a.0.wait().unwrap();
        assert!(b.0.try_wait().unwrap().is_none());
        assert!(unrelated.0.try_wait().unwrap().is_none());
        second.refresh();
        assert!(second.processes.contains_key(&b.0.id()));
        first.disarm();
        second.disarm();
    }

    #[test]
    fn tracked_process_survives_executable_change_and_journal_roundtrip() {
        let mut tree = Tree::new();
        let mut child = spawn(None);
        tree.add_root(child.0.id());
        tree.processes.get_mut(&child.0.id()).unwrap().executable =
            PathBuf::from("/previous/executable");
        let serialized = serde_json::to_value(&tree).unwrap();
        tree.disarm();
        let recovered = Tree::recover(&serialized).unwrap();
        assert!(recovered.processes.contains_key(&child.0.id()));
        recovered.signal(libc::SIGKILL);
        child.0.wait().unwrap();
    }

    #[tokio::test]
    async fn recovery_finds_marked_orphans_and_retains_record_until_exit() {
        let dir = tempfile::tempdir().unwrap();
        let journal = dir.path().join("process.json");
        let mut tree = Tree::new();
        let mut child = spawn(Some(tree.owner()));
        // Simulate crashing immediately after spawn, before recording a PID.
        tree.persist(&journal, &None).await.unwrap();
        tree.disarm();
        assert_eq!(
            crate::process::recover(&journal).unwrap_err().code,
            "recovery_required"
        );
        assert!(journal.exists());
        child.0.wait().unwrap();
        crate::process::recover(&journal).unwrap();
        assert!(!journal.exists());
    }

    #[test]
    fn rejects_invalid_ownership_records_without_signalling() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"version":3,"owner":uuid::Uuid::new_v4().to_string()}),
            serde_json::json!({"version":2,"owner":""}),
        ] {
            assert!(Tree::recover(&value).is_err());
        }
        let stale = OwnedProcess {
            pid: std::process::id(),
            birth: "stale".into(),
            executable: PathBuf::new(),
        };
        stale.signal(libc::SIGKILL); // Must not terminate the test process.
    }
}
