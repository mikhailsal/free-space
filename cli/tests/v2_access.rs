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
