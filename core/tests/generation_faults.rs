//! Linux syscall-boundary verification. Requires a C compiler and dynamic glibc.
//! ENOSPC is injected at pwrite/pwrite64, not simulated with max_page_count and
//! not physical disk exhaustion. No fault hooks or production changes needed.
#![cfg(all(
    target_os = "linux",
    target_env = "gnu",
    not(target_feature = "crt-static")
))]

use freespace_core::snapshot::v2::{Reader, Store};
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::{ffi::OsStrExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{OnceLock, mpsc},
    time::Duration,
};

fn shim() -> &'static Path {
    static SHIM: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    &SHIM
        .get_or_init(|| {
            let dir = tempfile::tempdir().unwrap();
            let lib = dir.path().join("enospc.so");
            let output = Command::new("cc")
                .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", "-o"])
                .arg(&lib)
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/generation_fault_helpers/enospc.c"),
                )
                .arg("-ldl")
                .output()
                .expect("Linux fault tests require cc");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            (dir, lib)
        })
        .1
}

fn read_events(stdout: std::process::ChildStdout, send: mpsc::Sender<&'static str>) {
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        if line.contains("GENFAULT_HIT") {
            let _ = send.send("hit");
        }
        if line.contains("GENFAULT_RETURNED") {
            let _ = send.send("returned");
        }
    }
}

struct Fixture {
    tmp: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
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
        for n in 2..6 {
            fs::write(root.join("local/a"), vec![1; n]).unwrap();
            store.refresh(&root.join("local"), 1).unwrap();
        }
        // Retire some history, retaining nonempty current refresh history too.
        store.scan(&root, 1).unwrap();
        store.refresh(&root.join("local"), 1).unwrap();
        drop(store);
        fs::write(root.join("local/new"), [2; 32]).unwrap();
        Self { tmp, root, db }
    }
    fn visible(&self) -> String {
        let r = Reader::open(&self.db).unwrap();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            r.generations,
            r.subtree(self.root.as_os_str().as_bytes(), false).unwrap(),
            r.subtree(self.root.as_os_str().as_bytes(), true).unwrap(),
            r.meta(false).unwrap(),
            r.meta(true).unwrap(),
            r.roots(false).unwrap(),
            r.roots(true).unwrap(),
            r.refresh_history().unwrap()
        )
    }
    fn internal(&self) -> Vec<Vec<String>> {
        let c = rusqlite::Connection::open(&self.db).unwrap();
        [
            "control",
            "identities",
            "live",
            "versions",
            "live_roots",
            "root_versions",
            "live_mounts",
            "mount_versions",
            "generation_meta",
            "refreshes",
            "retention",
            "storage_stats",
            "gc_cursor",
        ]
        .iter()
        .map(|table| {
            let mut q = c.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let columns = q.column_count();
            let mut rows = q
                .query_map([], |r| {
                    Ok((0..columns)
                        .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort();
            rows
        })
        .collect()
    }
    fn accounting(&self) {
        let store = Store::open(&self.db).unwrap();
        let c = rusqlite::Connection::open(&self.db).unwrap();
        assert_eq!(
            c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        let mut actual = 0i64;
        for table in [
            "identities",
            "versions",
            "root_versions",
            "mount_versions",
            "generation_meta",
            "refreshes",
        ] {
            actual += c
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap();
        }
        assert_eq!(store.retained_records().unwrap(), actual as u64);
    }
    fn run(&self, operation: &str, target: &str, mode: &str, at: usize) {
        let target_path = if target == "wal" {
            PathBuf::from(format!("{}-wal", self.db.display()))
        } else if target == "stage" {
            PathBuf::from("stage")
        } else {
            self.db.clone()
        };
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "generation_fault_child", "--nocapture"])
            .env("LD_PRELOAD", shim())
            .env("GENFAULT_DIR", self.tmp.path())
            .env("GENFAULT_TARGET", target_path)
            .env("GENFAULT_STAGE_DIR", self.tmp.path())
            .env("GENFAULT_ARM", self.tmp.path().join("armed"))
            .env("GENFAULT_MODE", mode)
            .env("GENFAULT_AT", at.to_string())
            .env("GENFAULT_OPERATION", operation)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, recv) = mpsc::channel();
        let reader = std::thread::spawn(move || read_events(stdout, send));
        let event = recv.recv_timeout(Duration::from_secs(20));
        if event != Ok("hit") {
            let _ = child.kill();
            let _ = child.wait();
            reader.join().unwrap();
            panic!("{operation}/{target}/{mode}/{at}: fault was not reached: {event:?}");
        }
        if mode == "kill" {
            child.kill().unwrap();
            assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        } else {
            let returned = recv.recv_timeout(Duration::from_secs(20));
            if returned != Ok("returned") {
                let _ = child.kill();
            }
            let status = child.wait().unwrap();
            assert_eq!(returned, Ok("returned"), "failure must return, not hang");
            assert!(status.success(), "{operation}/{target}/{at}: {status}");
        }
        reader.join().unwrap();
    }
    fn stages(&self) -> Vec<PathBuf> {
        let mut paths: Vec<_> = fs::read_dir(self.tmp.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .as_encoded_bytes()
                    .starts_with(b"scan-")
                    && p.file_name()
                        .unwrap()
                        .as_encoded_bytes()
                        .ends_with(b".stage.tmp")
            })
            .collect();
        paths.sort();
        paths
    }
    fn retry_low_memory(&self) {
        let before = Reader::open(&self.db).unwrap().generations.current;
        let mut store = Store::open(&self.db).unwrap();
        store
            .scan_roots_with_options(std::slice::from_ref(&self.root), true, 1, true, true)
            .unwrap();
        assert_eq!(
            Reader::open(&self.db).unwrap().generations.current,
            before + 1
        );
        drop(store);
        self.accounting();
    }
    fn retry(&self) {
        let before = Reader::open(&self.db).unwrap().generations.current;
        let mut store = Store::open(&self.db).unwrap();
        store.refresh(&self.root.join("local"), 1).unwrap();
        assert_eq!(
            Reader::open(&self.db).unwrap().generations.current,
            before + 1
        );
        assert!(!store.maintain(1000, true).unwrap().readers_blocking);
        drop(store);
        self.accounting();
    }
}

