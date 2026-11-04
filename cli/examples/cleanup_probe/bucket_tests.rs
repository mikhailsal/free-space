//! Behaviour specific to bucketed residuals: tiling, entry-count retention, range refresh.
use super::*;
use crate::buckets::{find, slot_of, validate};
use crate::model::{BUCKET_ENTRIES, SLOTS};

/// `dirs` directories d000.. holding `files` one-byte files each.
fn make_tree(root: &Path, dirs: usize, files: usize) {
    for d in 0..dirs {
        let dir = root.join(format!("d{d:03}"));
        fs::create_dir_all(&dir).unwrap();
        for f in 0..files {
            fs::write(dir.join(format!("f{f:03}")), b"x").unwrap();
        }
    }
}
fn shape(nodes: &[Node], root: &Path) -> (u64, u64, u64, u64) {
    let t = &nodes.iter().find(|n| n.path == root).unwrap().total;
    (t.allocated, t.apparent, t.files, t.dirs)
}
fn range_of(nodes: &[Node], target: &Path) -> Slice {
    match state::plan(nodes, target).unwrap() {
        Plan::Range(slice) => slice,
        Plan::Retained(path) => panic!("expected a range plan, got retained {path:?}"),
    }
}
fn grow(dir: &Path) {
    for f in 0..30 {
        fs::write(dir.join(format!("new{f:02}")), b"yy").unwrap();
    }
    for f in 0..10 {
        fs::remove_file(dir.join(format!("f{f:03}"))).unwrap();
    }
}
fn scan_command(root: &Path, snapshot: &Path) -> Value {
    let (root, snapshot, ppm) = (root.to_path_buf(), snapshot.to_path_buf(), 1_000_000);
    execute(Command::Scan {
        root,
        snapshot,
        ppm,
    })
    .unwrap()
}
fn refresh_command(root: &Path, snapshot: &Path, target: &Path) -> Value {
    let (root, snapshot, target) = (
        root.to_path_buf(),
        snapshot.to_path_buf(),
        target.to_path_buf(),
    );
    execute(Command::Refresh {
        root,
        snapshot,
        target,
    })
    .unwrap()
}

#[test]
fn folded_children_tile_all_slots_and_conserve_totals() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 50, 100);
    let result = walk::scan(&root, u64::MAX).unwrap();
    assert_eq!(result.nodes.len(), 1);
    let top = &result.nodes[0];
    validate(&top.buckets).unwrap();
    assert!(
        top.buckets.len() >= 2,
        "5050 folded entries cannot fit one {BUCKET_ENTRIES}-entry bucket"
    );
    assert!(
        top.buckets
            .iter()
            .all(|b| b.stats.entries() <= BUCKET_ENTRIES)
    );
    let rest = top.rest().unwrap();
    assert_eq!((rest.files, rest.dirs), (5000, 50));
    assert_eq!(top.total.allocated, top.own_allocated + rest.allocated);
    state::verify(&result.nodes).unwrap();
    let slot = slot_of(b"d017");
    let bucket = &top.buckets[find(&top.buckets, slot).unwrap()];
    assert!(bucket.lo <= slot && slot < bucket.hi && bucket.stats.dirs > 0);
}

#[test]
fn directory_with_many_entries_is_retained_even_when_it_is_small() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    for (name, files) in [("bulk", BUCKET_ENTRIES), ("edge", BUCKET_ENTRIES - 1)] {
        fs::create_dir_all(root.join(name)).unwrap();
        for f in 0..files {
            fs::File::create(root.join(name).join(format!("f{f}"))).unwrap();
        }
    }
    let result = walk::scan(&root, u64::MAX).unwrap();
    // bulk holds 4096 files plus itself, one over the cap; edge holds exactly the cap.
    let bulk = result
        .nodes
        .iter()
        .find(|n| n.path == root.join("bulk"))
        .unwrap();
    assert!(!result.nodes.iter().any(|n| n.path == root.join("edge")));
    validate(&bulk.buckets).unwrap();
    assert_eq!(bulk.rest().unwrap().files, BUCKET_ENTRIES);
    state::verify(&result.nodes).unwrap();
}

#[test]
fn deep_target_maps_to_the_bucket_of_its_first_folded_component() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 50, 100);
    let nodes = walk::scan(&root, u64::MAX).unwrap().nodes;
    let dir = range_of(&nodes, &root.join("d017"));
    let file = range_of(&nodes, &root.join("d017/f005"));
    let missing = range_of(&nodes, &root.join("d017/never/existed"));
    for other in [&file, &missing] {
        assert_eq!(
            (other.parent.as_path(), other.lo, other.hi),
            (dir.parent.as_path(), dir.lo, dir.hi)
        );
    }
    assert_eq!(dir.parent, root);
    assert!(dir.hi - dir.lo < SLOTS);
    assert!(matches!(state::plan(&nodes, &root).unwrap(), Plan::Retained(p) if p == root));
}

