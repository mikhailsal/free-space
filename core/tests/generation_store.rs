use freespace_core::snapshot::v2::{Entry, Reader, Store};
use rusqlite::Connection;
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant, SystemTime};

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        fs::create_dir(&root).unwrap();
        let db = tmp.path().join("v2.db");
        let store = Store::open(&db).unwrap();
        Self {
            _tmp: tmp,
            root,
            db,
            store,
        }
    }
    fn write(&self, name: &str, size: usize) {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![1; size]).unwrap();
    }
    fn scan(&mut self) {
        self.store.scan(&self.root, 1).unwrap();
    }
    fn reader(&self) -> Reader {
        Reader::open(&self.db).unwrap()
    }
    fn get(&self, name: &str) -> Entry {
        let path = if name.is_empty() {
            self.root.clone()
        } else {
            self.root.join(name)
        };
        self.reader().get(bytes(&path), false).unwrap().unwrap()
    }
}
fn bytes(path: &Path) -> &[u8] {
    path.as_os_str().as_bytes()
}

#[test]
fn pin_pair_refresh_delete_and_full_scan_rotation() {
    let mut f = Fixture::new();
    f.write("local/a", 8);
    f.write("other/b", 10);
    f.scan();
    let first = f.reader();
    assert_eq!(first.generations.current, 1);
    assert_eq!(first.generations.prev, None);
    f.scan();
    let pinned = f.reader();
    let unchanged = f.get("other/b");
    let before = f.get("");
    let local_before = f.get("local");
    f.write("local/a", 30);
    f.write("local/c", 20);
    let report = f.store.refresh(&f.root.join("local"), 1).unwrap();
    assert_eq!(
        (
            report.old_subtree_entries,
            report.new_subtree_entries,
            report.ancestors
        ),
        (2, 3, 1)
    );
    let after = f.reader();
    assert_eq!(
        (after.generations.current, after.generations.prev),
        (3, Some(1))
    );
    assert_eq!(f.get("other/b"), unchanged);
    assert_eq!(f.get("local").id, local_before.id);
    assert_eq!(
        pinned
            .get(bytes(&f.root.join("local/a")), false)
            .unwrap()
            .unwrap()
            .apparent,
        8
    );
    assert_eq!(
        first
            .get(bytes(&f.root.join("local/a")), false)
            .unwrap()
            .unwrap()
            .apparent,
        8
    );
    assert_eq!(
        after
            .get(bytes(&f.root.join("local/a")), true)
            .unwrap()
            .unwrap()
            .apparent,
        8
    );
    let local_after = f.get("local");
    assert_eq!(
        f.get("").apparent,
        before.apparent - local_before.apparent + local_after.apparent
    );
    assert_eq!(after.refresh_history().unwrap().len(), 1);
    assert!(
        after
            .diff(bytes(&f.root))
            .unwrap()
            .iter()
            .any(|c| c.path.ends_with(b"local/c") && c.before.is_none())
    );
    fs::remove_dir_all(f.root.join("local")).unwrap();
    f.store.refresh(&f.root.join("local"), 1).unwrap();
    assert!(
        f.reader()
            .get(bytes(&f.root.join("local")), false)
            .unwrap()
            .is_none()
    );
    assert_eq!(f.reader().generations.prev, Some(1));
    assert_eq!(f.reader().refresh_history().unwrap().len(), 2);
    assert_eq!(f.get("").apparent, before.apparent - local_before.apparent);
    f.scan();
    assert_eq!(f.reader().generations.prev, Some(4));
    assert!(f.reader().refresh_history().unwrap().is_empty());
    assert_eq!(pinned.generations.current, 2);
}

#[test]
fn deleted_extrema_and_negative_file_times() {
    let mut f = Fixture::new();
    f.write("local/newest", 1);
    f.write("older", 1);
    fs::create_dir(f.root.join("empty")).unwrap();
    for (name, seconds) in [("local/newest", 100), ("older", 200)] {
        fs::File::options()
            .write(true)
            .open(f.root.join(name))
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH - Duration::from_secs(seconds)),
            )
            .unwrap();
    }
    f.scan();
    assert_eq!(f.get("").newest_mtime, Some(-100));
    assert_eq!(f.get("empty").newest_mtime, None);
    let old_min = f.get("").min_scanned_at;
    fs::remove_file(f.root.join("local/newest")).unwrap();
    f.store.refresh(&f.root.join("local"), 1).unwrap();
    assert_eq!(f.get("").newest_mtime, Some(-200));
    assert_eq!(f.get("local").newest_mtime, None);
    assert_eq!(f.get("").min_scanned_at, old_min);
}

