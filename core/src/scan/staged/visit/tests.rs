use super::*;
use crate::scan::{MountRow, NO_PARENT};
use crate::snapshot::v2::{Reader, Store};
use std::os::unix::ffi::OsStrExt;
use std::time::Duration;

fn fixture() -> Tree {
    let mut tree = Tree {
        started_at: 100,
        ..Default::default()
    };
    let rows: &[(&[u8], u32, Kind, u32, u32)] = &[
        (b"/data", NO_PARENT, Kind::Dir, 2, 3),
        (b"/data/nested", NO_PARENT, Kind::Dir, 5, 1),
        (b"mount", 0, Kind::Mount, 0, 0),
        (b"nested", 0, Kind::Dir, 6, 1),
        (b"unknown-\xff", 0, Kind::Dir, 0, 0),
        (b"x", 1, Kind::File, 0, 0),
        (b"x", 3, Kind::File, 0, 0),
    ];
    for &(name, parent, kind, child_start, child_count) in rows {
        let (name_off, name_len) = tree.push_name(name);
        tree.nodes.push(Node {
            name_off,
            name_len,
            parent,
            kind: kind.code(),
            child_start,
            child_count,
            allocated: if kind == Kind::Mount { 0 } else { 11 },
            apparent: 7,
            errors: 1,
            scanned_at: 100,
            mtime: -30,
            nlink: 1,
            ..Default::default()
        });
    }
    tree.nodes[4].flags = flags::DENIED;
    tree.nodes[4].errors = u32::MAX;
    for id in [5, 6] {
        tree.nodes[id].nlink = 2;
    }
    tree.hardlinks = vec![(5, 7, 9), (6, 7, 9)];
    for (path, dev) in [("/data", 1), ("/data/nested", 2)] {
        tree.roots.push(RootInfo {
            path: path.into(),
            dev,
            fs_type: "fixture".into(),
            stats: platform::FsStats::default(),
        });
    }
    tree.mounts.push(MountRow {
        raw: 2,
        path: "/data/mount".into(),
        fs_type: "tmpfs".into(),
        stats: None,
    });
    tree
}
fn staged(tree: &Tree, path: &Path) -> StagedTree {
    let conn = open_database(path).unwrap();
    conn.execute_batch("CREATE TABLE raw(id INTEGER PRIMARY KEY,parent,name,kind,flags,allocated,apparent,mtime,nlink,errors,scanned_at);
        CREATE INDEX raw_children ON raw(parent,name,id);
        CREATE TABLE links(id INTEGER PRIMARY KEY,dev,ino,path,root);
        CREATE INDEX links_owner ON links(dev,ino,path,root,id);
        CREATE TABLE mount_info(id INTEGER PRIMARY KEY,path,fs_type,fs_used,fs_total);").unwrap();
    let staged = StagedTree {
        conn,
        path: path.to_owned(),
        roots: Tree {
            roots: tree.roots.clone(),
            started_at: tree.started_at,
            ..Default::default()
        },
        count: tree.nodes.len() as u64,
        one_file_system: false,
        reserve: Reserve {
            dir: path.parent().unwrap().to_owned(),
            min_free: 0,
            check_every: 1024,
        },
        cleanup: true,
    };
    for (id, node) in tree.nodes.iter().enumerate() {
        staged
            .insert(
                id as u64,
                (node.parent != NO_PARENT).then_some(node.parent as u64),
                tree.name(node),
                node,
            )
            .unwrap();
    }
    // Smaller raw ID belongs to the later root. Root ordinal resolves ties.
    for (id, root) in [(5, 1), (6, 0)] {
        staged
            .conn
            .execute(
                "INSERT INTO links VALUES(?1,7,9,?2,?3)",
                params![id, b"/data/nested/x".as_slice(), root],
            )
            .unwrap();
    }
    staged
        .conn
        .execute(
            "INSERT INTO mount_info VALUES(2,?1,'tmpfs',NULL,NULL)",
            [b"/data/mount".as_slice()],
        )
        .unwrap();
    staged
}
#[test]
fn raw_visitor_preserves_scoped_alias_mount_denied_and_unsaturated_own_values() {
    let tmp = tempfile::tempdir().unwrap();
    let memory_path = tmp.path().join("memory.db");
    let disk_path = tmp.path().join("disk.db");
    let tree = fixture();
    let staged = staged(&tree, &tmp.path().join("stage.tmp"));
    Store::open(&memory_path)
        .unwrap()
        .seed_tree(tree, false, Instant::now())
        .unwrap();
    Store::open(&disk_path)
        .unwrap()
        .seed_disk(&staged, false, true, Duration::ZERO)
        .unwrap();
    let memory = Reader::open(&memory_path).unwrap();
    let disk = Reader::open(&disk_path).unwrap();
    assert_eq!(disk.roots(false).unwrap(), memory.roots(false).unwrap());
    for root in disk.roots(false).unwrap() {
        assert_eq!(
            disk.subtree(&root.path, false).unwrap(),
            memory.subtree(&root.path, false).unwrap()
        );
    }
    let outer = disk.subtree(b"/data", false).unwrap();
    assert!(outer[0].errors > u32::MAX as i64);
    assert_eq!(outer[0].own_errors, 1);
    assert_eq!(outer[0].own_allocated, 11);
    let owner = outer.iter().find(|e| e.path == b"/data/nested/x").unwrap();
    let alias = disk.get(b"/data/nested/x", false).unwrap().unwrap();
    assert_eq!(owner.allocated, 11);
    assert_eq!(alias.allocated, 0);
    assert_eq!(alias.own_allocated, 11);
    assert_eq!(alias.newest_mtime, Some(-30));
    assert_ne!(owner.scope, alias.scope);
    let mount = outer.iter().find(|e| e.kind == Kind::Mount).unwrap();
    assert_eq!(
        disk.mount(mount.id, false).unwrap(),
        memory.mount(mount.id, false).unwrap()
    );
    assert_eq!(
        disk.unknown_subtrees(false, 10).unwrap(),
        memory.unknown_subtrees(false, 10).unwrap()
    );
    assert_eq!(staged.roots.roots[0].path.as_os_str().as_bytes(), b"/data");
}

#[test]
fn leaf_rows_keep_enter_leave_callbacks_and_parent_state() {
    type Event = (bool, Vec<u8>, Option<Vec<u8>>, Kind);
    struct Trace(Vec<Event>);
    impl Visitor for Trace {
        type State = (Vec<u8>, Kind);

        fn enter(&mut self, row: Row<'_>, parent: Option<&Self::State>) -> Result<Self::State> {
            let state = (row.path.to_vec(), row.node.kind());
            self.0
                .push((true, state.0.clone(), parent.map(|p| p.0.clone()), state.1));
            Ok(state)
        }

        fn leave(&mut self, state: Self::State, parent: Option<&mut Self::State>) -> Result<()> {
            self.0
                .push((false, state.0, parent.map(|p| p.0.clone()), state.1));
            Ok(())
        }
    }
    fn event(enter: bool, path: &[u8], parent: Option<&[u8]>, kind: Kind) -> Event {
        (enter, path.to_vec(), parent.map(<[u8]>::to_vec), kind)
    }

    let tmp = tempfile::tempdir().unwrap();
    let mut tree = fixture();
    tree.nodes[5].kind = Kind::Symlink.code();
    tree.nodes[5].nlink = 1;
    let staged = staged(&tree, &tmp.path().join("stage.tmp"));
    let mut trace = Trace(Vec::new());
    staged.visit(&mut trace).unwrap();
    assert_eq!(
        trace.0,
        vec![
            event(true, b"/data", None, Kind::Dir),
            event(true, b"/data/mount", Some(b"/data"), Kind::Mount),
            event(false, b"/data/mount", Some(b"/data"), Kind::Mount),
            event(true, b"/data/nested", Some(b"/data"), Kind::Dir),
            event(true, b"/data/nested/x", Some(b"/data/nested"), Kind::File),
            event(false, b"/data/nested/x", Some(b"/data/nested"), Kind::File),
            event(false, b"/data/nested", Some(b"/data"), Kind::Dir),
            event(true, b"/data/unknown-\xff", Some(b"/data"), Kind::Dir),
            event(false, b"/data/unknown-\xff", Some(b"/data"), Kind::Dir),
            event(false, b"/data", None, Kind::Dir),
            event(true, b"/data/nested", None, Kind::Dir),
            event(
                true,
                b"/data/nested/x",
                Some(b"/data/nested"),
                Kind::Symlink
            ),
            event(
                false,
                b"/data/nested/x",
                Some(b"/data/nested"),
                Kind::Symlink
            ),
            event(false, b"/data/nested", None, Kind::Dir),
        ]
    );
}

#[test]
fn reserve_failure_happens_before_generation_mutation() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = fixture();
    let mut staged = staged(&tree, &tmp.path().join("stage.tmp"));
    let db = tmp.path().join("v2.db");
    let mut store = Store::open(&db).unwrap();
    store.seed_tree(tree, false, Instant::now()).unwrap();
    let pinned = Reader::open(&db).unwrap();
    let count = store.retained_records().unwrap();
    staged.reserve.min_free = u64::MAX;
    assert_eq!(
        store
            .seed_disk(&staged, false, false, Duration::ZERO)
            .unwrap_err()
            .code,
        crate::ErrorCode::Space
    );
    let now = Reader::open(&db).unwrap();
    assert_eq!(now.generations, pinned.generations);
    assert_eq!(
        now.subtree(b"/data", false).unwrap(),
        pinned.subtree(b"/data", false).unwrap()
    );
    assert_eq!(store.retained_records().unwrap(), count);
}
