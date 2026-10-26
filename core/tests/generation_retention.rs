use freespace_core::{
    ErrorCode,
    snapshot::v2::{Reader, RetentionPolicy, Store},
};
use std::{fs, os::unix::ffi::OsStrExt, path::PathBuf};
struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        fs::create_dir_all(root.join("local")).unwrap();
        fs::write(root.join("local/a"), [1]).unwrap();
        let db = tmp.path().join("v2.db");
        let mut store = Store::open(&db).unwrap();
        store.scan(&root, 1).unwrap();
        store.scan(&root, 1).unwrap();
        Self {
            _tmp: tmp,
            root,
            db,
            store,
        }
    }
    fn reader(&self) -> Reader {
        Reader::open(&self.db).unwrap()
    }
    // Maintenance is deliberately nonblocking. A parallel subprocess spawn can
    // briefly inherit a CLOEXEC lease descriptor until exec closes it.
    fn upkeep(
        &mut self,
        budget: usize,
        checkpoint: bool,
    ) -> freespace_core::Result<freespace_core::snapshot::v2::MaintenanceReport> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let report = self.store.maintain(budget, checkpoint)?;
            if !report.readers_blocking {
                return Ok(report);
            }
            if std::time::Instant::now() >= deadline {
                return Err(freespace_core::Error::generic(
                    "reader lease remained held after expected readers closed",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn sweep(&mut self) {
        for _ in 0..160 {
            let r = self.upkeep(3, false).unwrap();
            assert!(!r.readers_blocking);
            assert!(r.examined <= 3);
        }
    }
    fn accounting(&self) {
        let c = rusqlite::Connection::open(&self.db).unwrap();
        let mut actual = 0;
        for t in [
            "identities",
            "versions",
            "root_versions",
            "mount_versions",
            "generation_meta",
            "refreshes",
        ] {
            actual += c
                .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap();
        }
        assert_eq!(self.store.retained_records().unwrap(), actual as u64);
    }
}

#[test]
fn leases_block_gc_then_bounded_batches_preserve_current_prev_and_history() {
    let mut f = Fixture::new();
    let pinned = f.reader();
    let original = pinned
        .subtree(f.root.as_os_str().as_bytes(), false)
        .unwrap();
    for n in 0..20 {
        fs::write(f.root.join("local/a"), vec![1; n + 1]).unwrap();
        let transient = f.root.join(format!("local/transient-{n}"));
        fs::write(&transient, [1]).unwrap();
        if n > 0 {
            fs::remove_file(f.root.join(format!("local/transient-{}", n - 1))).unwrap();
        }
        f.store.refresh(&f.root.join("local"), 1).unwrap();
    }
    let before = f.store.retained_records().unwrap();
    let blocked = f.store.maintain(3, true).unwrap();
    assert!(blocked.readers_blocking);
    assert_eq!(blocked.examined, 0);
    assert_eq!(blocked.deleted, 0);
    assert_eq!(
        pinned
            .subtree(f.root.as_os_str().as_bytes(), false)
            .unwrap(),
        original
    );
    drop(pinned);
    let r = f.reader();
    let current = r.subtree(f.root.as_os_str().as_bytes(), false).unwrap();
    let previous = r.subtree(f.root.as_os_str().as_bytes(), true).unwrap();
    let history = r.refresh_history().unwrap();
    drop(r);
    f.sweep();
    assert!(f.store.retained_records().unwrap() < before);
    f.accounting();
    let r = f.reader();
    assert_eq!(
        r.subtree(f.root.as_os_str().as_bytes(), false).unwrap(),
        current
    );
    assert_eq!(
        r.subtree(f.root.as_os_str().as_bytes(), true).unwrap(),
        previous
    );
    assert_eq!(r.refresh_history().unwrap(), history);
    drop(r);
    let c = rusqlite::Connection::open(&f.db).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM generation_meta", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    // Two full scans retire both generations that need the long refresh history.
    f.store.scan(&f.root, 1).unwrap();
    f.store.scan(&f.root, 1).unwrap();
    f.sweep();
    f.accounting();
    assert_eq!(
        c.query_row("SELECT count(*) FROM refreshes", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let identity_count: i64 = c
        .query_row("SELECT count(*) FROM identities", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        identity_count, 4,
        "dead transient path identities reclaimed"
    );
}

#[test]
fn record_budget_rejects_publication_without_truncating_required_history() {
    let mut f = Fixture::new();
    let before = f.reader().generations;
    let count = f.store.retained_records().unwrap();
    f.store
        .configure_retention(RetentionPolicy {
            max_records: count,
            max_wal_bytes: 1 << 30,
        })
        .unwrap();
    fs::write(f.root.join("local/a"), [1, 2, 3]).unwrap();
    let error = f.store.refresh(&f.root.join("local"), 1).unwrap_err();
    assert_eq!(error.code, ErrorCode::Space);
    assert_eq!(f.reader().generations, before);
    assert_eq!(f.store.retained_records().unwrap(), count);
    f.accounting();
    f.store
        .configure_retention(RetentionPolicy {
            max_records: count + 100,
            max_wal_bytes: 1 << 30,
        })
        .unwrap();
    f.store.refresh(&f.root.join("local"), 1).unwrap();
    assert_eq!(f.reader().generations.current, before.current + 1);
}

#[test]
fn explicit_checkpoint_resets_wal_admission_and_active_readers_defer_it() {
    let mut f = Fixture::new();
    f.store
        .configure_retention(RetentionPolicy {
            max_records: 100000,
            max_wal_bytes: 1,
        })
        .unwrap();
    let pinned = f.reader();
    assert_eq!(
        f.store.refresh(&f.root.join("local"), 1).unwrap_err().code,
        ErrorCode::Space
    );
    assert!(f.store.maintain(10, true).unwrap().readers_blocking);
    drop(pinned);
    let report = f.upkeep(10, true).unwrap();
    assert!(!report.checkpoint_busy);
    assert_eq!(report.wal_bytes, 0);
    // Admission threshold allows one transaction, and blocks the next until upkeep.
    f.store.refresh(&f.root.join("local"), 1).unwrap();
    assert_eq!(
        f.store.refresh(&f.root.join("local"), 1).unwrap_err().code,
        ErrorCode::Space
    );
    assert_eq!(f.upkeep(10, true).unwrap().wal_bytes, 0);
}

#[test]
fn lease_survives_read_only_symlink_open_and_releases_after_process_exit() {
    let mut f = Fixture::new();
    let alias = f._tmp.path().join("alias.db");
    std::os::unix::fs::symlink(&f.db, &alias).unwrap();
    let reader = Reader::open(&alias).unwrap();
    assert!(f.store.maintain(1, false).unwrap().readers_blocking);
    drop(reader);
    let ready = f._tmp.path().join("ready");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "reader_lease_child", "--nocapture"])
        .env("FREESPACE_V2_LEASE_DB", &f.db)
        .env("FREESPACE_V2_LEASE_READY", &ready)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !ready.exists() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("reader child failed to acquire lease");
    }
    assert!(f.store.maintain(1, true).unwrap().readers_blocking);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!f.upkeep(1, true).unwrap().readers_blocking);
}