#[test]
fn writer_kill_before_and_during_commit_preserves_generation_pair() {
    // First WAL write: before any commit bytes. Third: after header/frame bytes.
    for operation in ["scan", "refresh"] {
        for at in [1, 3] {
            let f = Fixture::new();
            let before = f.visible();
            let internal = f.internal();
            f.run(operation, "wal", "kill", at);
            assert_eq!(f.visible(), before);
            assert_eq!(f.internal(), internal);
            f.accounting();
            f.retry();
        }
    }
}

#[test]
fn maintenance_kill_during_pruning_and_checkpoint_preserves_readable_state() {
    for target in ["wal", "db"] {
        let f = Fixture::new();
        let before = f.visible();
        let internal = f.internal();
        f.run("maintain", target, "kill", 3);
        assert_eq!(f.visible(), before);
        if target == "wal" {
            assert_eq!(f.internal(), internal);
        }
        f.accounting();
        f.retry();
    }
}

#[test]
fn syscall_enospc_in_wal_commit_and_database_checkpoint_preserves_state() {
    for (operation, target) in [
        ("scan", "wal"),
        ("refresh", "wal"),
        ("maintain", "wal"),
        ("maintain", "db"),
    ] {
        for at in [1, 3] {
            let f = Fixture::new();
            let before = f.visible();
            let internal = f.internal();
            f.run(operation, target, "enospc", at);
            assert_eq!(f.visible(), before);
            if target == "wal" {
                assert_eq!(f.internal(), internal);
            }
            f.accounting();
            f.retry();
        }
    }
}

#[test]
fn low_memory_stage_and_publication_faults_preserve_state_and_cleanup_on_retry() {
    let mut leaked = Vec::new();
    for target in ["stage", "wal"] {
        for mode in ["kill", "enospc"] {
            for at in [1, 3] {
                leaked.extend(low_memory_fault(target, mode, at));
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "interrupted stage files survived retry: {leaked:?}"
    );
}

fn low_memory_fault(target: &str, mode: &str, at: usize) -> Vec<String> {
    let f = Fixture::new();
    let before = f.visible();
    let internal = f.internal();
    f.run("low_memory", target, mode, at);
    assert_eq!(f.visible(), before, "{target}/{mode}/{at}");
    assert_eq!(f.internal(), internal, "{target}/{mode}/{at}");
    f.accounting();
    if mode == "enospc" {
        assert!(
            f.stages().is_empty(),
            "normal error cleanup: {:?}",
            f.stages()
        );
    }
    f.retry_low_memory();
    f.stages()
        .iter()
        .map(|p| format!("{target}/{mode}/{at}: {}", p.display()))
        .collect()
}

#[test]
fn generation_fault_child() {
    let Some(dir) = std::env::var_os("GENFAULT_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let mut store = Store::open(&dir.join("v2.db")).unwrap();
    let operation = std::env::var("GENFAULT_OPERATION").unwrap();
    fs::write(dir.join("armed"), b"armed").unwrap();
    let result = if operation == "refresh" {
        store.refresh(&dir.join("root/local"), 1).map(|_| ())
    } else if operation == "low_memory" {
        store
            .scan_roots_with_options(&[dir.join("root")], true, 1, true, true)
            .map(|_| ())
    } else if operation == "scan" {
        store.scan(&dir.join("root"), 1).map(|_| ())
    } else {
        store.maintain(1000, true).map(|r| {
            assert!(!r.readers_blocking);
        })
    };
    let error = result.expect_err("injected write failure must surface");
    assert_eq!(error.code, freespace_core::ErrorCode::Space, "{error:?}");
    println!("GENFAULT_RETURNED");
}
