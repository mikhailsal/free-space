use super::*;
use crate::snapshot::v2::Store;
use std::{
    os::unix::fs::symlink,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn recovery_skips_active_stages_and_removes_only_exact_regular_single_link_names() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("state");
    fs::create_dir(&dir).unwrap();
    let active = StageFile::create(&dir).unwrap();
    fs::write(active.path(), b"active scanner").unwrap();
    let stale = dir.join("scan-0000000000000000.stage.tmp");
    fs::write(&stale, b"interrupted scanner").unwrap();
    let target = tmp.path().join("foreign");
    fs::write(&target, b"preserve").unwrap();
    let symbolic = dir.join("scan-1111111111111111.stage.tmp");
    symlink(&target, &symbolic).unwrap();
    let linked = dir.join("scan-2222222222222222.stage.tmp");
    fs::hard_link(&target, &linked).unwrap();
    let directory = dir.join("scan-3333333333333333.stage.tmp");
    fs::create_dir(&directory).unwrap();
    for name in [
        "scan-0123.stage.tmp",
        "scan-ABCDEF0123456789.stage.tmp",
        "foreign.stage.tmp",
        "snapshot.stage.tmp",
    ] {
        fs::write(dir.join(name), b"preserve").unwrap();
    }
    let root = tmp.path().join("root");
    fs::create_dir(&root).unwrap();
    let mut store = Store::open(&dir.join("generations.db")).unwrap();
    store
        .scan_roots_with_options(&[root], true, 1, true, true)
        .unwrap();
    assert!(!stale.exists());
    assert_eq!(fs::read(active.path()).unwrap(), b"active scanner");
    assert!(fs::symlink_metadata(&symbolic).unwrap().is_symlink());
    assert_eq!(fs::read(&linked).unwrap(), b"preserve");
    assert_eq!(fs::read(&target).unwrap(), b"preserve");
    assert!(directory.is_dir());
    for name in [
        "scan-0123.stage.tmp",
        "scan-ABCDEF0123456789.stage.tmp",
        "foreign.stage.tmp",
        "snapshot.stage.tmp",
    ] {
        assert_eq!(fs::read(dir.join(name)).unwrap(), b"preserve");
    }
    let active_path = active.path().to_owned();
    drop(active);
    assert!(!active_path.exists());
}

#[test]
fn guard_never_removes_a_replacement_name() {
    let tmp = tempfile::tempdir().unwrap();
    let stage = StageFile::create(tmp.path()).unwrap();
    let path = stage.path().to_owned();
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"replacement").unwrap();
    drop(stage);
    assert_eq!(fs::read(path).unwrap(), b"replacement");
}

#[test]
fn killed_stage_owner_releases_lock_and_next_scan_reclaims_file() {
    let tmp = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "snapshot::v2::stage_files::tests::crash_child"])
        .env("FREESPACE_STAGE_RECOVERY_CHILD", tmp.path())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let ready = tmp.path().join("ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() && Instant::now() < deadline {
        assert!(
            child.try_wait().unwrap().is_none(),
            "stage owner exited before ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    if !ready.exists() {
        child.kill().unwrap();
        child.wait().unwrap();
        panic!("stage owner did not start");
    }
    let path = PathBuf::from(fs::read_to_string(&ready).unwrap());
    recover(tmp.path()).unwrap();
    assert!(path.exists(), "live process must retain its staging inode");
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(path.exists(), "SIGKILL does not execute Drop");
    let root = tmp.path().join("root");
    fs::create_dir(&root).unwrap();
    Store::open(&tmp.path().join("generations.db"))
        .unwrap()
        .scan_roots_with_options(&[root], true, 1, true, true)
        .unwrap();
    assert!(!path.exists());
}

#[test]
fn crash_child() {
    let Some(dir) = std::env::var_os("FREESPACE_STAGE_RECOVERY_CHILD") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let stage = StageFile::create(&dir).unwrap();
    fs::write(stage.path(), b"in-progress staged rows").unwrap();
    fs::write(dir.join("ready"), stage.path().as_os_str().as_bytes()).unwrap();
    std::thread::sleep(Duration::from_secs(20));
    panic!("test controller did not terminate the stage owner");
}