#[test]
fn reader_lease_child() {
    let Some(path) = std::env::var_os("FREESPACE_V2_LEASE_DB") else {
        return;
    };
    let _reader = Reader::open(&PathBuf::from(path)).unwrap();
    fs::write(
        std::env::var_os("FREESPACE_V2_LEASE_READY").unwrap(),
        b"ready",
    )
    .unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn maintenance_failure_rolls_back_pruning_cursors_and_generation_state() {
    let mut f = Fixture::new();
    f.store.refresh(&f.root.join("local"), 1).unwrap();
    f.store.scan(&f.root, 1).unwrap();
    f.store.scan(&f.root, 1).unwrap();
    let r = f.reader();
    let pair = r.generations;
    let current = r.subtree(f.root.as_os_str().as_bytes(), false).unwrap();
    let previous = r.subtree(f.root.as_os_str().as_bytes(), true).unwrap();
    let history = r.refresh_history().unwrap();
    drop(r);
    let before = f.store.retained_records().unwrap();
    let c = rusqlite::Connection::open(&f.db).unwrap();
    c.execute_batch("CREATE TRIGGER fail_gc BEFORE UPDATE ON gc_cursor WHEN OLD.kind=1 BEGIN SELECT RAISE(ABORT,'injected failure after entry pruning'); END;").unwrap();
    assert!(
        f.upkeep(1000, true)
            .unwrap_err()
            .message
            .contains("injected failure")
    );
    assert_eq!(f.store.retained_records().unwrap(), before);
    f.accounting();
    assert_eq!(
        c.query_row("SELECT sum(k1+k2+k3) FROM gc_cursor", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let r = f.reader();
    assert_eq!(r.generations, pair);
    assert_eq!(
        r.subtree(f.root.as_os_str().as_bytes(), false).unwrap(),
        current
    );
    assert_eq!(
        r.subtree(f.root.as_os_str().as_bytes(), true).unwrap(),
        previous
    );
    assert_eq!(r.refresh_history().unwrap(), history);
    drop(r);
    c.execute_batch("DROP TRIGGER fail_gc").unwrap();
    assert!(f.upkeep(1000, false).unwrap().deleted > 0);
}

#[test]
fn reported_headroom_recovers_history_capacity_through_two_scans() {
    for hold_reader in [false, true] {
        let mut f = Fixture::new();
        for _ in 0..10 {
            f.store.refresh(&f.root.join("local"), 1).unwrap();
        }
        let pinned = hold_reader.then(|| f.reader());
        let limit = f.store.retained_records().unwrap();
        f.store
            .configure_retention(RetentionPolicy {
                max_records: limit,
                max_wal_bytes: 1 << 30,
            })
            .unwrap();
        let error = f.store.refresh(&f.root.join("local"), 1).unwrap_err();
        assert_eq!(error.code, ErrorCode::Space);
        let temporary: u64 = error
            .message
            .split("Temporary recovery budget: ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(temporary > limit);
        assert!(error.fix.as_ref().unwrap().contains("2 full scan(s)"));
        f.store
            .configure_retention(RetentionPolicy {
                max_records: temporary,
                max_wal_bytes: 1 << 30,
            })
            .unwrap();
        f.store.refresh(&f.root.join("local"), 1).unwrap();
        let prior = f.reader().generations.current;
        f.store.scan(&f.root, 1).unwrap();
        assert_eq!(f.reader().generations.prev, Some(prior));
        assert_fresh_diff(&f);
        let c = rusqlite::Connection::open(&f.db).unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM refreshes", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            11
        );
        if hold_reader {
            assert!(f.store.maintain(1000, false).unwrap().readers_blocking);
        } else {
            f.sweep();
            assert_eq!(
                c.query_row("SELECT count(*) FROM refreshes", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                11
            );
        }
        f.store.scan(&f.root, 1).unwrap();
        assert!(f.store.retained_records().unwrap() <= temporary);
        if let Some(reader) = pinned {
            assert!(f.store.maintain(1000, false).unwrap().readers_blocking);
            assert_eq!(reader.refresh_history().unwrap().len(), 10);
            drop(reader);
        }
        f.sweep();
        assert_eq!(
            c.query_row("SELECT count(*) FROM refreshes", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(f.store.retained_records().unwrap() < limit);
        f.store
            .configure_retention(RetentionPolicy {
                max_records: limit,
                max_wal_bytes: 1 << 30,
            })
            .unwrap();
        f.store.refresh(&f.root.join("local"), 1).unwrap();
        f.accounting();
    }
}

fn assert_fresh_diff(f: &Fixture) {
    let first = f.reader();
    let diff = freespace_core::queries::v2::diff(
        &first,
        &freespace_core::queries::params::DiffParams {
            path: Some(f.root.to_string_lossy().into_owned()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!diff.partial);
    assert!(diff.refreshed.is_empty());
    drop(first);
}

#[test]
fn initial_seed_budget_failure_reports_space_and_preserves_unpublished_state() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a"), [1]).unwrap();
    let db = tmp.path().join("v2.db");
    let mut store = Store::open(&db).unwrap();
    store
        .configure_retention(RetentionPolicy {
            max_records: 1,
            max_wal_bytes: 1 << 30,
        })
        .unwrap();
    let error = store.scan(&root, 1).unwrap_err();
    assert_eq!(error.code, ErrorCode::Space);
    assert!(error.message.contains("publication requires"));
    assert!(error.fix.as_ref().unwrap().contains("retry this operation"));
    let temporary: u64 = error
        .message
        .split("Temporary recovery budget: ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(store.retained_records().unwrap(), 0);
    let c = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM control", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM generation_meta", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(matches!(Reader::open(&db),Err(e) if e.code==ErrorCode::NoSnapshot));
    store
        .configure_retention(RetentionPolicy {
            max_records: temporary,
            max_wal_bytes: 1 << 30,
        })
        .unwrap();
    store.scan(&root, 1).unwrap();
    let r = Reader::open(&db).unwrap();
    assert_eq!(r.generations.current, 1);
    assert_eq!(r.generations.prev, None);
}

#[test]
fn empty_and_missing_stores_are_distinct_from_foreign_or_malformed_databases() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("empty.db");
    let _store = Store::open(&db).unwrap();
    assert!(matches!(Reader::open(&db),Err(e) if e.code==ErrorCode::NoSnapshot));
    assert!(
        matches!(Reader::open(&tmp.path().join("missing.db")),Err(e) if e.code==ErrorCode::NoSnapshot)
    );
    let foreign = tmp.path().join("foreign.db");
    rusqlite::Connection::open(&foreign)
        .unwrap()
        .execute_batch("CREATE TABLE other(value INTEGER)")
        .unwrap();
    assert!(matches!(Reader::open(&foreign),Err(e) if e.code==ErrorCode::Schema));
    let malformed = tmp.path().join("malformed.db");
    fs::write(&malformed, b"this is not sqlite").unwrap();
    assert!(matches!(Reader::open(&malformed),Err(e) if e.code==ErrorCode::Schema));
}

#[test]
fn store_and_sidecars_match_v1_modes_under_permissive_umask() {
    let tmp = tempfile::tempdir().unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "store_mode_child", "--nocapture"])
        .env("FREESPACE_V2_MODE_DIR", tmp.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn store_mode_child() {
    use freespace_core::snapshot::fsutil;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Some(dir) = std::env::var_os("FREESPACE_V2_MODE_DIR") else {
        return;
    };
    // This helper runs alone in a child process, so umask cannot race other tests.
    unsafe {
        libc::umask(0);
    }
    let dir = PathBuf::from(dir);
    let root = dir.join("root");
    fs::create_dir(&root).unwrap();
    let state = dir.join("state");
    let db = state.join("generations.db");
    let mut store = Store::open(&db).unwrap();
    store.scan(&root, 1).unwrap();
    let reader = Reader::open(&db).unwrap();
    drop(reader);
    drop(store);
    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        fsutil::DIR_MODE
    );
    for suffix in ["", ".readers", "-wal", "-shm"] {
        let mut path = db.as_os_str().to_os_string();
        path.push(suffix);
        let metadata = fs::metadata(path).unwrap();
        assert_eq!(
            metadata.permissions().mode() & 0o777,
            fsutil::FILE_MODE,
            "{suffix}"
        );
        if suffix == "-wal" {
            assert_eq!(
                metadata.len(),
                0,
                "last-close checkpoint leaves an empty persistent WAL"
            );
        }
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        if fsutil::is_root()
            && let Some(gid) = fsutil::group_gid(fsutil::GROUP)
        {
            assert_eq!(metadata.gid(), gid, "{suffix}");
        }
    }
    fs::set_permissions(&state, fs::Permissions::from_mode(0o550)).unwrap();
    let reader = Reader::open(&db).unwrap();
    assert_eq!(reader.generations.current, 1);
    drop(reader);
    fs::set_permissions(&state, fs::Permissions::from_mode(fsutil::DIR_MODE)).unwrap();
}
