use freespace_core::snapshot::v2::{Reader, RetentionPolicy, Store};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub struct Fixture {
    pub _tmp: tempfile::TempDir,
    pub root: PathBuf,
    pub db: PathBuf,
    pub store: Store,
}
impl Fixture {
    pub fn new(width: usize) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("tree");
        fs::create_dir_all(root.join("local")).unwrap();
        fs::create_dir_all(root.join("other")).unwrap();
        fs::write(root.join("local/a"), [1]).unwrap();
        for n in 0..width {
            fs::write(root.join(format!("other/{n:04}")), [1]).unwrap();
        }
        let db = tmp.path().join("v2.db");
        let mut store = Store::open(&db).unwrap();
        store
            .configure_retention(RetentionPolicy {
                max_records: 20000,
                max_wal_bytes: 16 * 1024 * 1024,
            })
            .unwrap();
        store.scan(&root, 1).unwrap();
        store.scan(&root, 1).unwrap();
        Self {
            _tmp: tmp,
            root,
            db,
            store,
        }
    }
    pub fn reader(&self) -> Reader {
        Reader::open(&self.db).unwrap()
    }
    pub fn measured_refresh(&mut self) -> (u64, u64) {
        let steps = Arc::new(AtomicU64::new(0));
        self.store.observe_vm_steps(Some(steps.clone())).unwrap();
        let report = self.store.refresh(&self.root.join("local"), 1).unwrap();
        self.store.observe_vm_steps(None).unwrap();
        assert_eq!(report.ancestors, 1);
        assert!(report.old_subtree_entries <= 3 && report.new_subtree_entries <= 3);
        (report.sql_changes, steps.load(Ordering::Relaxed))
    }
    pub fn upkeep(
        &mut self,
        budget: usize,
        checkpoint: bool,
    ) -> freespace_core::snapshot::v2::MaintenanceReport {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let report = self.store.maintain(budget, checkpoint).unwrap();
            if !report.readers_blocking {
                assert!(report.examined <= budget);
                assert!(report.deleted <= budget);
                return report;
            }
            assert!(
                Instant::now() < deadline,
                "unexpected reader lease remained held"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    pub fn integrity_and_accounting(&self) -> u64 {
        let conn = rusqlite::Connection::open(&self.db).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        let failures: i64 = conn
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(failures, 0);
        let mut actual = 0;
        for table in [
            "identities",
            "version_rows",
            "root_versions",
            "mount_versions",
            "generation_meta",
            "refreshes",
        ] {
            actual += conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap() as u64;
        }
        assert_eq!(self.store.retained_records().unwrap(), actual);
        actual
    }
}
