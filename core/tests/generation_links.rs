//! Real-filesystem v1/v2 parity for attribution and cross-boundary propagation.
use freespace_core::{
    job::{ScanRequest, run_scan},
    queries::common::{Paths, find_path},
    refresh::{RefreshRequest, run_refresh},
    scan::flags,
    snapshot::{
        SnapshotDir,
        reader::Snapshot,
        v2::{Reader, Store},
    },
};
use std::{fs, os::unix::ffi::OsStrExt, path::PathBuf};
struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
    v1: SnapshotDir,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        fs::create_dir(&root).unwrap();
        let db = tmp.path().join("v2.db");
        let store = Store::open(&db).unwrap();
        let v1 = SnapshotDir::new(tmp.path().join("v1"));
        Self {
            _tmp: tmp,
            root,
            db,
            v1,
            store,
        }
    }
    fn file(&self, p: &str, size: usize) {
        let p = self.root.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![1; size]).unwrap();
    }
    fn link(&self, from: &str, to: &str) {
        let p = self.root.join(to);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::hard_link(self.root.join(from), p).unwrap();
    }
    fn scan(&mut self) {
        self.store.scan(&self.root, 1).unwrap();
        run_scan(ScanRequest {
            roots: vec![self.root.clone()],
            one_file_system: true,
            workers: 1,
            dir: self.v1.clone(),
            keep_previous: true,
            on_progress: None,
        })
        .unwrap();
        self.parity();
    }
    fn refresh(&mut self, p: &str) -> freespace_core::snapshot::v2::WriteReport {
        let path = if p.is_empty() {
            self.root.clone()
        } else {
            self.root.join(p)
        };
        let report = self.store.refresh(&path, 1).unwrap();
        run_refresh(RefreshRequest {
            path: path.as_os_str().as_bytes().to_vec(),
            workers: 1,
            dir: self.v1.clone(),
            on_progress: None,
        })
        .unwrap();
        self.parity();
        report
    }
    fn reader(&self) -> Reader {
        Reader::open(&self.db).unwrap()
    }
    fn entry(&self, p: &str) -> freespace_core::snapshot::v2::Entry {
        self.reader()
            .get(self.root.join(p).as_os_str().as_bytes(), false)
            .unwrap()
            .unwrap()
    }
    fn parity(&self) {
        let snap = Snapshot::open_current(&self.v1).unwrap();
        let r = self.reader();
        let entries = r.subtree(self.root.as_os_str().as_bytes(), false).unwrap();
        let count: i64 = snap
            .conn
            .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(entries.len(), count as usize);
        let mut paths = Paths::new(&snap.conn);
        for e in entries {
            let v1 = find_path(&snap.conn, &e.path).unwrap().unwrap();
            assert_eq!(
                (
                    e.allocated,
                    e.apparent,
                    e.items,
                    e.errors,
                    e.flags,
                    e.kind.code(),
                    e.nlink,
                    e.newest_mtime.unwrap_or(0)
                ),
                (
                    v1.allocated,
                    v1.apparent,
                    v1.items,
                    v1.errors,
                    v1.flags,
                    v1.kind,
                    v1.nlink,
                    v1.newest_mtime
                ),
                "{}",
                String::from_utf8_lossy(&e.path)
            );
            if e.dev.is_some() {
                let owner =
                    freespace_core::queries::common::hardlink_owner(&snap.conn, &mut paths, v1.id)
                        .unwrap();
                assert_eq!(
                    r.hardlink_owner(&e.path, false)
                        .unwrap()
                        .map(|p| String::from_utf8_lossy(&p).into_owned()),
                    owner
                );
            }
        }
    }
}

#[test]
fn ownership_across_boundary_and_converging_ancestor_paths_match_v1() {
    let mut f = Fixture::new();
    f.file("a/x", 4096);
    f.link("a/x", "z/x");
    f.file("a/y", 8192);
    f.link("a/y", "z/y");
    f.file("untouched/u", 4);
    f.scan();
    f.scan();
    let old = f.reader();
    let unaffected = f.entry("untouched/u");
    f.refresh("a");
    f.refresh("z");
    fs::remove_file(f.root.join("a/x")).unwrap();
    fs::remove_file(f.root.join("a/y")).unwrap();
    let report = f.refresh("a");
    assert_eq!(report.touched_inode_groups, 2);
    assert_eq!(report.known_inode_members, 4);
    assert_eq!(report.ancestors, 2, "z and root are each updated once");
    assert_eq!(f.entry("untouched/u"), unaffected);
    assert_eq!(f.entry("z/x").flags & flags::ALIAS, 0);
    assert_eq!(f.entry("z/y").flags & flags::ALIAS, 0);
    assert_ne!(
        old.get(f.root.join("z/x").as_os_str().as_bytes(), false)
            .unwrap()
            .unwrap()
            .flags
            & flags::ALIAS,
        0
    );
    assert_ne!(
        f.reader()
            .get(f.root.join("z/x").as_os_str().as_bytes(), true)
            .unwrap()
            .unwrap()
            .flags
            & flags::ALIAS,
        0
    );
}

#[test]
fn unknown_aliases_zeroed_groups_and_recovery_match_v1() {
    let mut f = Fixture::new();
    for p in ["in", "out", "other"] {
        fs::create_dir(f.root.join(p)).unwrap();
    }
    f.scan();
    f.file("out/w", 8192);
    f.link("out/w", "in/z");
    f.refresh("in");
    assert_eq!(f.entry("in/z").allocated, 0);
    f.file("in/n1", 4096);
    f.link("in/n1", "in/n2");
    f.refresh("in");
    assert_eq!(f.entry("in/z").allocated, 0);
    assert!(f.entry("in/n1").allocated > 0);
    assert_eq!(f.entry("in/n2").allocated, 0);
    f.refresh("in");
    let report = f.refresh("other");
    assert_eq!(report.touched_inode_groups, 0);
    assert_eq!(f.entry("in/z").allocated, 0);
    f.refresh("out");
    assert!(f.entry("in/z").allocated > 0);
    assert_eq!(f.entry("out/w").allocated, 0);
}

