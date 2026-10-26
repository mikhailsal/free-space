//! Section 6 assertions on the publish protocol (T10): readers never see a
//! torn snapshot (#1), hardlink attribution does not depend on the worker
//! count (#4), and refresh never rotates prev (#6). Crashes at each publish
//! step are in `cli/tests/faults.rs`, where a real process can die.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use freespace_core::ErrorCode;
use freespace_core::job::{self, ScanReport, ScanRequest};
use freespace_core::queries::{self, diff, params::*};
use freespace_core::refresh::{self, RefreshRequest};
use freespace_core::snapshot::reader::Snapshot;
use freespace_core::snapshot::{SnapshotDir, state};

fn scan(root: &Path, dir: &SnapshotDir, workers: usize) -> ScanReport {
    scan_request(root, dir, workers, true).unwrap()
}

fn scan_request(
    root: &Path,
    dir: &SnapshotDir,
    workers: usize,
    keep_previous: bool,
) -> freespace_core::Result<ScanReport> {
    job::run_scan(ScanRequest {
        roots: vec![root.to_path_buf()],
        one_file_system: true,
        workers,
        dir: dir.clone(),
        keep_previous,
        on_progress: None,
    })
}

fn tree(root: &Path, dirs: usize, files: usize) {
    for d in 0..dirs {
        let p = root.join(format!("d{d}/sub"));
        fs::create_dir_all(&p).unwrap();
        for f in 0..files {
            fs::write(p.join(format!("f{f}")), vec![1u8; 4096 * (f % 3 + 1)]).unwrap();
        }
    }
}

/// path → (allocated, apparent, hardlink alias) for every entry in the snapshot.
fn everything(dir: &SnapshotDir, root: &Path) -> BTreeMap<String, (u64, u64, bool)> {
    let r = queries::find(
        &Snapshot::open_current(dir).unwrap(),
        &FindParams {
            path: Some(root.to_str().unwrap().to_owned()),
            kind: Some("any".into()),
            limit: Some(10_000),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!r.truncated);
    r.entries
        .into_iter()
        .map(|e| {
            let e = e.entry;
            let rel = e
                .path
                .strip_prefix(root.to_str().unwrap())
                .unwrap()
                .to_owned();
            (rel, (e.allocated, e.apparent, e.hardlink_alias))
        })
        .collect()
}

#[test]
fn readers_never_see_a_partial_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    tree(&root, 20, 100);
    let dir = SnapshotDir::new(tmp.path().join("snap"));
    scan(&root, &dir, 2);

    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let (dir, stop, root) = (dir.clone(), stop.clone(), root.clone());
            std::thread::spawn(move || {
                let mut seen = BTreeSet::new();
                let mut reads = 0u32;
                while !stop.load(Ordering::Relaxed) || reads == 0 {
                    let s = Snapshot::open_current(&dir).expect("current is always openable");
                    let ok: String = s
                        .conn
                        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                        .unwrap();
                    assert_eq!(ok, "ok", "generation {}", s.meta.generation);
                    let ls = queries::ls(
                        &s,
                        &LsParams {
                            path: Some(root.to_str().unwrap().to_owned()),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    assert_eq!(ls.header.generation, s.meta.generation);
                    seen.insert(s.meta.generation);
                    reads += 1;
                }
                seen
            })
        })
        .collect();
    for g in 0..4 {
        fs::write(root.join(format!("new{g}")), vec![2u8; 8192]).unwrap();
        scan(&root, &dir, 2);
    }
    stop.store(true, Ordering::Relaxed);
    let mut all = BTreeSet::new();
    for r in readers {
        all.extend(r.join().unwrap());
    }
    assert!(all.iter().all(|g| (1..=5).contains(g)), "{all:?}");
    assert!(all.len() > 1, "readers ran during the publishes: {all:?}");
}

#[test]
fn hardlinks_count_the_same_with_one_worker_or_eight() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    tree(&root, 12, 30);
    // links across sibling folders, so walk order would decide the owner
    for d in 0..12 {
        let src = root.join(format!("d{d}/sub/f0"));
        for k in [(d + 5) % 12, (d + 7) % 12] {
            fs::hard_link(&src, root.join(format!("d{k}/link-from-{d}"))).unwrap();
        }
    }
    let one = SnapshotDir::new(tmp.path().join("one"));
    let eight = SnapshotDir::new(tmp.path().join("eight"));
    scan(&root, &one, 1);
    scan(&root, &eight, 8);
    let (a, b) = (everything(&one, &root), everything(&eight, &root));
    assert_eq!(a.len(), 12 * 2 + 12 * 30 + 24, "folders, files and links");
    assert_eq!(a, b);
    assert_eq!(
        a.values().filter(|v| v.2).count(),
        24,
        "each link is an alias"
    );
}

