use super::*;
use crate::platform::{FsStats, MountInfo};
use crate::scan::{MountRow, NO_PARENT, Node, RootInfo, flags};
use std::fs;

fn node(tree: &mut Tree, name: &[u8], parent: u32, kind: Kind) -> Node {
    let (name_off, name_len) = tree.push_name(name);
    Node {
        name_off,
        name_len,
        parent,
        kind: kind.code(),
        scanned_at: 100,
        nlink: 1,
        ..Default::default()
    }
}
fn nested_tree() -> Tree {
    let mut t = Tree::default();
    let mut outer = node(&mut t, b"/data", NO_PARENT, Kind::Dir);
    outer.child_start = 2;
    outer.child_count = 2;
    let mut inner = node(&mut t, b"/data/mnt", NO_PARENT, Kind::Dir);
    inner.child_start = 4;
    inner.child_count = 1;
    let mut denied = node(&mut t, b"denied", 0, Kind::Dir);
    denied.flags = flags::DENIED;
    denied.errors = 1;
    denied.child_start = 5;
    denied.child_count = 1;
    let mount = node(&mut t, b"mnt", 0, Kind::Mount);
    let mut file = node(&mut t, b"x", 1, Kind::File);
    file.allocated = 4096;
    file.apparent = 10;
    let mut child = node(&mut t, b"nested", 2, Kind::Dir);
    child.flags = flags::DENIED;
    child.errors = 1;
    t.nodes = vec![outer, inner, denied, mount, file, child];
    t.roots = vec![
        RootInfo {
            path: "/data".into(),
            dev: 1,
            fs_type: "ext4".into(),
            stats: FsStats {
                total: 1000,
                used: 100,
                ..Default::default()
            },
        },
        RootInfo {
            path: "/data/mnt".into(),
            dev: 2,
            fs_type: "tmpfs".into(),
            stats: FsStats {
                total: 2000,
                used: 200,
                ..Default::default()
            },
        },
    ];
    t.mounts = vec![MountRow {
        raw: 3,
        path: "/data/mnt".into(),
        fs_type: "tmpfs".into(),
        stats: Some(t.roots[1].stats),
    }];
    t
}

#[test]
fn nested_root_mount_identity_and_denied_metadata_are_generation_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("v2.db");
    let mut store = Store::open(&db).unwrap();
    store
        .seed_tree(nested_tree(), true, Instant::now())
        .unwrap();
    let pinned = Reader::open(&db).unwrap();
    let outer = pinned.subtree(b"/data", false).unwrap();
    assert_eq!(outer.len(), 4);
    assert_eq!(outer[0].allocated, 0);
    assert_eq!(outer[0].errors, 2);
    let placeholder = outer.iter().find(|e| e.kind == Kind::Mount).unwrap();
    let nested = pinned.get(b"/data/mnt", false).unwrap().unwrap();
    assert_ne!(placeholder.id, nested.id);
    assert_eq!(nested.kind, Kind::Dir);
    assert_eq!(nested.allocated, 4096);
    assert_eq!(pinned.subtree(b"/data/mnt", false).unwrap().len(), 2);
    assert_eq!(
        pinned.mount(placeholder.id, false).unwrap().unwrap().used,
        Some(200)
    );
    assert_eq!(
        pinned.unknown_subtrees(false, 10).unwrap(),
        (true, vec![b"/data/denied".to_vec()])
    );
    assert_eq!(pinned.unknown_subtrees(false, 0).unwrap(), (true, vec![]));
    let mut updated = nested_tree();
    updated.mounts[0].fs_type = "changed".into();
    updated.roots[1].stats.used = 300;
    updated.nodes[2].flags = 0;
    updated.nodes[5].flags = 0;
    store.seed_tree(updated, true, Instant::now()).unwrap();
    let now = Reader::open(&db).unwrap();
    assert_eq!(now.get(b"/data/mnt", false).unwrap().unwrap().id, nested.id);
    assert_eq!(
        now.mount(placeholder.id, false).unwrap().unwrap().fs_type,
        "changed"
    );
    assert_eq!(
        now.mount(placeholder.id, true).unwrap().unwrap().fs_type,
        "tmpfs"
    );
    assert_eq!(now.roots(true).unwrap()[1].stats.used, 200);
    assert_eq!(now.roots(false).unwrap()[1].stats.used, 300);
    assert!(!now.unknown_subtrees(false, 10).unwrap().0);
    assert!(pinned.unknown_subtrees(false, 10).unwrap().0);
}

