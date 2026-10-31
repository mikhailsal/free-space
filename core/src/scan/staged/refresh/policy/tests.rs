use super::*;

#[test]
fn identical_paths_choose_first_normally_and_last_when_forced() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("stage.db");
    let conn = open_database(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE links(id INTEGER PRIMARY KEY,dev,ino,path BLOB,root);
         CREATE INDEX links_owner ON links(dev,ino,path,root,id);
         CREATE TABLE live(id INTEGER PRIMARY KEY,ordinal INTEGER UNIQUE);
         INSERT INTO links VALUES (1,7,9,X'2F782F66',0),(2,7,9,X'2F782F66',1);
         INSERT INTO live VALUES (1,3),(2,12);",
    )
    .unwrap();
    let mut reserve = Reserve::new(tmp.path());
    reserve.min_free = 0;
    let mut guard = SpaceGuard::new(&reserve).unwrap();
    let stage = StagedTree {
        conn,
        path,
        roots: Tree::default(),
        count: 2,
        one_file_system: false,
        reserve,
        cleanup: true,
        refresh_mode: true,
    };
    assert_eq!(
        choose_live(&stage, 7, 9, None, &mut guard).unwrap(),
        Some(1)
    );
    assert_eq!(
        choose_live(&stage, 7, 9, Some(b"/x/f"), &mut guard).unwrap(),
        Some(2)
    );
}
