use freespace_core::{
    ErrorCode,
    snapshot::v2::{Entry, Reader, RetentionPolicy, Store},
};
use std::{
    fs,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::symlink,
    },
    path::Path,
};

fn normalized(mut entries: Vec<Entry>) -> Vec<Entry> {
    // Both scans visit the same immutable filesystem at different wall times.
    for e in &mut entries {
        e.scanned_at = 0;
        e.min_scanned_at = 0;
    }
    entries
}
fn entries(db: &Path, root: &Path) -> Vec<Entry> {
    normalized(
        Reader::open(db)
            .unwrap()
            .subtree(root.as_os_str().as_bytes(), false)
            .unwrap(),
    )
}
fn no_stages(dir: &Path) {
    assert!(
        !fs::read_dir(dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .as_bytes()
            .ends_with(b".stage.tmp"))
    );
}
#[test]
fn staged_seed_matches_memory_and_remains_locally_refreshable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let other = tmp.path().join("z-other");
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir(&other).unwrap();
    fs::write(root.join("a/file"), [7; 4097]).unwrap();
    // Full-byte-path ownership differs from component-wise DFS visitation.
    fs::hard_link(root.join("a/file"), root.join("a-file")).unwrap();
    fs::hard_link(root.join("a/file"), other.join("alias")).unwrap();
    fs::write(
        root.join(std::ffi::OsString::from_vec(b"raw-\xff".to_vec())),
        [1; 3],
    )
    .unwrap();
    fs::File::create(root.join("sparse"))
        .unwrap()
        .set_len(1_000_000)
        .unwrap();
    symlink("a/file", root.join("symlink")).unwrap();
    let roots = vec![other.clone(), root.clone()];
    let paths = [tmp.path().join("memory.db"), tmp.path().join("disk.db")];
    let mut memory = Store::open(&paths[0]).unwrap();
    let mut disk = Store::open(&paths[1]).unwrap();
    memory
        .scan_roots_with_options(&roots, true, 1, false, true)
        .unwrap();
    disk.scan_roots_with_options(&roots, true, 1, true, true)
        .unwrap();
    for r in &roots {
        assert_eq!(entries(&paths[0], r), entries(&paths[1], r));
    }
    let reader = Reader::open(&paths[1]).unwrap();
    let alias = reader
        .get(root.join("a/file").as_os_str().as_bytes(), false)
        .unwrap()
        .unwrap();
    assert_eq!(alias.allocated, 0);
    assert!(alias.own_allocated > 0);
    let owner = reader.hardlink_owner(&alias.path, false).unwrap().unwrap();
    assert_eq!(owner, root.join("a-file").as_os_str().as_bytes());
    drop(reader);
    fs::write(root.join("a/new"), [2; 80]).unwrap();
    for store in [&mut memory, &mut disk] {
        store.refresh(&root.join("a"), 1).unwrap();
    }
    for r in &roots {
        assert_eq!(entries(&paths[0], r), entries(&paths[1], r));
    }
    no_stages(tmp.path());
}
#[test]
fn both_scan_profiles_honor_keep_previous_and_preserve_pinned_pairs() {
    for low_memory in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        fs::create_dir(&root).unwrap();
        let db = tmp.path().join("v2.db");
        let mut store = Store::open(&db).unwrap();
        let roots = vec![root.clone()];
        store
            .scan_roots_with_options(&roots, true, 1, low_memory, true)
            .unwrap();
        fs::write(root.join("second"), [1]).unwrap();
        store
            .scan_roots_with_options(&roots, true, 1, low_memory, true)
            .unwrap();
        let pinned = Reader::open(&db).unwrap();
        let old_pair = pinned.generations;
        let current = pinned.subtree(root.as_os_str().as_bytes(), false).unwrap();
        let previous = pinned.subtree(root.as_os_str().as_bytes(), true).unwrap();
        fs::write(root.join("third"), [2]).unwrap();
        store
            .scan_roots_with_options(&roots, true, 1, low_memory, false)
            .unwrap();
        let now = Reader::open(&db).unwrap();
        assert_eq!(now.generations.current, 3);
        assert_eq!(now.generations.prev, None);
        assert_eq!(now.meta(false).unwrap().prev_generation, None);
        assert_eq!(pinned.generations, old_pair);
        assert_eq!(
            pinned.subtree(root.as_os_str().as_bytes(), false).unwrap(),
            current
        );
        assert_eq!(
            pinned.subtree(root.as_os_str().as_bytes(), true).unwrap(),
            previous
        );
        assert!(store.maintain(100, true).unwrap().readers_blocking);
        drop(now);
        drop(pinned);
        store
            .scan_roots_with_options(&roots, true, 1, low_memory, true)
            .unwrap();
        assert_eq!(Reader::open(&db).unwrap().generations.prev, Some(3));
        no_stages(tmp.path());
    }
}
#[test]
fn failed_disk_publication_cleans_stage_and_retains_complete_pair() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir(&root).unwrap();
    let db = tmp.path().join("v2.db");
    let mut store = Store::open(&db).unwrap();
    store.scan(&root, 1).unwrap();
    store.scan(&root, 1).unwrap();
    let pinned = Reader::open(&db).unwrap();
    let rows = store.retained_records().unwrap();
    store
        .configure_retention(RetentionPolicy {
            max_records: rows,
            max_wal_bytes: 536870912,
        })
        .unwrap();
    assert_eq!(store.retention_policy().unwrap().max_records, rows);
    fs::write(root.join("new"), [1]).unwrap();
    let error = store
        .scan_roots_with_options(std::slice::from_ref(&root), true, 1, true, false)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Space);
    let after = Reader::open(&db).unwrap();
    assert_eq!(after.generations, pinned.generations);
    for previous in [false, true] {
        assert_eq!(
            after
                .subtree(root.as_os_str().as_bytes(), previous)
                .unwrap(),
            pinned
                .subtree(root.as_os_str().as_bytes(), previous)
                .unwrap()
        );
    }
    assert_eq!(store.retained_records().unwrap(), rows);
    no_stages(tmp.path());
}
#[test]
fn root_validation_matches_for_both_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("nested")).unwrap();
    for low_memory in [false, true] {
        let mut store = Store::open(&tmp.path().join(format!("{low_memory}.db"))).unwrap();
        assert_eq!(
            store
                .scan_roots_with_options(
                    &[root.clone(), root.join("nested")],
                    true,
                    1,
                    low_memory,
                    true
                )
                .unwrap_err()
                .code,
            ErrorCode::Usage
        );
        no_stages(tmp.path());
    }
}

