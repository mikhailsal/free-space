use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::AtomicU64;

use super::*;
use crate::coarse::buckets::slot_of;
use crate::coarse::model::BUCKET_ENTRIES;
use crate::coarse::testing::{KIB, NOTHING, naive, put, sample, walk_with};

fn relative(top: &Path, walked: &Walked) -> Vec<String> {
    let mut names: Vec<String> = walked
        .nodes
        .iter()
        .map(|n| n.path.strip_prefix(top).unwrap().display().to_string())
        .collect();
    names.sort();
    names
}

fn node<'a>(walked: &'a Walked, path: &Path) -> &'a Node {
    walked.nodes.iter().find(|n| n.path == path).unwrap()
}

#[test]
fn small_things_fold_and_the_totals_stay_exact() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    sample(top);
    let (walked, links) = walk_with(top, 100 * KIB, None);
    assert!(!links);
    assert_eq!(relative(top, &walked), ["", "a", "a/big.bin", "z.bin"]);

    let (allocated, apparent, entries) = naive(top);
    let root = node(&walked, top);
    assert!(root.root);
    assert_eq!(
        (
            root.total.allocated,
            root.total.apparent,
            root.total.entries
        ),
        (allocated, apparent, entries)
    );
    // everything the root holds is itself, its retained children and its buckets
    let a = node(&walked, &top.join("a"));
    let below = a.total.entries + node(&walked, &top.join("z.bin")).total.entries;
    assert_eq!(root.total.entries, 1 + below + root.rest().entries);
    // 50 small files fold into the buckets of `a`; `b` (5 entries) and `e` into the root's
    assert_eq!(a.rest().entries, 50);
    assert_eq!(root.rest().entries, 6);
    assert_eq!(walked.visited, entries - 1);
}

#[test]
fn a_retained_entry_keeps_every_ancestor() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    put(top, "x/y/z/huge.bin", 512 * 1024);
    put(top, "x/y/pebble", 100);
    let (walked, _) = walk_with(top, 100 * KIB, None);
    assert_eq!(
        relative(top, &walked),
        ["", "x", "x/y", "x/y/z", "x/y/z/huge.bin"]
    );
    assert_eq!(node(&walked, &top.join("x/y")).rest().entries, 1);
}

#[test]
fn a_directory_with_many_entries_stays_whatever_it_weighs() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    for i in 0..=BUCKET_ENTRIES {
        put(top, &format!("many/f{i}"), 0);
    }
    // far above anything these files weigh
    let (walked, _) = walk_with(top, NOTHING, None);
    assert_eq!(relative(top, &walked), ["", "many"]);
    let many = node(&walked, &top.join("many"));
    assert_eq!(many.rest().entries, BUCKET_ENTRIES + 1);
    assert!(many.buckets.len() > 1);
    crate::coarse::buckets::validate(&many.buckets).unwrap();
}

#[test]
fn hardlinked_bytes_count_once_and_belong_to_the_smallest_path() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    put(top, "a/orig.bin", 200 * 1024);
    fs::create_dir_all(top.join("b")).unwrap();
    fs::hard_link(top.join("a/orig.bin"), top.join("b/link.bin")).unwrap();
    let (walked, links) = walk_with(top, 100 * KIB, None);
    assert!(links, "the walk reports that it met hardlinks");
    let (allocated, _, entries) = naive(top);
    let root = node(&walked, top);
    assert_eq!(
        (root.total.allocated, root.total.entries),
        (allocated, entries)
    );
    assert_eq!(root.total.links, 2);
    // the owner is retained because it holds the bytes; the alias is folded away
    let owner = node(&walked, &top.join("a/orig.bin"));
    assert_eq!(owner.own_allocated, 200 * KIB);
    assert_eq!(relative(top, &walked), ["", "a", "a/orig.bin"]);
    assert_eq!(node(&walked, &top.join("a")).total.links, 1);
    assert_eq!(node(&walked, top).rest().links, 1);
}

#[test]
fn a_range_walk_covers_exactly_its_slots() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    let names: Vec<String> = (0..300).map(|i| format!("f{i:03}")).collect();
    for name in &names {
        put(top, name, 1);
    }
    let scope = Scope {
        lo: 0,
        hi: SLOTS / 2,
        skip: HashSet::new(),
    };
    let (walked, _) = walk_with(top, NOTHING, Some(&scope));
    let expected = names
        .iter()
        .filter(|n| slot_of(n.as_bytes()) < SLOTS / 2)
        .count() as u64;
    assert!(expected > 0 && expected < 300);
    let held: u64 = walked.buckets.iter().map(|b| b.stats.entries).sum();
    assert_eq!(held, expected);
    assert_eq!(walked.visited, expected);
    assert_eq!(walked.buckets.first().map(|b| b.lo), Some(0));
    assert_eq!(walked.buckets.last().map(|b| b.hi), Some(SLOTS / 2));
    assert!(walked.parent.is_some() && !walked.unreadable);
    assert!(walked.nodes.is_empty());
}

#[test]
fn a_range_walk_leaves_retained_children_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    put(top, "keep.bin", 300 * 1024);
    put(top, "other", 1);
    let scope = Scope {
        lo: 0,
        hi: SLOTS,
        skip: HashSet::from([b"keep.bin".to_vec()]),
    };
    let (walked, _) = walk_with(top, 100 * KIB, Some(&scope));
    assert_eq!(walked.visited, 1);
    assert!(walked.nodes.is_empty());
    assert_eq!(
        walked.buckets.iter().map(|b| b.stats.entries).sum::<u64>(),
        1
    );
}

#[test]
fn a_directory_that_cannot_be_read_stays_as_an_unreadable_leaf() {
    if unsafe { libc::geteuid() } == 0 {
        return; // root reads everything
    }
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    put(top, "open/file", 10);
    fs::create_dir_all(top.join("locked/inner")).unwrap();
    fs::set_permissions(top.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let (walked, _) = walk_with(top, NOTHING, None);
    fs::set_permissions(top.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(relative(top, &walked), ["", "locked"]);
    let locked = node(&walked, &top.join("locked"));
    assert!(locked.pinned());
    assert_eq!(locked.flags & flags::DENIED, flags::DENIED);
    assert_eq!(locked.total.errors, 1);
    assert_eq!(node(&walked, top).total.errors, 1);
}

#[test]
fn a_symlink_counts_as_itself_and_is_not_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    put(top, "real/data.bin", 400 * 1024);
    std::os::unix::fs::symlink(top.join("real"), top.join("alias")).unwrap();
    let (walked, _) = walk_with(top, 100 * KIB, None);
    let (allocated, _, entries) = naive(top);
    let root = node(&walked, top);
    assert_eq!(
        (root.total.allocated, root.total.entries),
        (allocated, entries)
    );
    assert_eq!(relative(top, &walked), ["", "real", "real/data.bin"]);
}

#[test]
fn a_missing_top_is_an_error_and_a_file_is_not_a_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    put(top, "f", 1);
    let boundary = Boundary::of(top, &[]).unwrap();
    let seen = AtomicU64::new(0);
    let w = Walk {
        boundary: &boundary,
        threshold: 1,
        scope: None,
        seen: &seen,
    };
    let mut owners = Owners::new();
    assert!(fold(&top.join("gone"), true, &w, &mut owners).is_err());
    let err = fold(&top.join("f"), true, &w, &mut owners).err().unwrap();
    assert_eq!(err.code, crate::ErrorCode::Usage);
}
