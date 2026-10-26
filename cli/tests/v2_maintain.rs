mod common;
use common::Env;
use freespace_core::snapshot::{
    SnapshotDir, lock,
    v2::{Reader, Store},
};
use serde_json::Value;
use std::{fs, os::unix::ffi::OsStrExt};

struct Fixture {
    env: Env,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let env = Env::new();
        fs::create_dir_all(env.root().join("local")).unwrap();
        fs::write(env.root().join("local/a"), [1]).unwrap();
        fs::create_dir_all(env.snap().join("v2")).unwrap();
        let mut store = Store::open(&env.snap().join("v2/generations.db")).unwrap();
        for _ in 0..3 {
            store.scan(&env.root(), 1).unwrap();
        }
        Self { env, store }
    }
    fn reader(&self) -> Reader {
        Reader::open(&self.env.snap().join("v2/generations.db")).unwrap()
    }
    fn maintain(&self, args: &[&str]) -> Value {
        let mut all = vec!["--backend", "v2", "maintain", "--json"];
        all.extend_from_slice(args);
        let out = self.env.run(&all);
        assert!(
            out.status.success(),
            "stdout: {} stderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
}

#[test]
fn reader_deferral_reports_saved_policy_and_later_batches_preserve_history() {
    let mut f = Fixture::new();
    for n in 0..6 {
        fs::write(f.env.root().join("local/a"), vec![1; n + 2]).unwrap();
        f.store.refresh(&f.env.root().join("local"), 1).unwrap();
    }
    let pin = f.reader();
    let current = pin
        .subtree(f.env.root().as_os_str().as_bytes(), false)
        .unwrap();
    let previous = pin
        .subtree(f.env.root().as_os_str().as_bytes(), true)
        .unwrap();
    let history = pin.refresh_history().unwrap();
    let generations = pin.generations;
    let blocked = f.maintain(&[
        "--budget",
        "3",
        "--checkpoint",
        "--max-records",
        "100000",
        "--max-wal-bytes",
        "1073741824",
    ]);
    assert_eq!(blocked["readers_blocking"], true);
    assert_eq!(blocked["examined"], 0);
    assert_eq!(blocked["deleted"], 0);
    assert_eq!(blocked["policy_updated"], true);
    assert_eq!(blocked["max_records"], 100000);
    assert_eq!(f.store.retention_policy().unwrap().max_records, 100000);
    drop(pin);
    let before = f.store.retained_records().unwrap();
    for _ in 0..80 {
        let report = f.maintain(&["--budget", "3"]);
        assert_eq!(report["readers_blocking"], false);
        assert!(report["examined"].as_u64().unwrap() <= 3);
        assert!(report["deleted"].as_u64().unwrap() <= 3);
        assert_eq!(report["generation"], generations.current);
        assert_eq!(report["prev_generation"], generations.prev.unwrap());
    }
    assert!(f.store.retained_records().unwrap() < before);
    let pin = f.reader();
    assert_eq!(
        pin.subtree(f.env.root().as_os_str().as_bytes(), false)
            .unwrap(),
        current
    );
    assert_eq!(
        pin.subtree(f.env.root().as_os_str().as_bytes(), true)
            .unwrap(),
        previous
    );
    assert_eq!(pin.refresh_history().unwrap(), history);
    drop(pin);
    let checkpoint = f.maintain(&["--budget", "100", "--checkpoint"]);
    assert_eq!(checkpoint["checkpoint_requested"], true);
    assert_eq!(checkpoint["checkpoint_busy"], false);
    assert!(checkpoint["wal_bytes"].is_u64());
}

#[test]
fn invalid_arguments_and_writer_lock_do_not_change_policy_or_generations() {
    let f = Fixture::new();
    let original = f.store.retention_policy().unwrap();
    let original_rows = f.store.retained_records().unwrap();
    for args in [
        vec!["--budget", "0", "--max-records", "1"],
        vec!["--budget", "100001", "--max-records", "1"],
        vec!["--max-records", "0"],
        vec!["--max-wal-bytes", "9223372036854775808"],
    ] {
        let mut all = vec!["--backend", "v2", "maintain", "--json"];
        all.extend(args);
        let out = f.env.run(&all);
        assert!(!out.status.success());
        assert_eq!(
            f.store.retention_policy().unwrap().max_records,
            original.max_records
        );
        assert_eq!(f.store.retained_records().unwrap(), original_rows);
        assert_eq!(f.reader().generations.current, 3);
    }
    let dir = SnapshotDir::new(f.env.snap().join("v2"));
    let _lock = lock::acquire(&dir, "scan", "/test").unwrap();
    let out = f.env.run(&[
        "--backend",
        "v2",
        "maintain",
        "--max-records",
        "1",
        "--json",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("LOCK"));
    assert_eq!(
        f.store.retention_policy().unwrap().max_records,
        original.max_records
    );
}

#[test]
fn explicit_policy_recovery_preserves_history_after_admission_failure() {
    let mut f = Fixture::new();
    let initial = f.reader().generations;
    let report = f.maintain(&["--max-records", "1"]);
    assert_eq!(report["max_records"], 1);
    let error = f.store.refresh(&f.env.root().join("local"), 1).unwrap_err();
    assert_eq!(error.code, freespace_core::ErrorCode::Space);
    assert_eq!(f.reader().generations, initial);
    let failed = f.env.run(&[
        "--backend",
        "v2",
        "maintain",
        "--max-wal-bytes",
        "1",
        "--json",
    ]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("retention policy saved"));
    assert_eq!(f.store.retention_policy().unwrap().max_wal_bytes, 1);
    assert_eq!(f.reader().generations, initial);
    f.maintain(&[
        "--max-records",
        "100000",
        "--max-wal-bytes",
        "1073741824",
        "--checkpoint",
    ]);
    f.store.refresh(&f.env.root().join("local"), 1).unwrap();
    let reader = f.reader();
    assert_eq!(reader.generations.current, initial.current + 1);
    assert_eq!(reader.generations.prev, initial.prev);
    assert_eq!(reader.refresh_history().unwrap().len(), 1);
}

#[test]
fn backend_and_missing_store_preflights_are_read_only() {
    let env = Env::new();
    for args in [
        vec!["maintain", "--json"],
        vec!["--backend", "v2", "maintain", "--json"],
    ] {
        let out = env.run(&args);
        assert!(!out.status.success());
        let body: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            body["error"]["code"],
            if args[0] == "maintain" {
                "NOT_SUPPORTED"
            } else {
                "NO_SNAPSHOT"
            }
        );
        assert!(!env.snap().exists());
    }
    fs::create_dir_all(env.snap().join("v2")).unwrap();
    let database = env.snap().join("v2/generations.db");
    fs::write(&database, b"foreign bytes").unwrap();
    let out = env.run(&[
        "--backend",
        "v2",
        "maintain",
        "--max-records",
        "1",
        "--json",
    ]);
    assert!(!out.status.success());
    assert_eq!(fs::read(database).unwrap(), b"foreign bytes");
    assert!(!env.snap().join("v2/.lock").exists());
}

#[test]
fn text_output_explains_deferred_cleanup_and_saved_configuration() {
    let f = Fixture::new();
    let _pin = f.reader();
    fs::write(f.env.config(), "backend = \"v2\"\n").unwrap();
    let out = f.env.run(&["maintain", "--max-records", "100000"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("deferred: readers active"));
    assert!(text.contains("max_records=100000"));
    assert!(text.contains("saved; applies even when cleanup is deferred"));
}

#[test]
fn checkpoint_busy_is_distinct_from_reader_lease_deferral() {
    let mut f = Fixture::new();
    let observer = rusqlite::Connection::open(f.env.snap().join("v2/generations.db")).unwrap();
    observer
        .execute_batch("BEGIN; SELECT current FROM control;")
        .unwrap();
    f.store.refresh(&f.env.root().join("local"), 1).unwrap();
    let report = f.maintain(&["--checkpoint"]);
    assert_eq!(report["readers_blocking"], false);
    assert_eq!(report["checkpoint_busy"], true);
    drop(observer);
    let report = f.maintain(&["--checkpoint"]);
    assert_eq!(report["checkpoint_busy"], false);
}

#[test]
fn maintain_lock_hides_stale_scan_progress() {
    let f = Fixture::new();
    let dir = SnapshotDir::new(f.env.snap().join("v2"));
    let stale = freespace_core::snapshot::progress::Progress {
        op: "scan".into(),
        path: "/stale".into(),
        stage: "walk".into(),
        started_at: 1,
        updated_at: 2,
        elapsed_seconds: 1,
        entries_seen: 17,
        entries_expected: Some(100),
        percent: Some(17),
    };
    fs::write(dir.progress(), serde_json::to_vec(&stale).unwrap()).unwrap();
    // Hold the exact operation lock payload deterministically; maintenance
    // itself can finish or defer before a status subprocess gets scheduled.
    let _running = lock::acquire(&dir, "maintain", "generations.db").unwrap();
    let out = f.env.run(&["--backend", "v2", "status", "--json"]);
    assert!(out.status.success());
    let status: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(status["scanning"]["op"], "maintain");
    assert_eq!(status["progress"], Value::Null);
}

#[test]
fn empty_store_policy_recovery_allows_the_first_scan_to_publish() {
    use freespace_core::snapshot::v2::RetentionPolicy;
    let env = Env::new();
    fs::create_dir_all(env.root()).unwrap();
    fs::write(env.root().join("a"), [1]).unwrap();
    fs::create_dir_all(env.snap().join("v2")).unwrap();
    let database = env.snap().join("v2/generations.db");
    let mut store = Store::open(&database).unwrap();
    store
        .configure_retention(RetentionPolicy {
            max_records: 1,
            max_wal_bytes: 1073741824,
        })
        .unwrap();
    let failed = env.run(&[
        "--backend",
        "v2",
        "scan",
        env.root().to_str().unwrap(),
        "--json",
    ]);
    assert!(!failed.status.success());
    let error: Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(error["error"]["code"], "SPACE");
    assert_eq!(
        Reader::open(&database).err().unwrap().code,
        freespace_core::ErrorCode::NoSnapshot
    );
    let maintained = env.run(&[
        "--backend",
        "v2",
        "maintain",
        "--max-records",
        "100000",
        "--json",
    ]);
    assert!(
        maintained.status.success(),
        "{}",
        String::from_utf8_lossy(&maintained.stdout)
    );
    let report: Value = serde_json::from_slice(&maintained.stdout).unwrap();
    assert_eq!(report["generation"], Value::Null);
    assert_eq!(report["prev_generation"], Value::Null);
    assert_eq!(report["max_records"], 100000);
    assert_eq!(store.retention_policy().unwrap().max_records, 100000);
    let scanned = env.run(&[
        "--backend",
        "v2",
        "scan",
        env.root().to_str().unwrap(),
        "--json",
    ]);
    assert!(
        scanned.status.success(),
        "{}",
        String::from_utf8_lossy(&scanned.stdout)
    );
    assert_eq!(Reader::open(&database).unwrap().generations.current, 1);
}
