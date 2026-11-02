use super::*;
use std::io::Write;
use std::os::unix::fs::MetadataExt;

#[test]
fn integer_threshold_rounds_up_and_rejects_bad_inputs() {
    assert_eq!(threshold(1001, 1000).unwrap(), 2);
    assert!(threshold(0, 1000).is_err());
    assert!(threshold(5, 0).is_err());
    assert!(threshold(5, 1_000_001).is_err());
}
#[test]
fn small_children_fold_and_zero_file_counts() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a"), b"").unwrap();
    fs::create_dir(root.join("tiny")).unwrap();
    fs::write(root.join("tiny/b"), b"x").unwrap();
    let result = walk::scan(&root, u64::MAX).unwrap();
    assert_eq!(result.nodes.len(), 1);
    let n = &result.nodes[0];
    assert_eq!((n.total.files, n.total.dirs), (2, 2));
    assert_eq!((n.rest.files, n.rest.dirs), (2, 1));
    assert_eq!(n.total.allocated, n.own_allocated + n.rest.allocated);
}
#[test]
fn retained_hierarchy_conserves_and_handles_sparse() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    fs::create_dir_all(root.join("a/b")).unwrap();
    let file = fs::File::create(root.join("a/b/sparse")).unwrap();
    file.set_len(1024 * 1024).unwrap();
    let mut result = walk::scan(&root, 1).unwrap().nodes;
    let before = result
        .iter()
        .find(|n| n.path == root)
        .unwrap()
        .total
        .clone();
    rebuild(&mut result).unwrap();
    let after = &result.iter().find(|n| n.path == root).unwrap().total;
    assert_eq!(
        (before.allocated, before.apparent, before.files, before.dirs),
        (after.allocated, after.apparent, after.files, after.dirs)
    );
    assert!(after.apparent >= 1024 * 1024);
}
#[test]
fn hardlinks_choose_lexical_owner() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    fs::create_dir(&root).unwrap();
    let mut f = fs::File::create(root.join("z")).unwrap();
    f.write_all(b"hello").unwrap();
    drop(f);
    fs::hard_link(root.join("z"), root.join("a")).unwrap();
    let result = walk::scan(&root, 1).unwrap();
    assert!(result.has_hardlinks);
    let a = result
        .nodes
        .iter()
        .find(|n| n.path == root.join("a"))
        .unwrap();
    let z = result
        .nodes
        .iter()
        .find(|n| n.path == root.join("z"))
        .unwrap();
    assert_eq!(
        a.total.allocated,
        fs::metadata(root.join("a")).unwrap().blocks() * 512
    );
    assert_eq!(z.total.allocated, 0);
}
#[test]
fn refresh_folds_boundary_and_rebuilds_ancestors() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    fs::create_dir_all(root.join("child")).unwrap();
    fs::write(root.join("child/file"), vec![1; 16384]).unwrap();
    let mut before = walk::scan(&root, 4096).unwrap().nodes;
    assert!(before.iter().any(|n| n.path == root.join("child")));
    fs::remove_file(root.join("child/file")).unwrap();
    let replacement = walk::scan(&root.join("child"), u64::MAX).unwrap().nodes;
    update(&mut before, &root.join("child"), replacement, u64::MAX).unwrap();
    let expected = walk::scan(&root, u64::MAX).unwrap().nodes;
    let a = &before.iter().find(|n| n.path == root).unwrap().total;
    let b = &expected[0].total;
    assert_eq!(
        (a.allocated, a.apparent, a.files, a.dirs),
        (b.allocated, b.apparent, b.files, b.dirs)
    );
}
#[test]
fn store_roundtrip_and_prev_generation() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    let snapshot = d.path().join("out");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&snapshot).unwrap();
    let nodes = walk::scan(&root, 1).unwrap().nodes;
    let mut m = store::Meta {
        root: root.clone(),
        threshold: 1,
        ppm: 1000,
        capacity: 1000,
        generation: 1,
        scanned_at: 1,
        has_hardlinks: false,
    };
    let tmp = snapshot.join(".coarse.tmp");
    store::write(&tmp, &m, &nodes).unwrap();
    store::publish(&snapshot, &tmp).unwrap();
    m.generation = 2;
    store::write(&tmp, &m, &nodes).unwrap();
    store::publish(&snapshot, &tmp).unwrap();
    assert_eq!(
        store::read(&snapshot.join("coarse.db"))
            .unwrap()
            .0
            .generation,
        2
    );
    assert_eq!(
        store::read(&snapshot.join("coarse.prev.db"))
            .unwrap()
            .0
            .generation,
        1
    );
}

#[test]
fn rejects_depth_beyond_limit() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    fs::create_dir(&root).unwrap();
    let mut path = root.clone();
    for _ in 0..model::MAX_DEPTH {
        path.push("d");
        fs::create_dir(&path).unwrap();
    }
    assert!(walk::scan(&root, 1).is_err());
}

#[test]
fn refresh_promotes_omitted_directory() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    fs::create_dir_all(root.join("child")).unwrap();
    let mut old = walk::scan(&root, 1_000_000).unwrap().nodes;
    assert_eq!(old.len(), 1);
    fs::write(root.join("child/big"), vec![7; 1_100_000]).unwrap();
    let replacement = walk::scan(&root, 1_000_000).unwrap().nodes;
    update(&mut old, &root, replacement, 1_000_000).unwrap();
    assert!(old.iter().any(|n| n.path == root.join("child")));
    let total = &old.iter().find(|n| n.path == root).unwrap().total;
    assert_eq!((total.files, total.dirs), (1, 2));
}

#[test]
fn refresh_reports_root_fallback_for_omitted_directory() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    let snapshot = d.path().join("out");
    fs::create_dir_all(root.join("small")).unwrap();
    fs::create_dir_all(root.join("large")).unwrap();
    fs::create_dir(&snapshot).unwrap();
    fs::write(root.join("large/big"), vec![1; 131_072]).unwrap();
    let nodes = walk::scan(&root, 65_536).unwrap().nodes;
    assert!(nodes.iter().any(|n| n.path == root.join("large")));
    assert!(!nodes.iter().any(|n| n.path == root.join("small")));
    let meta = store::Meta {
        root: root.clone(),
        threshold: 65_536,
        ppm: 1000,
        capacity: 65_536_000,
        generation: 1,
        scanned_at: now(),
        has_hardlinks: false,
    };
    store::write(&snapshot.join("coarse.db"), &meta, &nodes).unwrap();
    let small = execute(Command::Refresh {
        root: root.clone(),
        snapshot: snapshot.clone(),
        target: root.join("small"),
    })
    .unwrap();
    assert_eq!(small["fallback_full_root"], true);
    assert_eq!(small["boundary"], root.to_string_lossy().as_ref());
    assert!(small["visited"].as_u64().unwrap() > 0);
    let large = execute(Command::Refresh {
        root: root.clone(),
        snapshot,
        target: root.join("large"),
    })
    .unwrap();
    assert_eq!(large["fallback_full_root"], false);
    assert_eq!(
        large["boundary"],
        root.join("large").to_string_lossy().as_ref()
    );
    assert!(large["visited"].as_u64().unwrap() > 0);
}
