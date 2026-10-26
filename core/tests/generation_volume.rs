//! Physical ENOSPC on bounded, private-namespace tmpfs; no host disk is filled.
//! Requires dynamic glibc on Linux, cc, passwordless sudo, and mount namespaces.
//! The barrier uses LD_PRELOAD, which static release test binaries cannot load.
//! Missing namespace
//! privilege produces an explicit skip; once available, every assertion is required.
#![cfg(all(
    target_os = "linux",
    target_env = "gnu",
    not(target_feature = "crt-static")
))]
use freespace_core::snapshot::v2::{Reader, Store};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

#[test]
fn real_bounded_volume_enospc() {
    let probe = Command::new("sudo")
        .args([
            "-n",
            "unshare",
            "--mount",
            "--propagation",
            "private",
            "/bin/true",
        ])
        .output();
    if !probe.is_ok_and(|o| o.status.success()) {
        eprintln!(
            "SKIP real volume ENOSPC: passwordless sudo and private mount namespaces required"
        );
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let shim = tmp.path().join("barrier.so");
    let output = Command::new("cc")
        .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&shim)
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/generation_volume_helpers/barrier.c"),
        )
        .arg("-ldl")
        .output()
        .expect("cc required for syscall barrier");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let script = tmp.path().join("private-volume.sh");
    fs::write(
        &script,
        include_str!("generation_volume_helpers/private-volume.sh"),
    )
    .unwrap();
    let output = Command::new("sudo")
        .args([
            "-n",
            "unshare",
            "--mount",
            "--propagation",
            "private",
            "/bin/bash",
        ])
        .arg(script)
        .arg(std::env::current_exe().unwrap())
        .arg(tmp.path())
        .arg(shim)
        .arg(fs::read_link("/proc/self/ns/mnt").unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "private volume failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let proof = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        proof.matches("REAL_VOLUME_CASE_PASSED").count(),
        3,
        "{proof}"
    );
    println!("{proof}");
}

