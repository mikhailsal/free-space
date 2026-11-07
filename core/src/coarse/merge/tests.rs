use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicU64;

use super::*;
use crate::coarse::plan::{Plan, plan, scope};
use crate::coarse::testing::{KIB, fingerprint, model_of, naive, put, sample, walk_with};
use crate::coarse::walk::{Boundary, Walk, scan};

const THRESHOLD: u64 = 100 * KIB;

/// What the refresh job does for a path, without the disk protocol around it.
fn refresh(model: &mut Model, target: &Path) {
    let seen = AtomicU64::new(0);
    match plan(&model.nodes, target).unwrap() {
        Plan::Retained(path) => {
            let boundary = Boundary::of(&path, &[]).unwrap();
            let threshold = threshold_for(&model.roots, &path).unwrap();
            let w = Walk {
                boundary: &boundary,
                threshold,
                scope: None,
                seen: &seen,
            };
            let (walked, links) = scan(&path, false, &w).unwrap();
            assert!(!links);
            update(model, &path, walked.nodes).unwrap();
        }
        Plan::Range(slice) => {
            let range = scope(&model.nodes, &slice.parent, slice.lo, slice.hi);
            let boundary = Boundary::of(&slice.parent, &[]).unwrap();
            let threshold = threshold_for(&model.roots, &slice.parent).unwrap();
            let w = Walk {
                boundary: &boundary,
                threshold,
                scope: Some(&range),
                seen: &seen,
            };
            let (walked, links) = scan(&slice.parent, false, &w).unwrap();
            assert!(!links && !walked.unreadable);
            update_range(model, &slice.parent, &range, walked).unwrap();
        }
    }
    verify(model).unwrap();
}

fn paths(model: &Model, top: &Path) -> Vec<String> {
    let mut out: Vec<String> = model
        .nodes
        .iter()
        .map(|n| n.path.strip_prefix(top).unwrap().display().to_string())
        .collect();
    out.sort();
    out
}

#[test]
fn refreshing_a_retained_folder_equals_a_fresh_scan() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    sample(top);
    let mut model = model_of(top, THRESHOLD);
    put(top, "a/big2.bin", 200 * 1024);
    for i in 0..40 {
        fs::remove_file(top.join(format!("a/small{i}"))).unwrap();
    }
    refresh(&mut model, &top.join("a"));
    assert_eq!(fingerprint(&model), fingerprint(&model_of(top, THRESHOLD)));
    assert_eq!(
        paths(&model, top),
        ["", "a", "a/big.bin", "a/big2.bin", "z.bin"]
    );
}

#[test]
fn refreshing_a_folded_path_promotes_what_grew() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    sample(top);
    let mut model = model_of(top, THRESHOLD);
    assert_eq!(paths(&model, top), ["", "a", "a/big.bin", "z.bin"]);
    put(top, "b/c/d/medium.bin", 500 * 1024);
    refresh(&mut model, &top.join("b/c/d"));
    assert_eq!(fingerprint(&model), fingerprint(&model_of(top, THRESHOLD)));
    assert_eq!(
        paths(&model, top),
        [
            "",
            "a",
            "a/big.bin",
            "b",
            "b/c",
            "b/c/d",
            "b/c/d/medium.bin",
            "z.bin"
        ]
    );
}

#[test]
fn refreshing_a_folded_path_that_is_gone_removes_its_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    sample(top);
    let mut model = model_of(top, THRESHOLD);
    let before = model.nodes.iter().find(|n| n.root).unwrap().total.clone();
    fs::remove_dir_all(top.join("b/c")).unwrap();
    refresh(&mut model, &top.join("b/c"));
    assert_eq!(fingerprint(&model), fingerprint(&model_of(top, THRESHOLD)));
    let after = &model.nodes.iter().find(|n| n.root).unwrap().total;
    assert_eq!(after.entries, before.entries - 3);
    assert!(after.allocated < before.allocated);
}

