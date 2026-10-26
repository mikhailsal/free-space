use super::*;
use crate::job::{self, ScanRequest};
use std::fs;

fn scan(root: &std::path::Path, dir: &SnapshotDir) {
    job::run_scan(ScanRequest {
        roots: vec![root.to_path_buf()],
        one_file_system: true,
        workers: 1,
        dir: dir.clone(),
        keep_previous: true,
        on_progress: None,
    })
    .unwrap();
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, SnapshotDir) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("one"), b"one").unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snap"));
    scan(&root, &dir);
    (tmp, root, dir)
}

#[test]
fn publish_between_opens_retries_to_coherent_pair() {
    let (_tmp, root, dir) = fixture();
    let mut did_publish = false;
    let (cur, prev) = open_pair_with_hook(&dir, |attempt| {
        if attempt == 1 && !did_publish {
            fs::write(root.join("two"), b"two").unwrap();
            scan(&root, &dir);
            did_publish = true;
        }
    })
    .unwrap();
    assert_eq!((cur.meta.generation, prev.meta.generation), (2, 1));
}

#[test]
fn genuine_first_generation_absence_remains_no_prev() {
    let (_tmp, _root, dir) = fixture();
    let err = match open_pair_with_hook(&dir, |_| {}) {
        Ok(_) => panic!("pair opened without prev"),
        Err(err) => err,
    };
    assert_eq!(err.code, ErrorCode::NoPrev);
}

#[test]
fn declared_but_missing_previous_is_rotated() {
    let (_tmp, root, dir) = fixture();
    fs::write(root.join("two"), b"two").unwrap();
    scan(&root, &dir);
    fs::remove_file(dir.prev()).unwrap();
    let err = match open_pair_with_hook(&dir, |_| {}) {
        Ok(_) => panic!("pair opened without prev"),
        Err(err) => err,
    };
    assert_eq!(err.code, ErrorCode::Rotated);
    assert!(err.fix.as_deref().unwrap_or_default().contains("retry"));
}

#[test]
fn persistent_same_generation_pair_is_rotated() {
    let (_tmp, _root, dir) = fixture();
    fs::hard_link(dir.current(), dir.prev()).unwrap();
    let err = match open_pair_with_hook(&dir, |_| {}) {
        Ok(_) => panic!("same-generation pair opened"),
        Err(err) => err,
    };
    assert_eq!(err.code, ErrorCode::Rotated);
    assert!(err.fix.as_deref().unwrap_or_default().contains("retry"));
}

#[test]
fn old_schema_previous_remains_no_prev() {
    let (_tmp, root, dir) = fixture();
    fs::write(root.join("two"), b"two").unwrap();
    scan(&root, &dir);
    let old = rusqlite::Connection::open(dir.prev()).unwrap();
    old.execute("UPDATE meta SET schema_version = 0", [])
        .unwrap();
    drop(old);
    let err = match open_pair_with_hook(&dir, |_| {}) {
        Ok(_) => panic!("old-schema previous opened"),
        Err(err) => err,
    };
    assert_eq!(err.code, ErrorCode::NoPrev);
    assert!(err.message.contains("another freespace version"));
}

#[test]
fn persistent_generation_mismatch_is_rotated() {
    let (tmp, root, dir) = fixture();
    let stale = tmp.path().join("stale.db");
    fs::copy(dir.current(), &stale).unwrap();
    fs::write(root.join("two"), b"two").unwrap();
    scan(&root, &dir);
    fs::write(root.join("three"), b"three").unwrap();
    scan(&root, &dir);
    fs::remove_file(dir.prev()).unwrap();
    fs::copy(stale, dir.prev()).unwrap();
    let err = match open_pair_with_hook(&dir, |_| {}) {
        Ok(_) => panic!("mismatched pair opened"),
        Err(err) => err,
    };
    assert_eq!(err.code, ErrorCode::Rotated);
}