#[test]
fn mount_records_replace_locally_and_old_readers_retain_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let local = root.join("local");
    let mount = local.join("mnt");
    fs::create_dir_all(&mount).unwrap();
    fs::write(mount.join("x"), [1]).unwrap();
    let db = dir.path().join("v2.db");
    let mut store = Store::open(&db).unwrap();
    store.scan(&root, 1).unwrap();
    store.scan(&root, 1).unwrap();
    let fake = vec![
        MountInfo {
            mount_point: root.clone(),
            fs_type: "ext4".into(),
            source: "disk".into(),
            dev: scan::walk::dev_string(fs::metadata(&root).unwrap().dev()),
            root: "/".into(),
        },
        MountInfo {
            mount_point: mount.clone(),
            fs_type: "tmpfs".into(),
            source: "disk".into(),
            dev: scan::walk::dev_string(fs::metadata(&root).unwrap().dev()),
            root: "/elsewhere".into(),
        },
    ];
    let mut prepared = store.prepare_refresh(&local, 1).unwrap();
    let mut opts = WalkOptions::new(vec![local.clone()]);
    opts.workers = 1;
    prepared.staged = stage(
        scan::walk_with_mounts(&opts, &fake).unwrap(),
        Some(root.as_os_str().as_bytes()),
    )
    .unwrap();
    assert_eq!(prepared.staged.mounts.len(), 1);
    store.publish_refresh(prepared).unwrap();
    let pinned = Reader::open(&db).unwrap();
    let m = pinned
        .get(mount.as_os_str().as_bytes(), false)
        .unwrap()
        .unwrap();
    assert_eq!(m.kind, Kind::Mount);
    assert!(pinned.mount(m.id, false).unwrap().is_some());
    let before = pinned.generations;
    assert!(store.refresh(&mount, 1).is_err());
    assert_eq!(Reader::open(&db).unwrap().generations, before);
    // The next real walk sees an ordinary directory and retires its mount record.
    store.refresh(&local, 1).unwrap();
    let now = Reader::open(&db).unwrap();
    assert_eq!(
        now.get(mount.as_os_str().as_bytes(), false)
            .unwrap()
            .unwrap()
            .kind,
        Kind::Dir
    );
    assert!(now.mount(m.id, false).unwrap().is_none());
    assert!(pinned.mount(m.id, false).unwrap().is_some());
}

#[test]
fn concurrent_prepared_writers_publish_one_and_reject_the_stale_base() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    for name in ["a", "b"] {
        fs::create_dir_all(root.join(name)).unwrap();
    }
    let db = dir.path().join("v2.db");
    let mut seed = Store::open(&db).unwrap();
    seed.scan(&root, 1).unwrap();
    seed.scan(&root, 1).unwrap();
    for name in ["a", "b"] {
        fs::write(root.join(name).join("new"), [1]).unwrap();
    }
    let mut a = Store::open(&db).unwrap();
    let mut b = Store::open(&db).unwrap();
    let pa = a.prepare_refresh(&root.join("a"), 1).unwrap();
    let pb = b.prepare_refresh(&root.join("b"), 1).unwrap();
    assert!(
        a.conn.is_autocommit() && b.conn.is_autocommit(),
        "preparation retains no transaction"
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other = barrier.clone();
    let left = std::thread::spawn(move || {
        barrier.wait();
        a.publish_refresh(pa)
    });
    let right = std::thread::spawn(move || {
        other.wait();
        b.publish_refresh(pb)
    });
    let results = [left.join().unwrap(), right.join().unwrap()];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results.iter().find_map(|r| r.as_ref().err()).unwrap().code,
        ErrorCode::Rotated
    );
    let r = Reader::open(&db).unwrap();
    assert_eq!(r.generations.current, 3);
    assert_eq!(r.generations.prev, Some(1));
    assert_eq!(r.refresh_history().unwrap().len(), 1);
    let a_exists = r
        .get(root.join("a/new").as_os_str().as_bytes(), false)
        .unwrap()
        .is_some();
    let b_exists = r
        .get(root.join("b/new").as_os_str().as_bytes(), false)
        .unwrap()
        .is_some();
    assert_ne!(a_exists, b_exists);
    seed.refresh(&root.join(if a_exists { "b" } else { "a" }), 1)
        .unwrap();
    assert_eq!(Reader::open(&db).unwrap().generations.current, 4);
}

#[test]
fn sqlite_full_during_refresh_preserves_current_previous_and_accounting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let local = root.join("local");
    fs::create_dir_all(&local).unwrap();
    fs::write(local.join("a"), [1]).unwrap();
    let db = dir.path().join("v2.db");
    let mut store = Store::open(&db).unwrap();
    store.scan(&root, 1).unwrap();
    store.scan(&root, 1).unwrap();
    let r = Reader::open(&db).unwrap();
    let pair = r.generations;
    let old = r.subtree(root.as_os_str().as_bytes(), false).unwrap();
    let old_prev = r.subtree(root.as_os_str().as_bytes(), true).unwrap();
    let count = store.retained_records().unwrap();
    drop(r);
    let pages: i64 = store
        .conn
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .unwrap();
    store
        .conn
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    for n in 0..300 {
        fs::write(local.join(format!("new-file-{n:04}")), [1]).unwrap();
    }
    let error = store.refresh(&local, 1).unwrap_err();
    assert!(error.message.contains("full"), "{error}");
    let r = Reader::open(&db).unwrap();
    assert_eq!(r.generations, pair);
    assert_eq!(r.subtree(root.as_os_str().as_bytes(), false).unwrap(), old);
    assert_eq!(
        r.subtree(root.as_os_str().as_bytes(), true).unwrap(),
        old_prev
    );
    assert!(r.refresh_history().unwrap().is_empty());
    assert_eq!(store.retained_records().unwrap(), count);
}
