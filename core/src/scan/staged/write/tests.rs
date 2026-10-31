use super::*;
use crate::scan::walk::WalkOptions;

#[test]
fn production_writer_owner_query_uses_compact_index() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(STAGE_SCHEMA).unwrap();
    assert_index_plan(
        &conn,
        WRITE_LINK_SQL,
        &[rusqlite::types::Value::Integer(7)],
        "links_owner",
    );
}

#[test]
fn writer_deep_wide_pressure_never_refetches_buffered_rows() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let mut dir = root.clone();
    for _ in 0..10 {
        for i in 0..40 {
            std::fs::create_dir(dir.join(format!("z-{i:03}"))).unwrap();
        }
        dir = dir.join("0-next");
        std::fs::create_dir(&dir).unwrap();
    }
    let reserve = Reserve {
        dir: temp.path().to_owned(),
        min_free: 0,
        check_every: 1024,
    };
    let stage = StagedTree::walk(
        &WalkOptions::new(vec![root]),
        &temp.path().join("stage.db"),
        &reserve,
    )
    .unwrap();
    let output = temp.path().join("snapshot.db");
    let (stats, _, errors) = stage
        .write(&output, &WriteMeta::new(1, None, 123456))
        .unwrap();
    assert_eq!((stats.rows, errors), (411, 0));
    let (selects, rows) = batch::WRITER_COUNTS.get();
    assert_eq!(rows, 410, "every child row is read exactly once");
    assert!(
        selects < 821,
        "directory lookups must remain bounded: {selects}"
    );
    let conn = Connection::open(output).unwrap();
    let (count, items): (i64, i64) = conn
        .query_row(
            "SELECT count(*), (SELECT items FROM entries WHERE id=0) FROM entries",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((count, items), (411, 410));
}

#[test]
fn stat_error_leaf_is_written_and_folded_into_parent() {
    for kind in [Kind::File, Kind::Other, Kind::Symlink] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("leaf"), b"payload").unwrap();
        let reserve = Reserve {
            dir: temp.path().to_owned(),
            min_free: 0,
            check_every: 1024,
        };
        let stage = StagedTree::walk(
            &WalkOptions::new(vec![root]),
            &temp.path().join("stage.db"),
            &reserve,
        )
        .unwrap();
        assert_eq!(
            stage.conn.execute(
                "UPDATE raw SET kind=?1, flags=?2, allocated=0, apparent=0, errors=1, scanned_at=123456 WHERE parent IS NOT NULL",
                params![kind.code(), flags::STAT_ERROR],
            ).unwrap(),
            1
        );
        let output = temp.path().join("snapshot.db");
        let (_, _, errors) = stage
            .write(&output, &WriteMeta::new(1, None, 123456))
            .unwrap();
        assert_eq!(errors, 1);
        let conn = Connection::open(&output).unwrap();
        let leaf: (i64, i64, i64, i64, i64, i64, i64) = conn
            .query_row(
                "SELECT id, flags, allocated, apparent, errors, subtree_end, min_scanned_at FROM entries WHERE parent_id=0",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
            .unwrap();
        assert_eq!(leaf, (1, i64::from(flags::STAT_ERROR), 0, 0, 1, 1, 123456));
        let parent_errors: i64 = conn
            .query_row("SELECT errors FROM entries WHERE id=0", [], |r| r.get(0))
            .unwrap();
        assert_eq!(parent_errors, 1);
    }
}

#[test]
fn mount_row_is_written_and_folded_into_parent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("mount"), b"payload").unwrap();
    let reserve = Reserve {
        dir: temp.path().to_owned(),
        min_free: 0,
        check_every: 1024,
    };
    let stage = StagedTree::walk(
        &WalkOptions::new(vec![root.clone()]),
        &temp.path().join("stage.db"),
        &reserve,
    )
    .unwrap();
    stage
        .conn
        .execute(
            "UPDATE raw SET kind=?1,allocated=0,apparent=0,errors=1,scanned_at=123456 WHERE id=1",
            [Kind::Mount.code()],
        )
        .unwrap();
    stage
        .conn
        .execute(
            "INSERT INTO mount_info VALUES (1,?1,'testfs',12,34)",
            [root.join("mount").as_os_str().as_bytes()],
        )
        .unwrap();
    let output = temp.path().join("snapshot.db");
    let (_, _, errors) = stage
        .write(&output, &WriteMeta::new(1, None, 123456))
        .unwrap();
    assert_eq!(errors, 1);
    let conn = Connection::open(output).unwrap();
    let entry: (i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT kind,allocated,apparent,errors,subtree_end FROM entries WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(entry, (i64::from(Kind::Mount.code()), 0, 0, 1, 1));
    let mount: (Vec<u8>, String, i64, i64) = conn
        .query_row(
            "SELECT path,fs_type,fs_used,fs_total FROM mounts WHERE entry_id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        mount,
        (
            root.join("mount").as_os_str().as_bytes().to_vec(),
            "testfs".into(),
            12,
            34
        )
    );
    let parent_errors: i64 = conn
        .query_row("SELECT errors FROM entries WHERE id=0", [], |r| r.get(0))
        .unwrap();
    assert_eq!(parent_errors, 1);
}