#[test]
fn denied_report_streams_every_nested_path_from_its_pin() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let mut expected = Vec::new();
    for n in 0..125 {
        let parent = root.join(format!("dir-{n:03}"));
        fs::create_dir_all(parent.join("nested")).unwrap();
        expected.push(parent.to_string_lossy().into_owned());
        expected.push(parent.join("nested").to_string_lossy().into_owned());
    }
    let db = tmp.path().join("v2.db");
    let mut store = Store::open(&db).unwrap();
    store
        .scan_roots_with_options(std::slice::from_ref(&root), true, 1, true, true)
        .unwrap();
    // Inject unreadable directory metadata without depending on process UID.
    let conn = rusqlite::Connection::open(&db).unwrap();
    for table in ["live_rows", "version_rows"] {
        conn.execute(
            &format!("UPDATE {table} SET flags=flags|1 WHERE parent IS NOT NULL"),
            [],
        )
        .unwrap();
    }
    let pinned = Reader::open(&db).unwrap();
    store
        .scan_roots_with_options(&[root], true, 1, true, true)
        .unwrap();
    let report = pinned.denied_report(tmp.path()).unwrap();
    assert_eq!(report.len(), 250);
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::json!(expected)
    );
    let mut visited = Vec::new();
    pinned
        .visit_denied(|path| {
            visited.push(String::from_utf8_lossy(path).into_owned());
            Ok(())
        })
        .unwrap();
    assert_eq!(visited, expected);
    assert!(
        Reader::open(&db)
            .unwrap()
            .denied_report(tmp.path())
            .unwrap()
            .is_empty()
    );
    assert!(!tmp.path().join("snapshot.denied.tmp").exists());
}
