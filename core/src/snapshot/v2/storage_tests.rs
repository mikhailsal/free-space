use super::*;

fn components(path: &[u8]) -> Vec<&[u8]> {
    path.split(|b| *b == b'/')
        .filter(|c| !c.is_empty())
        .collect()
}

fn paths() -> Vec<Vec<u8>> {
    let mut paths = vec![b"/".to_vec()];
    for byte in 1..=255 {
        if byte == b'/' || byte == b'.' {
            continue;
        }
        paths.push(vec![b'/', byte]);
        paths.push(vec![b'/', b'a', b'/', byte]);
    }
    for path in [b"/a".as_slice(), b"/aa", b"/a/a", b"/a/aa", b"/a-/x"] {
        paths.push(path.to_vec());
    }
    paths
}

#[test]
fn compact_keys_follow_independent_raw_component_order() {
    let paths = paths();
    for path in &paths {
        validate_path(path).unwrap();
        assert!(key(path) < vec![255], "{path:?}");
        assert!(key(path).len() <= path.len() + 1);
        for other in &paths {
            assert_eq!(
                key(path).cmp(&key(other)),
                components(path).cmp(&components(other)),
                "{path:?} versus {other:?}"
            );
        }
    }
    assert!(key(b"/").is_empty());
    assert_eq!(key(b"/\xff/\x01"), vec![0, 255, 0, 1, 0]);
}

#[test]
fn compact_subtree_bounds_match_component_prefixes() {
    let paths = paths();
    for parent in &paths {
        let lower = key(parent);
        let upper = if lower.is_empty() {
            vec![255]
        } else {
            prefix_end(&lower)
        };
        for child in &paths {
            let candidate = key(child);
            assert_eq!(
                candidate >= lower && candidate < upper,
                components(child).starts_with(&components(parent)),
                "{parent:?} versus {child:?}"
            );
        }
    }
}

fn plan(conn: &Connection, sql: &str, bindings: impl rusqlite::Params) -> String {
    let mut query = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    query
        .query_map(bindings, |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join(" ")
}

#[test]
fn immutable_paths_exist_only_in_identity_storage() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("v2.db");
    let store = Store::open(&db).unwrap();
    for table in ["live_rows", "version_rows"] {
        let mut query = store
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let columns = query
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for immutable in ["path", "scope", "sort_key"] {
            assert!(
                !columns.iter().any(|c| c == immutable),
                "{table}: {columns:?}"
            );
        }
    }
    for view in ["live", "versions"] {
        let statement = store
            .conn
            .prepare(&format!("SELECT {COLUMNS} FROM {view}"))
            .unwrap();
        assert_eq!(
            statement.column_names(),
            COLUMNS.split(',').collect::<Vec<_>>()
        );
    }
    let sql: String = store
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='version_rows'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sql.contains("WITHOUT ROWID"));
    let id = identity(&store.conn, b"/", b"/\xff").unwrap();
    assert_eq!(identity(&store.conn, b"/", b"/\xff").unwrap(), id);
    let stored: Vec<u8> = store
        .conn
        .query_row("SELECT sort_key FROM identities WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stored, key(b"/\xff"));
    assert_eq!(store.retained_records().unwrap(), 1);
}

#[test]
fn normalized_query_plans_use_identity_ranges_and_sparse_denied_rows() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("v2.db")).unwrap();
    for sql in [
        "SELECT id FROM versions WHERE scope=?1 AND path=?2 AND born<=?3",
        "SELECT id FROM versions WHERE scope=?1 AND sort_key>=?2 AND sort_key<?4 AND born<=?3 ORDER BY sort_key",
    ] {
        let bindings = if sql.contains("?4") { 4 } else { 3 };
        let values = [
            rusqlite::types::Value::Blob(b"/".to_vec()),
            rusqlite::types::Value::Blob(vec![0]),
            rusqlite::types::Value::Integer(1),
            rusqlite::types::Value::Blob(vec![255]),
        ];
        let plan = plan(
            &store.conn,
            sql,
            rusqlite::params_from_iter(&values[..bindings]),
        );
        assert!(plan.contains("SEARCH i USING"), "{plan}");
        assert!(plan.contains("SEARCH r USING PRIMARY KEY"), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }
    for (sql, bindings) in [
        (
            reader::DENIED_QUERY,
            vec![
                rusqlite::types::Value::Blob(b"/".to_vec()),
                rusqlite::types::Value::Integer(1),
            ],
        ),
        (
            reader::UNKNOWN_SUBTREES_QUERY,
            vec![rusqlite::types::Value::Integer(1)],
        ),
    ] {
        let plan = plan(&store.conn, sql, rusqlite::params_from_iter(bindings));
        assert!(
            plan.contains("SCAN v USING INDEX versions_denied"),
            "{plan}"
        );
        assert!(
            plan.contains("SEARCH i USING INTEGER PRIMARY KEY"),
            "{plan}"
        );
    }
}