#[test]
fn a_retained_folder_that_shrank_is_folded_into_its_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    sample(top);
    let mut model = model_of(top, THRESHOLD);
    fs::remove_file(top.join("a/big.bin")).unwrap();
    for i in 0..30 {
        fs::remove_file(top.join(format!("a/small{i}"))).unwrap();
    }
    refresh(&mut model, &top.join("a"));
    assert_eq!(paths(&model, top), ["", "z.bin"]);
    assert_eq!(fingerprint(&model), fingerprint(&model_of(top, THRESHOLD)));
}

#[test]
fn a_refresh_leaves_other_regions_as_they_were() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    sample(top);
    let mut model = model_of(top, THRESHOLD);
    let before = fingerprint(&model);
    put(top, "z.bin", 3 * 1024 * 1024);
    refresh(&mut model, &top.join("a"));
    assert_eq!(fingerprint(&model), before);
}

#[test]
fn replacing_a_root_equals_scanning_it_again() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    sample(top);
    let mut model = model_of(top, THRESHOLD);
    put(top, "new/huge.bin", 2 * 1024 * 1024);
    fs::remove_file(top.join("z.bin")).unwrap();
    let (walked, _) = walk_with(top, THRESHOLD, None);
    replace_root(&mut model, top, walked.nodes).unwrap();
    assert_eq!(fingerprint(&model), fingerprint(&model_of(top, THRESHOLD)));
    assert_eq!(naive(top).2, model.entries());
}

#[test]
fn settle_adopts_consistent_totals_and_refuses_forged_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    sample(top);
    let mut model = model_of(top, THRESHOLD);
    let mut forged = model.nodes.clone();
    forged[0].total.allocated += 1;
    assert!(settle(&mut forged).is_err());
    settle(&mut model.nodes).unwrap();
    rebuild(&mut forged).unwrap();
    settle(&mut forged).unwrap();
}

#[test]
fn verify_rejects_models_that_are_not_trees_of_retained_nodes() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    sample(top);
    let model = model_of(top, THRESHOLD);
    verify(&model).unwrap();

    let mut twice = model.clone();
    twice.nodes.push(twice.nodes[0].clone());
    assert!(verify(&twice).is_err());

    let mut orphan = model.clone();
    let mut stray = orphan.nodes[0].clone();
    stray.root = false;
    stray.path = top.join("nowhere/else");
    orphan.nodes.push(stray);
    assert!(verify(&orphan).is_err());

    let mut gap = model.clone();
    let dir = gap.nodes.iter_mut().find(|n| n.root).unwrap();
    dir.buckets.remove(0);
    assert!(verify(&gap).is_err());

    let mut file_with_buckets = model.clone();
    let file = file_with_buckets
        .nodes
        .iter_mut()
        .find(|n| n.path.ends_with("z.bin"))
        .unwrap();
    file.buckets = model.nodes.iter().find(|n| n.root).unwrap().buckets.clone();
    assert!(verify(&file_with_buckets).is_err());

    let mut rootless = model;
    rootless.roots.clear();
    assert!(verify(&rootless).is_err());
}

#[test]
fn pruning_folds_what_no_longer_qualifies_and_keeps_what_still_does() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    sample(top);
    let mut model = model_of(top, THRESHOLD);
    // big.bin (300 KiB) is below the new bar, `a` (about 504 KiB) and z.bin are not
    model.roots[0].threshold = 500 * KIB;
    prune(&mut model).unwrap();
    assert_eq!(paths(&model, top), ["", "a", "z.bin"]);
    // now nothing reaches the bar: the folders fold one level after the other
    model.roots[0].threshold = 2 * 1024 * KIB;
    prune(&mut model).unwrap();
    assert_eq!(paths(&model, top), [""]);
    verify(&model).unwrap();
    let (allocated, _, entries) = naive(top);
    let root = &model.nodes[0];
    assert_eq!(
        (root.total.allocated, root.total.entries),
        (allocated, entries)
    );
}
