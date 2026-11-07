use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use rusqlite::Connection;

use super::*;
use crate::ErrorCode;
use crate::coarse::export::build;
use crate::coarse::merge::rebuild;
use crate::coarse::testing::{
    KIB, NOTHING, fingerprint, model_of, nested_model, put, sample, snapshot_of,
};
use crate::queries::{self, params::LsParams};
use crate::scan::{self, WalkOptions};
use crate::snapshot::writer::{self, Reserve, WriteMeta};

const THRESHOLD: u64 = 100 * KIB;

/// A folder's name and its buckets as `(lo, hi, totals)`.
type Folded = (String, Vec<(u32, u32, Stats)>);

fn sorted(model: &Model) -> Vec<Folded> {
    let mut out: Vec<_> = model
        .nodes
        .iter()
        .map(|n| {
            let buckets = n
                .buckets
                .iter()
                .map(|b| (b.lo, b.hi, b.stats.clone()))
                .collect();
            (n.path.display().to_string(), buckets)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The oldest scan time of every subtree is not stored in a row of its own; it is derived
/// from the rows and the buckets and must come out as it was.
fn same_scan_times(back: &Model, model: &Model) {
    assert!(model.nodes.iter().all(|n| n.total.min_scanned_at.is_some()));
    for node in &model.nodes {
        let twin = back.nodes.iter().find(|n| n.path == node.path).unwrap();
        assert_eq!(
            twin.total.min_scanned_at,
            node.total.min_scanned_at,
            "{}",
            node.path.display()
        );
    }
}

/// Like `same_scan_times`, for models where two nodes share a path: compared by position.
fn same_scan_times_by_index(back: &Model, model: &Model) {
    assert_eq!(back.nodes.len(), model.nodes.len());
    let key = |n: &Node| (n.path.clone(), n.root);
    for node in &model.nodes {
        let twin = back.nodes.iter().find(|n| key(n) == key(node)).unwrap();
        assert_eq!(twin.total.min_scanned_at, node.total.min_scanned_at);
    }
}

fn tamper(path: &Path, sql: &str) {
    Connection::open(path).unwrap().execute_batch(sql).unwrap();
}

#[test]
fn a_model_reads_back_exactly_as_it_was_written() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    sample(&top);
    let model = model_of(&top, THRESHOLD);
    let snap = snapshot_of(&model, &tmp.path().join("c.db"));
    let back = load(&snap).unwrap();
    assert_eq!(fingerprint(&back), fingerprint(&model));
    assert_eq!(sorted(&back), sorted(&model));
    assert_eq!(back.ppm, model.ppm);
    assert_eq!(back.roots.len(), 1);
    assert_eq!(back.roots[0].path, top);
    assert_eq!(back.roots[0].threshold, THRESHOLD);
    assert_eq!(back.entries(), model.entries());
    same_scan_times(&back, &model);
}

#[test]
fn links_scan_times_and_mounts_survive_the_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    put(&top, "a/orig.bin", 200 * 1024);
    std::fs::create_dir_all(top.join("b")).unwrap();
    std::fs::hard_link(top.join("a/orig.bin"), top.join("b/link.bin")).unwrap();
    let mut model = model_of(&top, THRESHOLD);
    let mut mount = model
        .nodes
        .iter()
        .find(|n| n.path == top.join("a"))
        .unwrap()
        .clone();
    mount.path = top.join("mnt");
    mount.kind = Kind::Mount;
    mount.own_allocated = 0;
    mount.own_apparent = 0;
    mount.buckets = Vec::new();
    mount.total = Stats::default();
    mount.mount = Some(Box::new(MountMeta {
        fs_type: "tmpfs".into(),
        stats: None,
    }));
    model.nodes.push(mount);
    rebuild(&mut model.nodes).unwrap();

    let snap = snapshot_of(&model, &tmp.path().join("c.db"));
    let back = load(&snap).unwrap();
    assert_eq!(fingerprint(&back), fingerprint(&model));
    let root = back.nodes.iter().find(|n| n.root).unwrap();
    assert_eq!(root.total.links, 2);
    let mnt = back.nodes.iter().find(|n| n.kind == Kind::Mount).unwrap();
    assert_eq!(
        mnt.mount.as_ref().map(|m| m.fs_type.as_str()),
        Some("tmpfs")
    );
    assert!(mnt.pinned());
    same_scan_times(&back, &model);
}

#[test]
fn a_root_nested_in_a_mount_point_reads_back_beside_the_entry_that_shares_its_path() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    put(&top, "a/big.bin", 300 * 1024);
    put(&top, "a/m/inner.bin", 200 * 1024);
    let inner = top.join("a/m");
    let model = nested_model(&top, &inner, THRESHOLD);
    let snap = snapshot_of(&model, &tmp.path().join("c.db"));
    let back = load(&snap).unwrap();
    assert_eq!(fingerprint(&back), fingerprint(&model));
    let roots: Vec<&Path> = back.roots.iter().map(|r| r.path.as_path()).collect();
    assert_eq!(roots, [top.as_path(), inner.as_path()]);
    assert_eq!(back.nodes.iter().filter(|n| n.path == inner).count(), 2);
    same_scan_times_by_index(&back, &model);

    // the query layer lists the mount point in the outer folder and, at the same path, what
    // the root holds, as it does for a v1 snapshot of the two roots
    let ls = |path: &Path| {
        let params = LsParams {
            path: Some(path.display().to_string()),
            ..Default::default()
        };
        queries::ls(&snap, &params).unwrap()
    };
    let outer = ls(&top.join("a"));
    let mount = outer.entries.iter().find(|e| e.name == "m").unwrap();
    assert_eq!(mount.allocated, 0);
    let names: Vec<String> = ls(&inner).entries.into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["inner.bin"]);

    // the order of the nodes in memory decides nothing
    let mut flipped = model.clone();
    flipped.nodes.reverse();
    let snap = snapshot_of(&flipped, &tmp.path().join("flipped.db"));
    assert_eq!(fingerprint(&load(&snap).unwrap()), fingerprint(&model));
}

