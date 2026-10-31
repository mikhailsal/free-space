use super::super::SpaceGuard;
use super::{import_links, import_nodes, subtract_children};
use crate::snapshot::writer::Reserve;
use rusqlite::Connection;

fn source() -> Connection {
    let source = Connection::open_in_memory().unwrap();
    source
        .execute_batch(
            "CREATE TABLE entries (id INTEGER PRIMARY KEY,parent_id INTEGER,name BLOB NOT NULL,
             kind INTEGER,flags INTEGER,allocated INTEGER,apparent INTEGER,mtime INTEGER,
             nlink INTEGER,errors INTEGER,scanned_at INTEGER);
         CREATE TABLE hardlinks (entry_id INTEGER PRIMARY KEY,dev INTEGER,ino INTEGER,
             allocated INTEGER,apparent INTEGER);
         INSERT INTO entries VALUES
             (0,NULL,X'72',1,0,100,100,0,1,4,1),
             (1,0,X'64',1,0,80,80,0,1,3,1),
             (2,1,X'66',0,2,70,70,0,2,2,1),
             (3,0,X'67',0,0,50,50,0,1,3,1);
         INSERT INTO hardlinks VALUES (2,3,4,200,300);",
        )
        .unwrap();
    source
}

fn values(stage: &Connection, id: i64) -> (i64, i64, u32, u8) {
    stage
        .query_row(
            "SELECT allocated,apparent,errors,flags FROM old_raw WHERE old_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
}

#[test]
fn reconstructs_full_tree_before_applying_hardlink_sizes() {
    let source = source();
    let stage = Connection::open_in_memory().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let mut reserve = Reserve::new(tmp.path());
    reserve.min_free = 0;
    let mut guard = SpaceGuard::new(&reserve).unwrap();
    let n = import_nodes(&source, &stage, &mut guard).unwrap();
    assert_eq!(n, 4);
    subtract_children(&stage, n, &mut guard).unwrap();
    assert_eq!(values(&stage, 0), (0, 0, 0, 0));
    assert_eq!(values(&stage, 1), (10, 10, 1, 0));
    assert_eq!(values(&stage, 2), (70, 70, 2, 0));
    import_links(&source, &stage, &mut guard).unwrap();
    assert_eq!(values(&stage, 1), (10, 10, 1, 0));
    assert_eq!(values(&stage, 2), (200, 300, 2, 0));
    let owner: bool = stage
        .query_row("SELECT was_owner FROM old_link WHERE old_id=2", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!owner);
}

#[test]
fn subtraction_uses_unsigned_bit_patterns_and_u32_saturation() {
    let source = source();
    source
        .execute(
            "UPDATE entries SET allocated=-1,apparent=-1,errors=4294967295 WHERE id=0",
            [],
        )
        .unwrap();
    let stage = Connection::open_in_memory().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let mut reserve = Reserve::new(tmp.path());
    reserve.min_free = 0;
    let mut guard = SpaceGuard::new(&reserve).unwrap();
    let n = import_nodes(&source, &stage, &mut guard).unwrap();
    subtract_children(&stage, n, &mut guard).unwrap();
    let (allocated, apparent, errors, _) = values(&stage, 0);
    assert_eq!(allocated as u64, u64::MAX - 80 - 50);
    assert_eq!(apparent as u64, u64::MAX - 80 - 50);
    assert_eq!(errors, u32::MAX - 3 - 3);
}