#[test]
fn revision_four_is_rejected_without_migration() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("old.db");
    drop(Store::open(&db).unwrap());
    let conn = Connection::open(&db).unwrap();
    conn.pragma_update(None, "user_version", 4).unwrap();
    assert_eq!(
        Store::open(&db).err().unwrap().code,
        ErrorCode::NotSupported
    );
    assert_eq!(Reader::open(&db).err().unwrap().code, ErrorCode::Schema);
    let revision: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(revision, 4);
}

#[test]
fn current_subtree_work_does_not_follow_retained_deleted_identities() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let mut steps = Vec::new();
    for width in [8, 2000] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let target = root.join("local");
        std::fs::create_dir_all(&target).unwrap();
        for n in 0..width {
            std::fs::write(target.join(format!("old-{n:04}")), [1]).unwrap();
        }
        let db = dir.path().join("v2.db");
        let mut store = Store::open(&db).unwrap();
        store.scan(&root, 1).unwrap();
        store.scan(&root, 1).unwrap();
        let pinned = Reader::open(&db).unwrap();
        std::fs::remove_dir_all(&target).unwrap();
        std::fs::create_dir_all(target.join("remaining/nested")).unwrap();
        store.refresh(&target, 1).unwrap();
        let current = Reader::open(&db).unwrap();
        assert_eq!(
            pinned
                .subtree(target.as_os_str().as_bytes(), false)
                .unwrap()
                .len(),
            width + 1
        );
        assert_eq!(
            current
                .subtree(target.as_os_str().as_bytes(), true)
                .unwrap()
                .len(),
            width + 1
        );
        let expected = current
            .subtree(target.as_os_str().as_bytes(), false)
            .unwrap();
        assert_eq!(expected.len(), 3);
        let counter = Arc::new(AtomicU64::new(0));
        store.observe_vm_steps(Some(counter.clone())).unwrap();
        let rows = writer::old_subtree(&store.conn, &expected[0]).unwrap();
        steps.push(counter.load(Ordering::Relaxed));
        store.observe_vm_steps(None).unwrap();
        assert_eq!(rows, expected);
        let plan = plan(&store.conn, &writer::old_subtree_sql(), [expected[0].id]);
        assert!(
            plan.contains("SEARCH l USING COVERING INDEX live_child_"),
            "{plan}"
        );
        assert!(!plan.contains("SCAN i"), "{plan}");
    }
    assert!(steps[1] < steps[0] * 2, "retired identity work: {steps:?}");
}

#[test]
fn child_streams_preserve_raw_basename_order_across_history() {
    use std::ffi::OsStr;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    let names = [b"a".as_slice(), b"aa", b"a-", b"\x01", b"\xff", b"\xff\x01"];
    for name in names {
        std::fs::create_dir_all(root.join(OsStr::from_bytes(name)).join("grandchild/deeper"))
            .unwrap();
    }
    let db = dir.path().join("v2.db");
    let mut store = Store::open(&db).unwrap();
    for _ in 0..4 {
        store.scan(&root, 1).unwrap();
    }
    let reader = Reader::open(&db).unwrap();
    let parent = reader
        .get(root.as_os_str().as_bytes(), false)
        .unwrap()
        .unwrap();
    let mut expected = names.to_vec();
    expected.sort();
    for previous in [false, true] {
        let rows = reader.children(&parent.path, previous).unwrap();
        let actual: Vec<_> = rows
            .iter()
            .map(|e| e.path.rsplit(|b| *b == b'/').next().unwrap())
            .collect();
        assert_eq!(actual, expected);
        assert!(
            rows.windows(2)
                .all(|pair| pair[0].sort_key < pair[1].sort_key)
        );
        let plan = plan(
            &store.conn,
            &reader::children_sql(),
            params![parent.id, reader.generation(previous).unwrap()],
        );
        assert!(
            plan.contains("SEARCH r USING INDEX versions_children (parent=?)"),
            "{plan}"
        );
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }
}
