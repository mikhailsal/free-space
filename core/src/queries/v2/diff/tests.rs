use super::*;
use crate::snapshot::v2::Store;
use std::os::unix::ffi::OsStrExt;

#[test]
fn depth_one_stream_uses_parent_basename_index_without_sorting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    for n in 0..80 {
        std::fs::create_dir_all(root.join(format!("child-{n:03}/grandchild/deeper"))).unwrap();
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
    let (scope, lower, upper) = bounds(Some(&parent));
    let sql = level_sql(1);
    let conn = reader.connection();
    let mut query = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let plan = query
        .query_map(
            params![scope, lower, upper, reader.generations.current, parent.id],
            |r| r.get::<_, String>(3),
        )
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join(" ");
    assert!(
        plan.contains("SEARCH r USING INDEX versions_children (parent=?)"),
        "{plan}"
    );
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    for previous in [false, true] {
        let generation = reader.generation(previous).unwrap();
        let mut query = conn.prepare(&sql).unwrap();
        let rows = query
            .query_map(
                params![scope, lower, upper, generation, parent.id],
                Entry::row,
            )
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), 80);
        assert!(rows.iter().all(|e| e.parent == Some(parent.id)));
        assert!(
            rows.windows(2)
                .all(|pair| pair[0].sort_key < pair[1].sort_key)
        );
        assert_eq!(rows, reader.children(&parent.path, previous).unwrap());
    }
}
