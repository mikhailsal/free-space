mod common;
use common::Env;
use freespace_core::{ErrorCode, snapshot::v2::Reader};
use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn unreadable_v2_files_preserve_the_snapshot_access_contract() {
    // Root bypasses DAC permissions; exercise the real reader as an ordinary user.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let e = Env::new();
    fs::create_dir(e.root()).unwrap();
    fs::write(e.root().join("a"), b"data").unwrap();
    let root = e.root();
    for _ in 0..2 {
        assert!(
            e.run(&["--backend", "v2", "scan", root.to_str().unwrap(), "--json"])
                .status
                .success()
        );
    }
    let dir = e.snap().join("v2");
    let db = dir.join("generations.db");
    for name in [
        "generations.db",
        "generations.db.readers",
        "generations.db-wal",
        "generations.db-shm",
        ".",
    ] {
        let path = if name == "." {
            dir.clone()
        } else {
            dir.join(name)
        };
        let permissions = fs::metadata(&path).unwrap().permissions();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let error = Reader::open(&db)
            .err()
            .expect("reader must reject denied access");
        let outputs: Vec<_> = ["status", "ls", "top", "find", "diff"]
            .map(|command| e.run(&["--backend", "v2", command, "--json"]))
            .into();
        fs::set_permissions(&path, permissions).unwrap();
        assert_eq!(error.code, ErrorCode::SnapshotAccess, "{name}: {error:?}");
        assert_eq!(error.code.http_status(), 403);
        assert!(
            error
                .fix
                .as_deref()
                .unwrap()
                .contains("--backend v2 install-timer")
        );
        for output in outputs {
            assert_eq!(output.status.code(), Some(9), "{name}: {output:?}");
            let body: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(body["error"]["code"], "SNAPSHOT_ACCESS");
        }
    }
    assert_eq!(Reader::open(&db).unwrap().generations.current, 2);
}

#[test]
fn invalid_store_with_fifo_lease_returns_schema_error_without_hanging() {
    use std::{ffi::CString, process::Stdio, time::Duration};
    for published in [false, true] {
        let e = Env::new();
        let dir = e.snap().join("v2");
        fs::create_dir_all(&dir).unwrap();
        if published {
            fs::create_dir(e.root()).unwrap();
            assert!(
                e.run(&[
                    "--backend",
                    "v2",
                    "scan",
                    e.root().to_str().unwrap(),
                    "--json"
                ])
                .status
                .success()
            );
            fs::remove_file(dir.join("generations.db.readers")).unwrap();
        } else {
            fs::write(dir.join("generations.db"), b"not a SQLite database").unwrap();
        }
        let fifo = CString::new(
            dir.join("generations.db.readers")
                .as_os_str()
                .as_encoded_bytes(),
        )
        .unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let mut child = e
            .cmd(&["--backend", "v2", "status", "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut completed = false;
        // gstack-timing-ok: bound a potentially blocked owned child; assert its error contract, not latency.
        for _ in 0..500 {
            if child.try_wait().unwrap().is_some() {
                completed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !completed {
            child.kill().unwrap();
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(5), "{output:?}");
        let body: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["error"]["code"], "SCHEMA");
    }
}
