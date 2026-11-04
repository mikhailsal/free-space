//! Refresh of targets that no longer exist, a parent directory whose own size changed, a child
//! that outgrows the entry limit, and structural damage in a stored snapshot. The tree helpers
//! are shared with `bucket_tests`.
use crate::bucket_tests::{make_tree, range_of, refresh_command, scan_command, shape};
use crate::model::BUCKET_ENTRIES;
use crate::*;

/// Write a snapshot of `root` made with an explicit threshold instead of a share of capacity.
fn seed(root: &Path, snapshot: &Path, threshold: u64) -> Vec<Node> {
    fs::create_dir_all(snapshot).unwrap();
    let nodes = walk::scan(root, threshold).unwrap().nodes;
    let meta = store::Meta {
        root: fs::canonicalize(root).unwrap(),
        threshold,
        ppm: 1000,
        capacity: 1_000_000_000,
        generation: 1,
        scanned_at: 1,
        has_hardlinks: false,
    };
    store::write(&snapshot.join("coarse.db"), &meta, &nodes).unwrap();
    nodes
}

#[test]
fn deleted_target_is_refreshed_through_the_bucket_that_held_it() {
    let d = tempfile::tempdir().unwrap();
    let (root, snapshot) = (d.path().join("tree"), d.path().join("out"));
    make_tree(&root, 50, 100);
    let scan = scan_command(&root, &snapshot);
    fs::remove_dir_all(root.join("d017")).unwrap();
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
    assert_eq!(old, new + 101, "the directory and its 100 files are gone");
    let (meta, nodes) = store::read(&snapshot.join("coarse.db")).unwrap();
    let full = walk::scan(&root, meta.threshold).unwrap();
    assert_eq!(shape(&nodes, &root), shape(&full.nodes, &root));
    assert_eq!((nodes[0].total.files, nodes[0].total.dirs), (4900, 50));
}

#[test]
fn deleted_target_below_a_surviving_directory_still_uses_its_bucket() {
    let d = tempfile::tempdir().unwrap();
    let (root, snapshot) = (d.path().join("tree"), d.path().join("out"));
    make_tree(&root, 50, 100);
    scan_command(&root, &snapshot);
    fs::remove_file(root.join("d017/f005")).unwrap();
    let out = refresh_command(&root, &snapshot, &root.join("d017/f005"));
    assert_eq!(
        (&out["scope"], &out["fallback_full_root"]),
        (&Value::from("range"), &Value::from(false))
    );
    let (meta, nodes) = store::read(&snapshot.join("coarse.db")).unwrap();
    let full = walk::scan(&root, meta.threshold).unwrap();
    assert_eq!(shape(&nodes, &root), shape(&full.nodes, &root));
    assert_eq!(nodes[0].total.files, 4999);
}

#[test]
fn deleted_retained_directory_falls_back_to_its_surviving_ancestor() {
    let d = tempfile::tempdir().unwrap();
    let (root, snapshot) = (d.path().join("tree"), d.path().join("out"));
    make_tree(&root, 3, 5);
    fs::write(root.join("d001/big"), vec![7; 1_100_000]).unwrap();
    let nodes = seed(&root, &snapshot, 1_000_000);
    assert!(nodes.iter().any(|n| n.path == root.join("d001")));
    fs::remove_dir_all(root.join("d001")).unwrap();
    let out = refresh_command(&root, &snapshot, &root.join("d001"));
    // Its own directory is gone, so the nearest survivor is rescanned and says so.
    assert_eq!(
        (&out["scope"], &out["fallback_full_root"]),
        (&Value::from("full"), &Value::from(true))
    );
    let (meta, nodes) = store::read(&snapshot.join("coarse.db")).unwrap();
    assert!(!nodes.iter().any(|n| n.path.starts_with(root.join("d001"))));
    let full = walk::scan(&root, meta.threshold).unwrap();
    assert_eq!(shape(&nodes, &root), shape(&full.nodes, &root));
}