#[test]
fn byte_paths_component_dfs_and_stable_ids() {
    let mut f = Fixture::new();
    f.write("a/z", 1);
    f.write("a-", 2);
    f.write("ab/x", 3);
    let raw = f.root.join(std::ffi::OsString::from_vec(vec![0xff]));
    fs::write(&raw, [1]).unwrap();
    f.scan();
    let r = f.reader();
    let entries = r.subtree(bytes(&f.root), false).unwrap();
    let names: Vec<_> = entries
        .iter()
        .map(|e| {
            Path::new(std::ffi::OsStr::from_bytes(&e.path))
                .strip_prefix(&f.root)
                .unwrap()
                .as_os_str()
                .as_bytes()
                .to_vec()
        })
        .collect();
    assert_eq!(
        names,
        vec![
            b"".to_vec(),
            b"a".to_vec(),
            b"a/z".to_vec(),
            b"a-".to_vec(),
            b"ab".to_vec(),
            b"ab/x".to_vec(),
            vec![255]
        ]
    );
    assert_eq!(r.subtree(bytes(&f.root.join("a")), false).unwrap().len(), 2);
    assert!(r.get(bytes(&raw), false).unwrap().is_some());
    let stable = f.get("ab/x");
    f.write("a/new", 5);
    f.store.refresh(&f.root.join("a"), 1).unwrap();
    assert_eq!(f.get("ab/x"), stable);
}

#[test]
fn failure_after_staged_deletion_rolls_back_all_state() {
    let mut f = Fixture::new();
    f.write("local/a", 1);
    f.scan();
    f.scan();
    let before = f.reader();
    let rows = before.subtree(bytes(&f.root), false).unwrap();
    let conn = Connection::open(&f.db).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_publish BEFORE UPDATE ON control BEGIN SELECT RAISE(ABORT,'injected publication failure'); END;").unwrap();
    f.write("local/b", 9);
    assert!(f.store.refresh(&f.root.join("local"), 1).is_err());
    let after = f.reader();
    assert_eq!(after.generations, before.generations);
    assert_eq!(after.subtree(bytes(&f.root), false).unwrap(), rows);
    assert!(after.refresh_history().unwrap().is_empty());
    conn.execute_batch("DROP TRIGGER fail_publish").unwrap();
    f.store.refresh(&f.root.join("local"), 1).unwrap();
    assert_eq!(f.reader().generations.current, 3);
}

