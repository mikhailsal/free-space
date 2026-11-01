use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use super::{ROOT_READS, checked_capacity, load_tree, prepare_refresh};
use crate::queries::common::{Paths, resolve};
use crate::refresh::graft;
use crate::scan::{self, WalkOptions};
use crate::snapshot::SnapshotDir;
use crate::snapshot::reader::Snapshot;
use crate::snapshot::writer::{self, Reserve, WriteMeta};

fn files(dir: &Path, count: usize) {
    for i in 0..count {
        fs::write(dir.join(format!("f{i:03}")), b"x").unwrap();
    }
}

fn snapshot(root: &Path, dir: &SnapshotDir) -> Snapshot {
    fs::create_dir_all(dir.as_path()).unwrap();
    let mut opts = WalkOptions::new(vec![root.to_path_buf()]);
    opts.workers = 1;
    let mut tree = scan::walk(&opts).unwrap();
    let meta = WriteMeta::new(1, None, tree.started_at);
    let fin = tree.finalize();
    let reserve = Reserve {
        dir: dir.path.clone(),
        min_free: 0,
        check_every: 64 * 1024 * 1024,
    };
    writer::write_snapshot(&dir.current(), tree, fin, &meta, &reserve).unwrap();
    Snapshot::open_current(dir).unwrap()
}

fn check_refresh(old: usize, new: Option<usize>) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    files(&target, old);
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    let snap = snapshot(&root, &dir);
    let (row, path) = resolve(&snap, target.as_os_str().as_bytes()).unwrap();
    assert_eq!(row.items as usize, old);
    fs::remove_dir_all(&target).unwrap();
    if let Some(count) = new {
        fs::create_dir(&target).unwrap();
        files(&target, count);
    }
    let prepared =
        prepare_refresh(&snap, &row, &path, 1, Arc::new(AtomicU64::new(0)), "target").unwrap();
    let mut loaded = prepared.loaded;
    let expected_extra = new.unwrap_or(0);
    assert_eq!(
        prepared.sub.as_ref().map(|t| t.nodes.len()),
        new.map(|n| n + 1)
    );
    let old_nodes = loaded.tree.nodes.len();
    assert_eq!(old_nodes, old + 2);
    assert!(loaded.tree.nodes.capacity() >= old_nodes + expected_extra);
    let before = loaded.tree.nodes.as_ptr();
    let root_raw = loaded.raw_of[0] as usize;
    let mut paths = Paths::new(&snap.conn);
    let policy = graft(
        &mut loaded,
        row.id,
        row.subtree_end,
        prepared.sub,
        &mut paths,
    )
    .unwrap();
    assert_eq!(loaded.tree.nodes.as_ptr(), before);
    let fin = loaded.tree.finalize_with(&policy);
    assert_eq!(fin.order.len(), 1 + new.map_or(0, |n| n + 1));
    assert_eq!(
        loaded.tree.nodes[root_raw].items as usize,
        new.map_or(0, |n| n + 1)
    );
}

#[test]
fn actual_subtree_growth_is_reserved_before_loading() {
    check_refresh(0, Some(128));
}

#[test]
fn actual_subtree_shrink_and_removal_are_reserved() {
    check_refresh(128, Some(1));
    check_refresh(128, None);
}

#[test]
fn capacity_rejects_raw_id_and_usize_overflow() {
    assert_eq!(checked_capacity(100, 128).unwrap(), 228);
    assert!(checked_capacity(u32::MAX as usize, 1).is_err());
    assert!(checked_capacity(usize::MAX, 1).is_err());
}

#[test]
fn refresh_reads_roots_once_and_preserves_multiple_root_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let first = tmp.path().join("a");
    let second = tmp.path().join("b");
    let target = first.join("target");
    fs::create_dir_all(&target).unwrap();
    fs::create_dir_all(&second).unwrap();
    files(&target, 2);
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    fs::create_dir_all(dir.as_path()).unwrap();
    let mut opts = WalkOptions::new(vec![first, second]);
    opts.workers = 1;
    let mut tree = scan::walk(&opts).unwrap();
    let meta = WriteMeta::new(1, None, tree.started_at);
    let fin = tree.finalize();
    let reserve = Reserve {
        dir: dir.path.clone(),
        min_free: 0,
        check_every: 64 * 1024 * 1024,
    };
    writer::write_snapshot(&dir.current(), tree, fin, &meta, &reserve).unwrap();
    let snap = Snapshot::open_current(&dir).unwrap();
    let expected = load_tree(&snap).unwrap();
    let (row, path) = resolve(&snap, target.as_os_str().as_bytes()).unwrap();
    ROOT_READS.with(|count| count.set(0));
    let actual =
        prepare_refresh(&snap, &row, &path, 1, Arc::new(AtomicU64::new(0)), "target").unwrap();
    ROOT_READS.with(|count| assert_eq!(count.get(), 1));
    assert_eq!(actual.loaded.tree.roots.len(), 2);
    for (left, right) in actual.loaded.tree.roots.iter().zip(&expected.tree.roots) {
        assert_eq!(left.path, right.path);
        assert_eq!(left.fs_type, right.fs_type);
        assert_eq!(left.dev, right.dev);
        assert_eq!(left.stats.total, right.stats.total);
        assert_eq!(left.stats.used, right.stats.used);
        assert_eq!(left.stats.free, right.stats.free);
        assert_eq!(left.stats.avail, right.stats.avail);
        assert_eq!(left.stats.inodes_total, right.stats.inodes_total);
        assert_eq!(left.stats.inodes_free, right.stats.inodes_free);
    }
}