#[test]
fn a_folder_with_many_buckets_keeps_their_borders() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    for i in 0..5000 {
        put(&top, &format!("many/f{i}"), 0);
    }
    let model = model_of(&top, NOTHING);
    let snap = snapshot_of(&model, &tmp.path().join("c.db"));
    let back = load(&snap).unwrap();
    let many = back
        .nodes
        .iter()
        .find(|n| n.path == top.join("many"))
        .unwrap();
    assert!(many.buckets.len() > 1);
    assert_eq!(sorted(&back), sorted(&model));
}

#[test]
fn buckets_that_disagree_with_their_rows_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    sample(&top);
    let path = tmp.path().join("c.db");
    drop(snapshot_of(&model_of(&top, THRESHOLD), &path));
    // a row's own bytes are whatever its children leave of its total, so only the rows of
    // folded items can disagree with what the bucket table says about them
    tamper(
        &path,
        "UPDATE coarse_buckets SET allocated = allocated + 1 WHERE entry_id IS NOT NULL",
    );
    let err = load(&Snapshot::open(&path).unwrap()).err().unwrap();
    assert!(err.message.contains("do not match"), "{}", err.message);
}

#[test]
fn a_snapshot_that_lost_rows_or_buckets_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    sample(&top);
    for sql in [
        "DELETE FROM entries WHERE id = 2",
        "DELETE FROM coarse_buckets WHERE parent_id = 0 AND lo = 0",
        "DELETE FROM coarse_roots",
    ] {
        let path = tmp.path().join("c.db");
        drop(snapshot_of(&model_of(&top, THRESHOLD), &path));
        tamper(&path, sql);
        assert!(load(&Snapshot::open(&path).unwrap()).is_err(), "{sql}");
    }
}

#[test]
fn bucket_parameters_this_build_does_not_use_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    sample(&top);
    let path = tmp.path().join("c.db");
    drop(snapshot_of(&model_of(&top, THRESHOLD), &path));
    tamper(&path, "UPDATE coarse_meta SET slot_bits = 10");
    let err = load(&Snapshot::open(&path).unwrap()).err().unwrap();
    assert_eq!(err.code, ErrorCode::Schema);
}

#[test]
fn a_v1_snapshot_is_not_a_coarse_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    sample(&top);
    let opts = WalkOptions {
        roots: vec![top.clone()],
        one_file_system: true,
        workers: 1,
        seen: Arc::new(AtomicU64::new(0)),
        expected: None,
    };
    let mut tree = scan::walk(&opts).unwrap();
    let fin = tree.finalize();
    let path = tmp.path().join("v1.db");
    let meta = WriteMeta::new(1, None, 1_700_000_000);
    writer::write_snapshot(&path, tree, fin, &meta, &Reserve::new(tmp.path())).unwrap();
    let err = load(&Snapshot::open(&path).unwrap()).err().unwrap();
    assert_eq!(err.code, ErrorCode::Schema);
    assert!(err.message.contains("coarse"), "{}", err.message);
    assert_eq!(
        store::folded_below(&Snapshot::open(&path).unwrap().conn),
        None
    );
}

#[test]
fn exporting_twice_gives_the_same_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    sample(&top);
    let model = model_of(&top, THRESHOLD);
    let (a, b) = (build(&model, 5).unwrap(), build(&model, 5).unwrap());
    assert_eq!(a.fin.order, b.fin.order);
    assert_eq!(a.fin.subtree_end, b.fin.subtree_end);
    assert_eq!(a.fin.path_hash, b.fin.path_hash);
}
