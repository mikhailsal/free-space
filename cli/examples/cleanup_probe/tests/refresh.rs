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

fn refresh_error(root: &Path, snapshot: &Path, target: PathBuf) -> String {
    let command = Command::Refresh {
        root: root.to_path_buf(),
        snapshot: snapshot.to_path_buf(),
        target,
    };
    execute(command).unwrap_err().to_string()
}

#[test]
fn targets_with_parent_components_are_refused() {
    let d = tempfile::tempdir().unwrap();
    let (root, snapshot) = (d.path().join("tree"), d.path().join("out"));
    make_tree(&root, 3, 5);
    scan_command(&root, &snapshot);
    let before = fs::read(snapshot.join("coarse.db")).unwrap();
    // `gone` does not exist, so `..` after it cannot be resolved: it must not pick a bucket
    // or slip past the root check by lexical comparison.
    for target in ["gone/../d001", "gone/../../../etc", "d001/../d002"] {
        let error = refresh_error(&root, &snapshot, root.join(target));
        assert!(error.contains("'..'"), "{target}: {error}");
    }
    assert_eq!(fs::read(snapshot.join("coarse.db")).unwrap(), before);
}

#[test]
fn targets_outside_the_root_are_refused() {
    let d = tempfile::tempdir().unwrap();
    let (root, snapshot, elsewhere) = (
        d.path().join("tree"),
        d.path().join("out"),
        d.path().join("elsewhere"),
    );
    make_tree(&root, 3, 5);
    fs::create_dir(&elsewhere).unwrap();
    // A link inside the tree whose target lies outside is resolved before the comparison.
    std::os::unix::fs::symlink(&elsewhere, root.join("escape")).unwrap();
    scan_command(&root, &snapshot);
    let before = fs::read(snapshot.join("coarse.db")).unwrap();
    let targets = [
        elsewhere.clone(),
        elsewhere.join("not-yet"),
        d.path().to_path_buf(),
        root.join("escape"),
    ];
    for target in targets {
        let error = refresh_error(&root, &snapshot, target.clone());
        assert!(
            error.contains("outside root"),
            "{}: {error}",
            target.display()
        );
    }
    assert_eq!(fs::read(snapshot.join("coarse.db")).unwrap(), before);
}

#[test]
fn relative_missing_targets_resolve_against_the_working_directory() {
    let (live, missing) = live_ancestor(Path::new("freespace-no-such-target")).unwrap();
    assert_eq!(live, fs::canonicalize(".").unwrap());
    assert_eq!(missing, Path::new("freespace-no-such-target"));
    let (live, missing) = live_ancestor(Path::new("freespace-no-such/dir/below")).unwrap();
    assert_eq!(live, fs::canonicalize(".").unwrap());
    assert_eq!(missing, Path::new("freespace-no-such/dir/below"));
}

#[test]
fn a_directory_replaced_by_a_file_is_refreshed_from_what_replaced_it() {
    let d = tempfile::tempdir().unwrap();
    let (root, snapshot) = (d.path().join("tree"), d.path().join("out"));
    make_tree(&root, 50, 100);
    scan_command(&root, &snapshot);
    // `d017` was folded into a bucket of the root; a path below it is now below a file.
    fs::remove_dir_all(root.join("d017")).unwrap();
    fs::write(root.join("d017"), b"now a file").unwrap();
    let out = refresh_command(&root, &snapshot, &root.join("d017/f005"));
    assert_eq!(out["scope"].as_str(), Some("range"));
    let (meta, nodes) = store::read(&snapshot.join("coarse.db")).unwrap();
    let full = walk::scan(&root, meta.threshold).unwrap();
    assert_eq!(shape(&nodes, &root), shape(&full.nodes, &root));
    // 49 surviving directories and the root itself; the replaced one now counts as a file.
    assert_eq!((nodes[0].total.files, nodes[0].total.dirs), (4901, 50));
}

#[test]
fn a_retained_directory_replaced_by_a_file_is_refreshed_from_what_replaced_it() {
    // Each target gets its own tree: the first refresh already repairs the snapshot. `big` is a
    // retained child, `f000` a folded one, and `d001` the replaced directory itself.
    for target in ["d001/big", "d001/f000", "d001"] {
        let d = tempfile::tempdir().unwrap();
        let (root, snapshot) = (d.path().join("tree"), d.path().join("out"));
        make_tree(&root, 3, 5);
        fs::write(root.join("d001/big"), vec![7; 1_100_000]).unwrap();
        let nodes = seed(&root, &snapshot, 1_000_000);
        assert!(nodes.iter().any(|n| n.path == root.join("d001")));
        fs::remove_dir_all(root.join("d001")).unwrap();
        fs::write(root.join("d001"), b"now a file").unwrap();
        refresh_command(&root, &snapshot, &root.join(target));
        let (meta, nodes) = store::read(&snapshot.join("coarse.db")).unwrap();
        assert!(!nodes.iter().any(|n| n.path.starts_with(root.join("d001"))));
        let full = walk::scan(&root, meta.threshold).unwrap();
        assert_eq!(shape(&nodes, &root), shape(&full.nodes, &root), "{target}");
        assert_eq!(
            (nodes[0].total.files, nodes[0].total.dirs),
            (11, 3),
            "{target}"
        );
    }
}

#[test]
fn read_rejects_a_root_other_than_the_one_in_the_metadata() {
    let d = tempfile::tempdir().unwrap();
    let (root, snapshot) = (d.path().join("tree"), d.path().join("out"));
    make_tree(&root, 2, 3);
    let nodes = seed(&root, &snapshot, 1_000_000);
    let meta = store::Meta {
        root: d.path().join("elsewhere"),
        threshold: 1_000_000,
        ppm: 1000,
        capacity: 1_000_000_000,
        generation: 1,
        scanned_at: 1,
        has_hardlinks: false,
    };
    let other = snapshot.join("other.db");
    store::write(&other, &meta, &nodes).unwrap();
    let error = store::read(&other).err().unwrap().to_string();
    assert!(error.contains("differs from its metadata"), "{error}");
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
    // A lone root, so that nothing else is rejected before the root itself is looked at.
    let single = walk::scan(&root, u64::MAX).unwrap().nodes;
    assert_eq!(single.len(), 1);
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
        (
            "file as root",
            damage(&single, |n| {
                n[0].kind = 0;
                n[0].buckets.clear();
            }),
            "root must be a directory",
        ),
    ];
    for (name, error, expected) in cases {
        assert!(error.contains(expected), "{name}: {error}");
    }
}
