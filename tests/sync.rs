use macrun::sync::{Receiver, scan};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
fn apply(source: &Path, target: &Path, state: &Path) -> usize {
    let m = scan(source, &[]).unwrap();
    let r = Receiver::prepare(target, state, m.clone()).unwrap();
    for p in &r.needed {
        let t = target.join("transfer.tmp");
        fs::copy(source.join(p), &t).unwrap();
        r.install(p, &t).unwrap();
    }
    r.commit(&m).unwrap();
    r.needed.len()
}
#[test]
fn changes_deletes_links_and_unmanaged_files() {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("src");
    let d = t.path().join("dst");
    let state = t.path().join("state");
    fs::create_dir(&s).unwrap();
    fs::write(s.join("a"), "first").unwrap();
    fs::set_permissions(s.join("a"), fs::Permissions::from_mode(0o755)).unwrap();
    symlink(s.join("a"), s.join("link")).unwrap();
    symlink("/etc/hosts", s.join("outside")).unwrap();
    assert_eq!(apply(&s, &d, &state), 1);
    assert_eq!(fs::read_link(d.join("link")).unwrap(), Path::new("a"));
    assert!(!d.join("outside").exists());
    assert_eq!(
        fs::metadata(d.join("a")).unwrap().permissions().mode() & 0o111,
        0o111
    );
    assert_eq!(apply(&s, &d, &state), 0);
    fs::write(d.join("generated"), "cache").unwrap();
    fs::write(s.join("a"), "changed").unwrap();
    assert_eq!(apply(&s, &d, &state), 1);
    fs::remove_file(s.join("a")).unwrap();
    fs::remove_file(s.join("link")).unwrap();
    apply(&s, &d, &state);
    assert!(!d.join("a").exists());
    assert!(d.join("generated").exists());
}
#[test]
fn interrupted_first_sync_is_repaired_and_removed_files_do_not_survive() {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("src");
    let d = t.path().join("dst");
    let st = t.path().join("state");
    fs::create_dir(&s).unwrap();
    fs::write(s.join("a"), "a").unwrap();
    let m = scan(&s, &[]).unwrap();
    let r = Receiver::prepare(&d, &st, m).unwrap();
    let temp = d.join("transfer.tmp");
    fs::write(&temp, "a").unwrap();
    r.install("a", &temp).unwrap();
    assert!(st.join("sync_incomplete.json").exists());
    fs::remove_file(s.join("a")).unwrap();
    apply(&s, &d, &st);
    assert!(!d.join("a").exists());
    assert!(!st.join("sync_incomplete.json").exists());
}
#[test]
fn source_mutation_and_corruption_are_detected() {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("src");
    let d = t.path().join("dst");
    let st = t.path().join("state");
    fs::create_dir(&s).unwrap();
    fs::write(s.join("a"), "original").unwrap();
    let m = scan(&s, &[]).unwrap();
    let r = Receiver::prepare(&d, &st, m).unwrap();
    let temp = d.join("transfer.tmp");
    fs::write(&temp, "tampered").unwrap();
    assert_eq!(r.install("a", &temp).unwrap_err().code, "source_changed");
    apply(&s, &d, &st);
    fs::write(d.join("a"), "corrupt").unwrap();
    assert_eq!(apply(&s, &d, &st), 1);
    assert_eq!(fs::read_to_string(d.join("a")).unwrap(), "original");
}
#[test]
fn type_changes_preserve_unmanaged_children() {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("src");
    let d = t.path().join("dst");
    let st = t.path().join("state");
    fs::create_dir(&s).unwrap();
    fs::write(s.join("a"), "a").unwrap();
    apply(&s, &d, &st);
    fs::remove_file(s.join("a")).unwrap();
    fs::create_dir(s.join("a")).unwrap();
    fs::write(s.join("a/b"), "b").unwrap();
    apply(&s, &d, &st);
    fs::write(d.join("a/generated"), "keep").unwrap();
    fs::remove_file(s.join("a/b")).unwrap();
    fs::remove_dir(s.join("a")).unwrap();
    fs::write(s.join("a"), "file").unwrap();
    assert_eq!(
        Receiver::prepare(&d, &st, scan(&s, &[]).unwrap())
            .err()
            .unwrap()
            .code,
        "path_conflict"
    );
    assert!(d.join("a/generated").exists());
    fs::remove_file(d.join("a/generated")).unwrap();
    apply(&s, &d, &st);
    assert_eq!(fs::read_to_string(d.join("a")).unwrap(), "file");
}