fn visible(db: &Path, root: &Path) -> String {
    let r = Reader::open(db).unwrap();
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        r.generations,
        r.subtree(root.as_os_str().as_bytes(), false).unwrap(),
        r.subtree(root.as_os_str().as_bytes(), true).unwrap(),
        r.meta(false).unwrap(),
        r.meta(true).unwrap(),
        r.roots(false).unwrap(),
        r.roots(true).unwrap(),
        r.refresh_history().unwrap()
    )
}
fn accounting(db: &Path) {
    let s = Store::open(db).unwrap();
    let c = rusqlite::Connection::open(db).unwrap();
    assert_eq!(
        c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let mut count = 0i64;
    for t in [
        "identities",
        "versions",
        "root_versions",
        "mount_versions",
        "generation_meta",
        "refreshes",
    ] {
        count += c
            .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
    }
    assert_eq!(s.retained_records().unwrap(), count as u64);
}
fn events(out: std::process::ChildStdout, tx: mpsc::Sender<String>) {
    for line in BufReader::new(out).lines() {
        let _ = tx.send(line.unwrap());
    }
}
fn await_event(rx: &mpsc::Receiver<String>, event: &str, child: &mut std::process::Child) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(line) if line.contains(event) => return,
            Ok(_) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("missing {event}: {error}");
            }
        }
    }
}
fn fill_private_tmpfs(dir: &Path) -> u64 {
    assert_eq!(
        fs::read_link("/proc/self/ns/mnt").unwrap().as_os_str(),
        std::env::var_os("VOLUME_NAMESPACE").unwrap()
    );
    assert_ne!(
        std::env::var_os("VOLUME_NAMESPACE"),
        std::env::var_os("VOLUME_CALLER_NAMESPACE")
    );
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).unwrap();
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    assert_eq!(unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) }, 0);
    let info = unsafe { info.assume_init() };
    assert_eq!(info.f_type, 0x01021994, "must be tmpfs");
    assert!(
        info.f_blocks * info.f_bsize as u64 <= 768 * 1024 * 1024,
        "volume must be bounded"
    );
    let mut file = fs::File::create(dir.join("filler")).unwrap();
    let block = vec![0u8; 1024 * 1024];
    let mut written = 0u64;
    loop {
        match file.write(&block) {
            Ok(0) => panic!("filler made no progress"),
            Ok(n) => {
                written += n as u64;
                assert!(written <= 768 * 1024 * 1024);
            }
            Err(e) => {
                assert_eq!(e.raw_os_error(), Some(libc::ENOSPC));
                break;
            }
        }
    }
    written
}
fn volume_case(volume: &Path, operation: &str) {
    let dir = volume.join(operation);
    fs::create_dir(&dir).unwrap();
    let root = dir.join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a"), [1]).unwrap();
    let db = dir.join("v2.db");
    let mut store = Store::open(&db).unwrap();
    store.scan(&root, 1).unwrap();
    store.scan(&root, 1).unwrap();
    store.refresh(&root, 1).unwrap();
    drop(store);
    let old_db_size = fs::metadata(&db).unwrap().len();
    let mut held = Store::open(&db).unwrap();
    // Keep this connection open so checkpoint must extend the old database file.
    for n in 0..600 {
        fs::write(root.join(format!("new-{n}")), [2]).unwrap();
    }
    if operation == "checkpoint" {
        held.scan(&root, 1).unwrap();
    }
    let before = visible(&db, &root);
    let target = match operation {
        "stage" => PathBuf::from("stage"),
        "checkpoint" => db.clone(),
        _ => PathBuf::from(format!("{}-wal", db.display())),
    };
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "real_volume_writer_child", "--nocapture"])
        .env("LD_PRELOAD", std::env::var_os("VOLUME_SHIM").unwrap())
        .env("VOLUME_DIR", &dir)
        .env("VOLUME_ARM", dir.join("armed"))
        .env("VOLUME_TARGET", target)
        .env(
            "VOLUME_MIN_OFFSET",
            if operation == "checkpoint" {
                old_db_size
            } else {
                0
            }
            .to_string(),
        )
        .env("VOLUME_OPERATION", operation)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let out = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || events(out, tx));
    await_event(&rx, "VOLUME_READY", &mut child);
    let filled = fill_private_tmpfs(volume);
    child.stdin.take().unwrap().write_all(b"x").unwrap();
    await_event(&rx, "REAL_ENOSPC", &mut child);
    await_event(&rx, "VOLUME_SPACE", &mut child);
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();
    fs::remove_file(volume.join("filler")).unwrap();
    assert_eq!(visible(&db, &root), before, "{operation}");
    accounting(&db);
    let generation = Reader::open(&db).unwrap().generations.current;
    held.scan_roots_with_options(std::slice::from_ref(&root), true, 1, true, true)
        .unwrap();
    assert_eq!(
        Reader::open(&db).unwrap().generations.current,
        generation + 1
    );
    assert!(!held.maintain(1000, true).unwrap().readers_blocking);
    drop(held);
    accounting(&db);
    println!(
        "REAL_VOLUME_CASE_PASSED {operation}: filler={filled} bytes; real pwrite ENOSPC, Space, coherent pair/history/accounting, retry"
    );
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn real_volume_namespace_child() {
    let Some(volume) = std::env::var_os("VOLUME_ROOT") else {
        return;
    };
    for operation in ["stage", "wal", "checkpoint"] {
        volume_case(Path::new(&volume), operation);
    }
}
#[test]
fn real_volume_writer_child() {
    let Some(dir) = std::env::var_os("VOLUME_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let mut store = Store::open(&dir.join("v2.db")).unwrap();
    fs::write(dir.join("armed"), b"armed").unwrap();
    let result = match std::env::var("VOLUME_OPERATION").unwrap().as_str() {
        "stage" | "wal" => store
            .scan_roots_with_options(&[dir.join("root")], true, 1, true, true)
            .map(|_| ()),
        _ => store.maintain(1, true).map(|_| ()),
    };
    let error = result.expect_err("real filesystem exhaustion must fail");
    assert_eq!(error.code, freespace_core::ErrorCode::Space, "{error:?}");
    println!("VOLUME_SPACE");
}
