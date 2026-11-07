//! Helpers shared by the unit tests of the coarse modules: build a tree on disk, scan it
//! into a model, and compare models without caring about timestamps or bucket borders.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::AtomicU64;

use super::export::{self, Built};
use super::model::{Model, RootMeta, Stats};
use super::store;
use super::walk::{Boundary, Scope, Walk, Walked, scan};
use crate::platform::FsStats;
use crate::snapshot::reader::Snapshot;
use crate::snapshot::writer::{self, Reserve, WriteMeta};

pub const KIB: u64 = 1024;
/// A threshold no entry reaches: everything folds except what must stay.
pub const NOTHING: u64 = 1024 * 1024 * KIB;

pub fn put(root: &Path, rel: &str, bytes: usize) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

/// A small tree: `a` holds one large file and 50 small ones, `b` is deep and modest, `e` is
/// empty, `z.bin` is large. With a 100 KiB threshold `a` and `z.bin` stay, `b` and `e` fold.
pub fn sample(top: &Path) {
    put(top, "a/big.bin", 300 * 1024);
    for i in 0..50 {
        put(top, &format!("a/small{i}"), 1024);
    }
    put(top, "b/c/d/medium.bin", 64 * 1024);
    put(top, "b/tiny.txt", 10);
    fs::create_dir_all(top.join("e")).unwrap();
    put(top, "z.bin", 1024 * 1024);
}

/// (allocated, apparent, entries) of everything below and including `dir`, counting every
/// hardlinked inode once, by the simplest walk there is.
pub fn naive(dir: &Path) -> (u64, u64, u64) {
    fn go(path: &Path, seen: &mut HashSet<(u64, u64)>, acc: &mut (u64, u64, u64)) {
        let md = fs::symlink_metadata(path).unwrap();
        acc.2 += 1;
        if md.nlink() > 1 && !md.is_dir() && !seen.insert((md.dev(), md.ino())) {
            return;
        }
        acc.0 += md.blocks() * 512;
        acc.1 += md.len();
        if md.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                go(&entry.unwrap().path(), seen, acc);
            }
        }
    }
    let mut acc = (0, 0, 0);
    go(dir, &mut HashSet::new(), &mut acc);
    acc
}

pub fn walk_with(top: &Path, threshold: u64, scope: Option<&Scope>) -> (Walked, bool) {
    let boundary = Boundary::of(top, &[]).unwrap();
    let seen = AtomicU64::new(0);
    let w = Walk {
        boundary: &boundary,
        threshold,
        scope,
        seen: &seen,
    };
    scan(top, true, &w).unwrap()
}

/// A model of the tree at `top`, scanned the way `freespace scan` scans a root.
pub fn model_of(top: &Path, threshold: u64) -> Model {
    let (walked, _) = walk_with(top, threshold, None);
    Model {
        roots: vec![RootMeta {
            path: top.to_path_buf(),
            fs_type: String::new(),
            stats: FsStats::default(),
            threshold,
        }],
        nodes: walked.nodes,
        ppm: 1000,
    }
}

fn stats_line(s: &Stats) -> String {
    format!(
        "{}/{}/{}/{}/{}/{:?}",
        s.allocated, s.apparent, s.entries, s.errors, s.links, s.newest_mtime
    )
}

/// One line per retained node: everything about it except timestamps of the scan and the
/// borders of its buckets, which depend on the order refreshes happened in. The folded
/// totals of a directory are compared as a whole.
pub fn fingerprint(model: &Model) -> Vec<String> {
    let mut lines: Vec<String> = model
        .nodes
        .iter()
        .map(|n| {
            format!(
                "{} {:?} flags={} own={}/{}/{} total={} folded={}",
                n.path.display(),
                n.kind,
                n.flags,
                n.own_allocated,
                n.own_apparent,
                n.own_errors,
                stats_line(&n.total),
                stats_line(&n.rest()),
            )
        })
        .collect();
    lines.sort();
    lines
}

/// Write `model` as a coarse snapshot file at `path` and open it.
pub fn snapshot_of(model: &Model, path: &Path) -> Snapshot {
    let built = export::build(model, 1_700_000_000).unwrap();
    let Built {
        tree, fin, side, ..
    } = built;
    let meta = WriteMeta::new(1, None, 1_700_000_000);
    let dir = path.parent().unwrap();
    writer::write_snapshot(path, tree, fin, &meta, &Reserve::new(dir)).unwrap();
    store::write(path, &side).unwrap();
    Snapshot::open(path).unwrap()
}