#[test]
fn range_refresh_matches_a_full_rescan_and_visits_less() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 50, 100);
    let nodes = walk::scan(&root, u64::MAX).unwrap().nodes;
    grow(&root.join("d017"));
    let slice = range_of(&nodes, &root.join("d017"));
    let scope = state::scope(&nodes, &slice.parent, slice.lo, slice.hi);
    let ranged = walk::scan_range(&slice.parent, u64::MAX, &scope).unwrap();
    let full = walk::scan(&root, u64::MAX).unwrap();
    assert!(
        ranged.visited < full.visited,
        "{} vs {}",
        ranged.visited,
        full.visited
    );
    let mut merged = nodes.clone();
    state::update_range(&mut merged, &slice.parent, &scope, ranged, u64::MAX).unwrap();
    assert_eq!(shape(&merged, &root), shape(&full.nodes, &root));
    state::verify(&merged).unwrap();
}

#[test]
fn range_refresh_promotes_a_child_that_grew_large() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 3, 5);
    let nodes = walk::scan(&root, 1_000_000).unwrap().nodes;
    assert_eq!(nodes.len(), 1);
    fs::write(root.join("d001/big"), vec![7; 1_100_000]).unwrap();
    let slice = range_of(&nodes, &root.join("d001"));
    let scope = state::scope(&nodes, &slice.parent, slice.lo, slice.hi);
    let ranged = walk::scan_range(&slice.parent, 1_000_000, &scope).unwrap();
    let mut merged = nodes.clone();
    state::update_range(&mut merged, &slice.parent, &scope, ranged, 1_000_000).unwrap();
    let promoted = merged.iter().find(|n| n.path == root.join("d001")).unwrap();
    assert_eq!(promoted.parent.as_deref(), Some(root.as_path()));
    assert!(merged.iter().any(|n| n.path == root.join("d001/big")));
    let full = walk::scan(&root, 1_000_000).unwrap();
    assert_eq!(shape(&merged, &root), shape(&full.nodes, &root));
    // d001 is a retained node now, so no bucket of the root may still count it.
    let top = merged.iter().find(|n| n.path == root).unwrap();
    assert_eq!(top.rest().unwrap().dirs, 2);
    state::verify(&merged).unwrap();
}

#[test]
fn range_refresh_skips_retained_siblings() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 3, 5);
    fs::write(root.join("d001/big"), vec![7; 1_100_000]).unwrap();
    let nodes = walk::scan(&root, 1_000_000).unwrap().nodes;
    assert!(nodes.iter().any(|n| n.path == root.join("d001")));
    fs::write(root.join("d002/extra"), b"zz").unwrap();
    let slice = range_of(&nodes, &root.join("d002"));
    let scope = state::scope(&nodes, &slice.parent, slice.lo, slice.hi);
    assert!(scope.skip.contains(&b"d001"[..]));
    let ranged = walk::scan_range(&slice.parent, 1_000_000, &scope).unwrap();
    assert!(
        !ranged
            .nodes
            .iter()
            .any(|n| n.path.starts_with(root.join("d001")))
    );
    let mut merged = nodes.clone();
    state::update_range(&mut merged, &slice.parent, &scope, ranged, 1_000_000).unwrap();
    let full = walk::scan(&root, 1_000_000).unwrap();
    assert_eq!(shape(&merged, &root), shape(&full.nodes, &root));
    state::verify(&merged).unwrap();
}

#[test]
fn demoted_child_is_absorbed_into_the_bucket_of_its_slot() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 50, 100);
    fs::write(root.join("d017/big"), vec![7; 1_100_000]).unwrap();
    let mut nodes = walk::scan(&root, 1_000_000).unwrap().nodes;
    assert!(nodes.iter().any(|n| n.path == root.join("d017")));
    let before: Vec<u64> = nodes
        .iter()
        .find(|n| n.path == root)
        .unwrap()
        .buckets
        .iter()
        .map(|b| b.stats.entries())
        .collect();
    fs::remove_file(root.join("d017/big")).unwrap();
    let fresh = walk::scan(&root.join("d017"), 1_000_000).unwrap().nodes;
    state::update(&mut nodes, &root.join("d017"), fresh, 1_000_000).unwrap();
    assert_eq!(nodes.len(), 1);
    let after: Vec<u64> = nodes[0].buckets.iter().map(|b| b.stats.entries()).collect();
    let home = find(&nodes[0].buckets, slot_of(b"d017")).unwrap();
    assert_eq!(before.len(), after.len());
    for (i, (was, now)) in before.iter().zip(&after).enumerate() {
        assert_eq!(*now, *was + if i == home { 101 } else { 0 }, "bucket {i}");
    }
    let full = walk::scan(&root, 1_000_000).unwrap();
    assert_eq!(shape(&nodes, &root), shape(&full.nodes, &root));
    state::verify(&nodes).unwrap();
}

#[test]
fn retained_file_target_rescans_its_directory() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 3, 5);
    fs::write(root.join("big"), vec![7; 1_100_000]).unwrap();
    let nodes = walk::scan(&root, 1_000_000).unwrap().nodes;
    assert!(nodes.iter().any(|n| n.path == root.join("big")));
    assert!(
        matches!(state::plan(&nodes, &root.join("big")).unwrap(), Plan::Retained(p) if p == root)
    );
}

