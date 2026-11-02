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
    // Threshold zero exposes both paths to verify ownership, independently of folding.
    let result = walk::scan(&root, 0).unwrap();
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
    let coarse = walk::scan(&root, 1).unwrap();
    assert!(!coarse.nodes.iter().any(|n| n.path == root.join("z")));
    assert_eq!(
        coarse
            .nodes
            .iter()
            .find(|n| n.path == root)
            .unwrap()
            .rest
            .files,
        1
    );
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

#[test]
fn refresh_new_cross_boundary_hardlink_falls_back_to_root() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("tree");
    let snapshot = d.path().join("out");
    fs::create_dir_all(root.join("large")).unwrap();
    fs::create_dir(&snapshot).unwrap();
    fs::write(root.join("outside"), vec![1; 131_072]).unwrap();
    fs::write(root.join("large/keep"), vec![2; 131_072]).unwrap();
    let nodes = walk::scan(&root, 65_536).unwrap().nodes;
    assert!(nodes.iter().any(|n| n.path == root.join("large")));
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
    fs::hard_link(root.join("outside"), root.join("large/link")).unwrap();
    let result = execute(Command::Refresh {
        root: root.clone(),
        snapshot: snapshot.clone(),
        target: root.join("large"),
    })
    .unwrap();
    assert_eq!(result["fallback_full_root"], true);
    assert_eq!(result["boundary"], root.to_string_lossy().as_ref());
    assert!(result["visited"].as_u64().unwrap() >= 2 * 5);
    assert!(
        store::read(&snapshot.join("coarse.db"))
            .unwrap()
            .0
            .has_hardlinks
    );
}

#[test]
fn rebuild_merges_and_recomputes_freshness_extrema() {
    let root = PathBuf::from("/root");
    let mut parent = Node {
        path: root.clone(),
        parent: None,
        kind: 1,
        own_allocated: 0,
        own_apparent: 0,
        own_errors: 0,
        total: model::Stats::default(),
        rest: model::Stats::default(),
        scanned_at: 100,
    };
    parent.rest.min_scanned_at = Some(40);
    parent.rest.newest_mtime = Some(75);
    let child = Node {
        path: root.join("file"),
        parent: Some(root),
        kind: 0,
        own_allocated: 0,
        own_apparent: 0,
        own_errors: 0,
        total: model::Stats {
            newest_mtime: Some(85),
            ..Default::default()
        },
        rest: model::Stats::default(),
        scanned_at: 90,
    };
    let mut nodes = vec![parent, child];
    rebuild(&mut nodes).unwrap();
    assert_eq!(nodes[0].total.min_scanned_at, Some(40));
    assert_eq!(nodes[0].total.newest_mtime, Some(85));
    nodes[0].rest.min_scanned_at = Some(60);
    nodes[0].rest.newest_mtime = Some(70);
    nodes[1].scanned_at = 95;
    nodes[1].total.newest_mtime = Some(65);
    rebuild(&mut nodes).unwrap();
    assert_eq!(nodes[0].total.min_scanned_at, Some(60));
    assert_eq!(nodes[0].total.newest_mtime, Some(70));
}