#[test]
fn refresh_keeps_prev_and_marks_the_diff_partial() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    tree(&root, 3, 5);
    let dir = SnapshotDir::new(tmp.path().join("snap"));
    scan(&root, &dir, 2); // 1
    fs::write(root.join("d0/grew"), vec![3u8; 64 << 10]).unwrap();
    scan(&root, &dir, 2); // 2, prev 1
    fs::write(root.join("d1/sub/more"), vec![3u8; 64 << 10]).unwrap();
    let rep = refresh::run_refresh(RefreshRequest {
        path: root.join("d1").as_os_str().as_bytes().to_vec(),
        workers: 2,
        dir: dir.clone(),
        on_progress: None,
    })
    .unwrap();
    assert_eq!(rep.generation, 3);

    let (cur, prev) = diff::open_pair(&dir).unwrap();
    assert_eq!(cur.meta.generation, 3);
    assert_eq!(prev.meta.generation, 1, "refresh does not rotate prev");
    assert_eq!(cur.meta.prev_generation, Some(1));
    let d = queries::diff(
        &dir,
        &DiffParams {
            path: Some(root.to_str().unwrap().to_owned()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(d.partial);
    assert_eq!(d.refreshed.len(), 1);
    for grew in ["d0", "d1"] {
        let e = d.entries.iter().find(|e| e.name == grew).unwrap();
        assert!(e.delta >= 64 << 10, "{grew}: {}", e.delta);
    }
}

#[test]
fn after_a_schema_change_the_old_snapshot_is_not_kept_as_prev() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    tree(&root, 2, 3);
    let dir = SnapshotDir::new(tmp.path().join("snap"));
    scan(&root, &dir, 2); // 1
    scan(&root, &dir, 2); // 2, prev 1
    // what an older freespace left behind: current and prev both unreadable
    for p in [dir.current(), dir.prev()] {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute("UPDATE meta SET schema_version = 0", []).unwrap();
    }
    let Err(e) = diff::open_pair(&dir) else {
        panic!("diff opened")
    };
    assert_eq!(
        e.code,
        ErrorCode::Schema,
        "current is unreadable: {}",
        e.message
    );

    let rep = scan(&root, &dir, 2); // 3, the rescan after the upgrade
    assert_eq!(rep.prev_generation, None);
    assert!(
        !dir.prev().exists(),
        "the old-schema file is not rotated into prev"
    );
    let Err(e) = diff::open_pair(&dir) else {
        panic!("diff opened")
    };
    assert_eq!(e.code, ErrorCode::NoPrev, "{}", e.message);

    scan(&root, &dir, 2); // 4, prev 3
    let (cur, prev) = diff::open_pair(&dir).unwrap();
    assert_eq!((cur.meta.generation, prev.meta.generation), (4, 3));
}

#[test]
fn an_old_schema_prev_reads_as_no_prev() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    tree(&root, 2, 3);
    let dir = SnapshotDir::new(tmp.path().join("snap"));
    scan(&root, &dir, 2);
    scan(&root, &dir, 2);
    let c = rusqlite::Connection::open(dir.prev()).unwrap();
    c.execute("UPDATE meta SET schema_version = 0", []).unwrap();
    drop(c);
    let Err(e) = diff::open_pair(&dir) else {
        panic!("diff opened")
    };
    assert_eq!(e.code, ErrorCode::NoPrev, "{}", e.message);
}

fn two_generations(root: &Path, dir: &SnapshotDir) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("a"), [1; 4096]).unwrap();
    scan(root, dir, 1);
    fs::write(root.join("b"), [2; 4096]).unwrap();
    scan(root, dir, 1);
}

fn assert_retained(dir: &SnapshotDir, bytes: &[u8], inode: u64) {
    assert!(
        fs::read(dir.prev()).unwrap() == bytes,
        "previous bytes changed"
    );
    assert_eq!(fs::metadata(dir.prev()).unwrap().ino(), inode);
}

#[test]
fn non_sqlite_current_retains_previous_bytes_generation_and_diff() {
    assert_corrupt_current_retains_previous(b"not a SQLite database");
}

#[test]
fn empty_current_retains_previous_bytes_generation_and_diff() {
    assert_corrupt_current_retains_previous(b"");
}

#[test]
fn logically_incomplete_current_does_not_replace_previous() {
    for mutation in [
        "DROP TABLE entries",
        "ALTER TABLE entries DROP COLUMN flags",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        let dir = SnapshotDir::new(tmp.path().join("snap"));
        two_generations(&root, &dir);
        let bytes = fs::read(dir.prev()).unwrap();
        let inode = fs::metadata(dir.prev()).unwrap().ino();
        let conn = rusqlite::Connection::open(dir.current()).unwrap();
        conn.execute(mutation, []).unwrap();
        drop(conn);
        fs::write(root.join("c"), [3; 4096]).unwrap();

        let rep = scan(&root, &dir, 1);
        assert_retained(&dir, &bytes, inode);
        assert_eq!((rep.generation, rep.prev_generation), (3, Some(1)));
        let (current, previous) = diff::open_pair(&dir).unwrap();
        assert_eq!(current.meta.prev_generation, Some(1));
        assert_eq!(previous.meta.generation, 1);
    }
}