#[test]
fn reject_foreign_databases_without_mutation() {
    let f = Fixture::new();
    let p = f.root.join("v1.db");
    let c = Connection::open(&p).unwrap();
    c.execute_batch("CREATE TABLE meta(schema_version INTEGER); INSERT INTO meta VALUES(1)")
        .unwrap();
    assert!(Store::open(&p).is_err());
    assert_eq!(
        c.query_row("SELECT schema_version FROM meta", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn unrelated_width_does_not_expand_mutations_or_vm_work() {
    let mut measurements = Vec::new();
    for width in [10, 2000] {
        let mut f = Fixture::new();
        f.write("local/a", 1);
        // Many direct children exercises extrema recomputation on a wide parent.
        for n in 0..width {
            f.write(&format!("unrelated-{n:05}"), 1);
        }
        f.scan();
        let count = Arc::new(AtomicU64::new(0));
        f.store.observe_vm_steps(Some(count.clone())).unwrap();
        let report = f.store.refresh(&f.root.join("local"), 1).unwrap();
        measurements.push((report.sql_changes, count.load(Ordering::Relaxed)));
        let c = Connection::open(&f.db).unwrap();
        for sql in [
            "SELECT min_scanned_at FROM live WHERE parent=1 ORDER BY min_scanned_at LIMIT 1",
            "SELECT newest_mtime FROM live WHERE parent=1 AND newest_mtime IS NOT NULL ORDER BY newest_mtime DESC LIMIT 1",
        ] {
            let mut q = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let plan = q
                .query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .join(" ");
            assert!(plan.contains("COVERING INDEX live_child_"), "{plan}");
            assert!(!plan.contains("TEMP B-TREE"), "{plan}");
        }
    }
    assert_eq!(measurements[0].0, measurements[1].0);
    assert!(
        measurements[1].1 < measurements[0].1 * 2,
        "unexpected unrelated work: {measurements:?}"
    );
}

#[test]
#[ignore = "reproducible locality benchmark; run explicitly in release mode"]
fn locality_benchmark() {
    println!(
        "unrelated,repeat,scan_total_ms,scan_walk_ms,scan_store_ms,refresh_total_ms,refresh_walk_ms,refresh_store_ms,sql_changes,vm_steps,wal_growth_bytes"
    );
    for width in [100, 1000, 10000] {
        let mut f = Fixture::new();
        f.write("local/a", 32);
        for n in 0..width {
            f.write(&format!("unrelated-{n:05}"), 16);
        }
        f.scan();
        for repeat in 0..5 {
            let start = Instant::now();
            let scan = f.store.scan(&f.root, 1).unwrap();
            let scan_total = start.elapsed();
            let wal = f.db.with_extension("db-wal");
            let size_before = fs::metadata(&wal).unwrap().len();
            let start = Instant::now();
            let report = f.store.refresh(&f.root.join("local"), 1).unwrap();
            let refresh_total = start.elapsed();
            let growth = fs::metadata(&wal).unwrap().len() - size_before;
            let counter = Arc::new(AtomicU64::new(0));
            f.store.observe_vm_steps(Some(counter.clone())).unwrap();
            f.store.refresh(&f.root.join("local"), 1).unwrap();
            f.store.observe_vm_steps(None).unwrap();
            println!(
                "{width},{repeat},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{},{},{}",
                scan_total.as_secs_f64() * 1000.,
                scan.walk.as_secs_f64() * 1000.,
                scan.transaction.as_secs_f64() * 1000.,
                refresh_total.as_secs_f64() * 1000.,
                report.walk.as_secs_f64() * 1000.,
                report.transaction.as_secs_f64() * 1000.,
                report.sql_changes,
                counter.load(Ordering::Relaxed),
                growth
            );
        }
    }
}

#[test]
fn deleting_oldest_branch_recomputes_minimum_from_live_index() {
    let mut f = Fixture::new();
    f.write("local/a", 1);
    f.write("sibling", 1);
    f.scan();
    // Distinct deterministic scan stamps model a snapshot after earlier refreshes.
    let c = Connection::open(&f.db).unwrap();
    c.execute_batch("UPDATE live SET scanned_at=500,min_scanned_at=500")
        .unwrap();
    c.execute(
        "UPDATE live SET scanned_at=100,min_scanned_at=100 WHERE path=?1",
        [bytes(&f.root.join("local"))],
    )
    .unwrap();
    c.execute(
        "UPDATE live SET scanned_at=1000,min_scanned_at=100 WHERE path=?1",
        [bytes(&f.root)],
    )
    .unwrap();
    c.execute_batch("UPDATE versions SET scanned_at=(SELECT scanned_at FROM live WHERE live.id=versions.id),min_scanned_at=(SELECT min_scanned_at FROM live WHERE live.id=versions.id)").unwrap();
    assert_eq!(f.get("").min_scanned_at, 100);
    fs::remove_dir_all(f.root.join("local")).unwrap();
    f.store.refresh(&f.root.join("local"), 1).unwrap();
    assert_eq!(f.get("").min_scanned_at, 500);
    assert_eq!(f.get("").items, 1);
}

#[test]
fn seeded_aggregates_match_existing_tree_pipeline() {
    let mut f = Fixture::new();
    f.write("a/b/x", 123);
    f.write("a/y", 321);
    f.write("z", 5);
    f.scan();
    let mut tree = freespace_core::scan::walk(&freespace_core::scan::WalkOptions::new(vec![
        f.root.clone(),
    ]))
    .unwrap();
    tree.finalize();
    let old = &tree.nodes[0];
    let new = f.get("");
    assert_eq!(new.allocated, old.allocated as i64);
    assert_eq!(new.apparent, old.apparent as i64);
    assert_eq!(new.items, old.items as i64);
    assert_eq!(new.errors, old.errors as i64);
    assert_eq!(new.newest_mtime, Some(old.newest_mtime));
}

#[test]
fn reopened_store_retains_generations_and_rejects_unsafe_paths() {
    let mut f = Fixture::new();
    f.write("local/a", 1);
    f.scan();
    f.scan();
    let mut reopened = Store::open(&f.db).unwrap();
    reopened.refresh(&f.root.join("local"), 1).unwrap();
    assert_eq!(f.reader().generations.current, 3);
    assert_eq!(f.reader().generations.prev, Some(1));
    reopened.refresh(&f.root, 1).unwrap();
    assert!(reopened.refresh(&f.root.join("local/.."), 1).is_err());
    fs::remove_dir_all(f.root.join("local")).unwrap();
    std::os::unix::fs::symlink(&f.root, f.root.join("local")).unwrap();
    assert!(reopened.refresh(&f.root.join("local"), 1).is_err());
    assert_eq!(f.reader().generations.current, 4);
}
