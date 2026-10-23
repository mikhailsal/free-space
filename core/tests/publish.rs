//! Section 6 assertions on the publish protocol (T10): readers never see a
//! torn snapshot (#1), hardlink attribution does not depend on the worker
//! count (#4), and refresh never rotates prev (#6). Crashes at each publish
//! step are in `cli/tests/faults.rs`, where a real process can die.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use freespace_core::job::{self, ScanReport, ScanRequest};
use freespace_core::queries::{self, diff, params::*};
use freespace_core::refresh::{self, RefreshRequest};
use freespace_core::snapshot::SnapshotDir;
use freespace_core::snapshot::reader::Snapshot;

fn scan(root: &Path, dir: &SnapshotDir, workers: usize) -> ScanReport {
    job::run_scan(ScanRequest {
        roots: vec![root.to_path_buf()],
        one_file_system: true,
        workers,
        dir: dir.clone(),
        keep_previous: true,
        on_progress: None,
    })
    .unwrap()
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
