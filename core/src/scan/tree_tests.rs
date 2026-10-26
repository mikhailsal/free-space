use super::*;

fn add(t: &mut Tree, parent: u32, name: &str, kind: Kind, alloc: u64, mtime: i64) -> u32 {
    let (off, len) = t.push_name(name.as_bytes());
    t.nodes.push(Node {
        allocated: alloc,
        apparent: alloc,
        mtime,
        scanned_at: 1000,
        name_off: off,
        name_len: len,
        parent,
        kind: kind.code(),
        nlink: 1,
        ..Default::default()
    });
    (t.nodes.len() - 1) as u32
}

fn set_children(t: &mut Tree, dir: u32, start: u32, count: u32) {
    t.nodes[dir as usize].child_start = start;
    t.nodes[dir as usize].child_count = count;
}

/// /            (0)
///   a/         (1)  children 3,4
///   b          (2)  file 10
///   a/x        (3)  file 5 mtime 50
///   a/y        (4)  file 7 mtime 2000 (future vs scanned_at 1000)
fn sample() -> Tree {
    let mut t = Tree {
        roots: vec![RootInfo {
            path: "/".into(),
            dev: 1,
            fs_type: "ext4".into(),
            stats: FsStats::default(),
        }],
        ..Default::default()
    };
    add(&mut t, NO_PARENT, "/", Kind::Dir, 4, 0);
    add(&mut t, 0, "a", Kind::Dir, 4, 9999);
    add(&mut t, 0, "b", Kind::File, 10, 100);
    add(&mut t, 1, "x", Kind::File, 5, 50);
    add(&mut t, 1, "y", Kind::File, 7, 2000);
    set_children(&mut t, 0, 1, 2);
    set_children(&mut t, 1, 3, 2);
    t
}

#[test]
fn preorder_and_aggregates() {
    let mut t = sample();
    let f = t.finalize();
    assert_eq!(f.order, vec![0, 1, 3, 4, 2]);
    let root = t.nodes[0];
    assert_eq!(root.allocated, 4 + 4 + 10 + 5 + 7);
    assert_eq!(root.items, 4);
    let a = t.nodes[1];
    assert_eq!(a.allocated, 16);
    assert_eq!(a.items, 2);
    // y is future-dated: clamped to its scanned_at (D4); dir mtimes are excluded (D5).
    assert_eq!(a.newest_mtime, 1000);
    assert_eq!(root.newest_mtime, 1000);
    assert_eq!(f.path_hash[1], path_hash(b"/a"));
    assert_eq!(f.path_hash[2], path_hash(b"/a/x"));
    assert_eq!(f.path_hash[4], path_hash(b"/b"));
}

#[test]
fn newest_mtime_identity_is_min_not_zero() {
    // A dir whose only file has a negative mtime keeps the negative value;
    // a dir without files stores 0 ("unknown").
    let mut t = Tree {
        roots: vec![RootInfo {
            path: "/".into(),
            dev: 1,
            fs_type: "ext4".into(),
            stats: FsStats::default(),
        }],
        ..Default::default()
    };
    add(&mut t, NO_PARENT, "/", Kind::Dir, 0, 0);
    add(&mut t, 0, "empty", Kind::Dir, 0, 500);
    add(&mut t, 0, "old", Kind::File, 1, -86400);
    set_children(&mut t, 0, 1, 2);
    t.finalize();
    assert_eq!(t.nodes[0].newest_mtime, -86400);
    assert_eq!(t.nodes[1].newest_mtime, 0);
}

#[test]
fn hardlink_smallest_path_wins() {
    let mut t = sample();
    // b and a/x are the same inode; "/a/x" < "/b" byte-wise.
    t.nodes[2].nlink = 2;
    t.nodes[3].nlink = 2;
    t.nodes[2].allocated = 5;
    t.hardlinks = vec![(2, 1, 77), (3, 1, 77)];
    let f = t.finalize();
    assert_eq!(t.nodes[3].flags & flags::ALIAS, 0);
    assert_ne!(t.nodes[2].flags & flags::ALIAS, 0);
    assert_eq!(t.nodes[2].allocated, 0);
    assert_eq!(t.nodes[0].allocated, 4 + 4 + 5 + 7);
    assert_eq!(f.hardlinks.len(), 2);
    assert!(f.hardlinks.iter().all(|h| h.allocated == 5));
}

#[test]
fn byte_order_not_dfs_order_for_hardlinks() {
    // "/a-c" < "/a/b" byte-wise ('-' = 0x2d < '/' = 0x2f), although DFS visits a/b first.
    let mut t = Tree {
        roots: vec![RootInfo {
            path: "/".into(),
            dev: 1,
            fs_type: "ext4".into(),
            stats: FsStats::default(),
        }],
        ..Default::default()
    };
    add(&mut t, NO_PARENT, "/", Kind::Dir, 0, 0);
    add(&mut t, 0, "a", Kind::Dir, 0, 0);
    add(&mut t, 0, "a-c", Kind::File, 8, 0);
    add(&mut t, 1, "b", Kind::File, 8, 0);
    set_children(&mut t, 0, 1, 2);
    set_children(&mut t, 1, 3, 1);
    t.hardlinks = vec![(2, 1, 5), (3, 1, 5)];
    t.finalize();
    assert_eq!(t.nodes[2].flags & flags::ALIAS, 0, "/a-c wins");
    assert_ne!(t.nodes[3].flags & flags::ALIAS, 0);
    assert_eq!(t.nodes[1].allocated, 0);
}