#[test]
fn range_refresh_rereads_the_parent_directory_itself() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 50, 100);
    let mut nodes = walk::scan(&root, u64::MAX).unwrap().nodes;
    let (own, stamp) = (
        (nodes[0].own_allocated, nodes[0].own_apparent),
        nodes[0].scanned_at,
    );
    // Pretend the snapshot recorded the directory before it grew or shrank.
    nodes[0].own_allocated += 8192;
    nodes[0].own_apparent += 12_345;
    nodes[0].scanned_at -= 1000;
    state::rebuild(&mut nodes).unwrap();
    let slice = range_of(&nodes, &root.join("d017"));
    let scope = state::scope(&nodes, &slice.parent, slice.lo, slice.hi);
    let ranged = walk::scan_range(&slice.parent, u64::MAX, &scope).unwrap();
    state::update_range(&mut nodes, &slice.parent, &scope, ranged, u64::MAX).unwrap();
    assert_eq!((nodes[0].own_allocated, nodes[0].own_apparent), own);
    assert!(nodes[0].scanned_at >= stamp);
    let full = walk::scan(&root, u64::MAX).unwrap();
    assert_eq!(shape(&nodes, &root), shape(&full.nodes, &root));
    state::verify(&nodes).unwrap();
}

#[test]
fn range_refresh_promotes_a_child_that_passed_the_entry_limit() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 3, 5);
    let nodes = walk::scan(&root, u64::MAX).unwrap().nodes;
    assert_eq!(nodes.len(), 1);
    for f in 0..=BUCKET_ENTRIES {
        fs::File::create(root.join(format!("d001/more{f}"))).unwrap();
    }
    let slice = range_of(&nodes, &root.join("d001"));
    let scope = state::scope(&nodes, &slice.parent, slice.lo, slice.hi);
    let ranged = walk::scan_range(&slice.parent, u64::MAX, &scope).unwrap();
    let mut merged = nodes.clone();
    state::update_range(&mut merged, &slice.parent, &scope, ranged, u64::MAX).unwrap();
    let promoted = merged.iter().find(|n| n.path == root.join("d001")).unwrap();
    assert_eq!(promoted.parent.as_deref(), Some(root.as_path()));
    assert_eq!(promoted.rest().unwrap().files, 5 + BUCKET_ENTRIES + 1);
    // The root no longer counts d001 in any bucket.
    assert_eq!(merged[0].rest().unwrap().dirs, 2);
    let full = walk::scan(&root, u64::MAX).unwrap();
    assert_eq!(shape(&merged, &root), shape(&full.nodes, &root));
    state::verify(&merged).unwrap();
}

fn damage(nodes: &[Node], edit: impl FnOnce(&mut [Node])) -> String {
    let mut copy = nodes.to_vec();
    edit(&mut copy);
    state::verify(&copy).err().unwrap().to_string()
}

#[test]
fn verify_rejects_structural_damage() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    make_tree(&root, 2, 3);
    fs::write(root.join("d000/big"), vec![7; 1_100_000]).unwrap();
    fs::write(root.join("d001/big"), vec![7; 1_100_000]).unwrap();
    let nodes = walk::scan(&root, 1_000_000).unwrap().nodes;
    state::verify(&nodes).unwrap();
    let at = |name: &str| {
        nodes
            .iter()
            .position(|n| n.path == root.join(name))
            .unwrap()
    };
    let (dir, file, sibling) = (at("d000"), at("d000/big"), at("d001/big"));
    let cases = [
        (
            "two roots",
            damage(&nodes, |n| n[dir].parent = None),
            "exactly one root",
        ),
        (
            "wrong directory",
            damage(&nodes, |n| n[file].parent = Some(root.join("d001"))),
            "not a child",
        ),
        (
            "file as parent",
            damage(&nodes, |n| n[file].parent = Some(n[sibling].path.clone())),
            "not a child",
        ),
        (
            "missing parent",
            damage(&nodes, |n| n[file].parent = Some(root.join("nope"))),
            "missing parent",
        ),
        (
            "unknown kind",
            damage(&nodes, |n| n[file].kind = 9),
            "unknown node kind",
        ),
    ];
    for (name, error, expected) in cases {
        assert!(error.contains(expected), "{name}: {error}");
    }
}