#[test]
fn surviving_owner_beats_new_smaller_path_and_full_scan_uses_raw_order() {
    let mut f = Fixture::new();
    fs::create_dir(f.root.join("a")).unwrap();
    f.file("z/x", 4096);
    f.link("z/x", "zz/x");
    f.scan();
    f.link("z/x", "a/x");
    f.refresh("a");
    assert_eq!(f.entry("a/x").allocated, 0);
    assert!(f.entry("z/x").allocated > 0);
    // a-/x sorts before a/x as a raw full path, while DFS visits a first.
    f.link("z/x", "a-/x");
    f.scan();
    assert!(f.entry("a-/x").allocated > 0);
    assert_eq!(f.entry("a/x").allocated, 0);
    f.refresh("a-");
    assert!(f.entry("a-/x").allocated > 0);
}

#[test]
fn root_refresh_and_same_filesystem_overlap_validation() {
    let mut f = Fixture::new();
    f.file("local/a", 4096);
    f.link("local/a", "outside");
    f.scan();
    f.file("new", 4);
    f.refresh("");
    let before = f.reader().generations;
    assert!(
        f.store
            .scan_roots(&[f.root.clone(), f.root.join("local")], true, 1)
            .is_err()
    );
    assert_eq!(f.reader().generations, before);
    f.store
        .scan_roots(&[f.root.clone(), f.root.clone()], true, 1)
        .unwrap();
    assert_eq!(f.reader().roots(false).unwrap().len(), 1);
}

#[test]
fn hardlink_owner_moves_between_independently_configured_roots() {
    let mut f = Fixture::new();
    f.file("a/x", 4096);
    f.link("a/x", "z/x");
    let roots = vec![f.root.join("a"), f.root.join("z")];
    f.store.scan_roots(&roots, true, 1).unwrap();
    run_scan(ScanRequest {
        roots,
        one_file_system: true,
        workers: 1,
        dir: f.v1.clone(),
        keep_previous: true,
        on_progress: None,
    })
    .unwrap();
    let before = f.entry("z/x");
    assert_eq!(before.allocated, 0);
    fs::remove_file(f.root.join("a/x")).unwrap();
    f.store.refresh(&f.root.join("a"), 1).unwrap();
    run_refresh(RefreshRequest {
        path: f.root.join("a").as_os_str().as_bytes().to_vec(),
        workers: 1,
        dir: f.v1.clone(),
        on_progress: None,
    })
    .unwrap();
    let snap = Snapshot::open_current(&f.v1).unwrap();
    let r = f.reader();
    for root in r.roots(false).unwrap() {
        for e in r.subtree(&root.path, false).unwrap() {
            let old = find_path(&snap.conn, &e.path).unwrap().unwrap();
            assert_eq!(
                (e.allocated, e.apparent, e.flags),
                (old.allocated, old.apparent, old.flags)
            );
        }
    }
    assert_eq!(f.entry("z/x").id, before.id);
    assert!(f.entry("z/x").allocated > 0);
}

#[test]
fn unrelated_inode_groups_do_not_expand_touched_group_work() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let mut measurements = Vec::new();
    for width in [10, 1000] {
        let mut f = Fixture::new();
        f.file("a/x", 4096);
        f.link("a/x", "z/x");
        for n in 0..width {
            f.file(&format!("unrelated/{n}/a"), 16);
            f.link(&format!("unrelated/{n}/a"), &format!("unrelated/{n}/b"));
        }
        f.store.scan(&f.root, 1).unwrap();
        fs::remove_file(f.root.join("a/x")).unwrap();
        let steps = Arc::new(AtomicU64::new(0));
        f.store.observe_vm_steps(Some(steps.clone())).unwrap();
        let report = f.store.refresh(&f.root.join("a"), 1).unwrap();
        assert_eq!(report.touched_inode_groups, 1);
        assert_eq!(report.known_inode_members, 2);
        assert_eq!(report.ancestors, 2);
        measurements.push((report.sql_changes, steps.load(Ordering::Relaxed)));
    }
    assert_eq!(measurements[0].0, measurements[1].0);
    assert!(
        measurements[1].1 < measurements[0].1 * 2,
        "{measurements:?}"
    );
}

#[test]
fn publication_failure_rolls_back_external_owner_and_root_metadata() {
    let mut f = Fixture::new();
    f.file("a/x", 4096);
    f.link("a/x", "z/x");
    f.scan();
    f.scan();
    let pinned = f.reader();
    let old = pinned
        .subtree(f.root.as_os_str().as_bytes(), false)
        .unwrap();
    let old_roots = pinned.roots(false).unwrap();
    fs::remove_file(f.root.join("a/x")).unwrap();
    let c = rusqlite::Connection::open(&f.db).unwrap();
    c.execute_batch("CREATE TRIGGER fail_publish BEFORE UPDATE ON control BEGIN SELECT RAISE(ABORT,'injected publication failure'); END;").unwrap();
    assert!(f.store.refresh(&f.root.join("a"), 1).is_err());
    let after = f.reader();
    assert_eq!(after.generations, pinned.generations);
    assert_eq!(
        after.subtree(f.root.as_os_str().as_bytes(), false).unwrap(),
        old
    );
    assert_eq!(after.roots(false).unwrap(), old_roots);
    assert!(after.refresh_history().unwrap().is_empty());
    c.execute_batch("DROP TRIGGER fail_publish").unwrap();
    f.refresh("a");
}