fn assert_corrupt_current_retains_previous(corruption: &[u8]) {
    for state_case in ["normal", "missing", "corrupt", "newer_previous"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        let dir = SnapshotDir::new(tmp.path().join("snap"));
        two_generations(&root, &dir);
        let prev_generation = if state_case == "newer_previous" {
            17
        } else {
            1
        };
        if state_case == "newer_previous" {
            let conn = rusqlite::Connection::open(dir.prev()).unwrap();
            conn.execute("UPDATE meta SET generation = 17", []).unwrap();
        }
        match state_case {
            "missing" | "newer_previous" => fs::remove_file(dir.state()).unwrap(),
            "corrupt" => fs::write(dir.state(), b"{not json").unwrap(),
            _ => {}
        }
        let bytes = fs::read(dir.prev()).unwrap();
        let inode = fs::metadata(dir.prev()).unwrap().ino();
        fs::write(dir.current(), corruption).unwrap();
        fs::write(root.join("c"), [3; 4096]).unwrap();
        let rep = scan(&root, &dir, 1);
        let generation = if state_case == "normal" {
            3
        } else {
            prev_generation + 1
        };
        assert_retained(&dir, &bytes, inode);
        assert_eq!(
            (rep.generation, rep.prev_generation),
            (generation, Some(prev_generation))
        );
        let (cur, prev) = diff::open_pair(&dir).unwrap();
        assert_eq!(cur.meta.generation, generation);
        assert_eq!(cur.meta.prev_generation, Some(prev_generation));
        assert_eq!(prev.meta.generation, prev_generation);
        let st = state::read(&dir).unwrap().unwrap();
        assert_eq!(st.current_generation, Some(generation));
        assert_eq!(st.prev_generation, Some(prev_generation));
        let result = queries::diff(
            &dir,
            &DiffParams {
                path: Some(root.to_str().unwrap().into()),
                kind: Some("file".into()),
                ..Default::default()
            },
        )
        .unwrap();
        for name in ["b", "c"] {
            assert!(
                result
                    .entries
                    .iter()
                    .any(|entry| entry.name == name && entry.delta >= 4096)
            );
        }
    }
}

#[test]
fn corrupt_current_drops_previous_only_when_unusable_or_disabled() {
    for corruption in [b"not a SQLite database".as_slice(), b"".as_slice()] {
        for control in [
            "disabled",
            "incompatible_previous",
            "corrupt_previous",
            "missing_table_previous",
            "missing_previous",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("r");
            let dir = SnapshotDir::new(tmp.path().join("snap"));
            two_generations(&root, &dir);
            match control {
                "incompatible_previous" => {
                    let conn = rusqlite::Connection::open(dir.prev()).unwrap();
                    conn.execute("UPDATE meta SET schema_version = 0", [])
                        .unwrap();
                }
                "corrupt_previous" => fs::write(dir.prev(), b"not SQLite").unwrap(),
                "missing_table_previous" => {
                    let conn = rusqlite::Connection::open(dir.prev()).unwrap();
                    conn.execute("DROP TABLE entries", []).unwrap();
                }
                "missing_previous" => fs::remove_file(dir.prev()).unwrap(),
                _ => {}
            }
            fs::write(dir.current(), corruption).unwrap();
            let rep = scan_request(&root, &dir, 1, control != "disabled").unwrap();
            assert_eq!((rep.generation, rep.prev_generation), (3, None));
            assert!(!dir.prev().exists());
            assert_eq!(
                Snapshot::open_current(&dir).unwrap().meta.prev_generation,
                None
            );
            assert_eq!(state::read(&dir).unwrap().unwrap().prev_generation, None);
            assert_eq!(diff::open_pair(&dir).err().unwrap().code, ErrorCode::NoPrev);
        }
    }
}

#[test]
fn failed_recovery_publish_preserves_previous() {
    for keep_previous in [true, false] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        let dir = SnapshotDir::new(tmp.path().join("snap"));
        two_generations(&root, &dir);
        let bytes = fs::read(dir.prev()).unwrap();
        let inode = fs::metadata(dir.prev()).unwrap().ino();
        // An unreadable current that also makes rename fail, after tmp is written.
        fs::remove_file(dir.current()).unwrap();
        fs::create_dir(dir.current()).unwrap();
        fs::write(dir.current().join("block-rename"), b"x").unwrap();
        let err = scan_request(&root, &dir, 1, keep_previous).unwrap_err();
        assert!(err.message.contains("publish step rename failed"), "{err}");
        assert_retained(&dir, &bytes, inode);
        assert_eq!(Snapshot::open(&dir.prev()).unwrap().meta.generation, 1);
    }
}