#[test]
fn execute_refresh_of_omitted_directory_reports_a_bounded_range() {
    let d = tempfile::tempdir().unwrap();
    let (root, snapshot) = (d.path().join("tree"), d.path().join("out"));
    make_tree(&root, 50, 100);
    let scan = scan_command(&root, &snapshot);
    assert_eq!(scan["scope"], "full");
    assert!(scan["buckets"].as_u64().unwrap() >= 2);
    grow(&root.join("d017"));
    let out = refresh_command(&root, &snapshot, &root.join("d017"));
    assert_eq!(
        (&out["scope"], &out["fallback_full_root"]),
        (&Value::from("range"), &Value::from(false))
    );
    assert_eq!(out["boundary"].as_str().unwrap(), root.to_str().unwrap());
    assert!(out["visited"].as_u64().unwrap() < scan["visited"].as_u64().unwrap());
    let (old, new) = (
        out["range"]["old_entries"].as_u64().unwrap(),
        out["range"]["new_entries"].as_u64().unwrap(),
    );
    assert_eq!(new, old + 20);
    let (meta, nodes) = store::read(&snapshot.join("coarse.db")).unwrap();
    assert_eq!(
        (meta.generation, nodes.len(), nodes[0].total.files),
        (2, 1, 5020)
    );
    assert!(snapshot.join("coarse.prev.db").exists());
}

fn dir_in_bucket(nodes: &[Node], root: &Path, last: bool) -> PathBuf {
    let buckets = &nodes.iter().find(|n| n.path == root).unwrap().buckets;
    let want = if last { buckets.len() - 1 } else { 0 };
    (0..50)
        .map(|d| root.join(format!("d{d:03}")))
        .find(|p| find(buckets, slot_of(p.file_name().unwrap().as_encoded_bytes())) == Some(want))
        .unwrap()
}

#[test]
fn region_with_hardlinks_is_never_refreshed_on_its_own() {
    let d = tempfile::tempdir().unwrap();
    let (root, snapshot) = (d.path().join("tree"), d.path().join("out"));
    make_tree(&root, 50, 100);
    scan_command(&root, &snapshot);
    let nodes = store::read(&snapshot.join("coarse.db")).unwrap().1;
    let (x, y) = (
        dir_in_bucket(&nodes, &root, false),
        dir_in_bucket(&nodes, &root, true),
    );
    assert_ne!(x, y);
    // A link created after the snapshot makes the rescan see hardlinks: fall back to the root.
    fs::hard_link(x.join("f000"), x.join("link")).unwrap();
    let first = refresh_command(&root, &snapshot, &x);
    assert_eq!(
        (&first["scope"], &first["fallback_full_root"]),
        (&Value::from("full"), &Value::from(true))
    );
    // Links in another bucket do not matter to a region without any.
    let other = refresh_command(&root, &snapshot, &y);
    assert_eq!(
        (&other["scope"], &other["fallback_full_root"]),
        (&Value::from("range"), &Value::from(false))
    );
    // The region that now holds links is refreshed through the root.
    let again = refresh_command(&root, &snapshot, &x);
    assert_eq!(
        (&again["scope"], &again["fallback_full_root"]),
        (&Value::from("full"), &Value::from(true))
    );
    let meta = store::read(&snapshot.join("coarse.db")).unwrap().0;
    assert!(meta.has_hardlinks);
}

fn tamper(db: &Path, sql: &str) {
    rusqlite::Connection::open(db)
        .unwrap()
        .execute_batch(sql)
        .unwrap();
}

#[test]
fn store_rejects_inconsistent_snapshots() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 50, 100);
    let nodes = walk::scan(&root, u64::MAX).unwrap().nodes;
    let meta = store::Meta {
        root,
        threshold: 1_000_000,
        ppm: 1000,
        capacity: 1_000_000_000,
        generation: 1,
        scanned_at: 1,
        has_hardlinks: false,
    };
    let good = d.path().join("good.db");
    store::write(&good, &meta, &nodes).unwrap();
    store::read(&good).unwrap();
    let cases = [
        (
            "gap",
            "DELETE FROM buckets WHERE hi = 4096",
            "cover all slots",
        ),
        (
            "totals",
            "UPDATE nodes SET files = files + 1",
            "do not match",
        ),
        (
            "format",
            "UPDATE meta SET format = 1",
            "unsupported snapshot format",
        ),
        (
            "orphan",
            "INSERT INTO buckets VALUES(x'00',0,4096,0,0,0,0,0,0,0,NULL,NULL)",
            "missing or non-directory",
        ),
    ];
    for (name, sql, expected) in cases {
        let copy = d.path().join(format!("{name}.db"));
        fs::copy(&good, &copy).unwrap();
        tamper(&copy, sql);
        let error = store::read(&copy).err().unwrap().to_string();
        assert!(error.contains(expected), "{name}: {error}");
    }
}
