use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rusqlite::{Connection, types::Value};

use super::*;
use crate::scan::WalkOptions;

fn scan_at(path: &Path) -> Tree {
    let mut options = WalkOptions::new(vec![path.to_path_buf()]);
    options.workers = 1;
    let mut tree = scan::walk(&options).unwrap();
    for node in &mut tree.nodes {
        node.scanned_at = 1_700_000_000;
    }
    tree.started_at = 1_700_000_000;
    tree
}

fn snapshot(root: &Path, dir: &SnapshotDir) -> Snapshot {
    fs::create_dir_all(dir.as_path()).unwrap();
    let mut tree = scan_at(root);
    let fin = tree.finalize();
    let meta = WriteMeta::new(1, None, tree.started_at);
    let mut reserve = Reserve::new(dir.as_path());
    reserve.min_free = 0;
    writer::write_snapshot(&dir.current(), tree, fin, &meta, &reserve).unwrap();
    Snapshot::open_current(dir).unwrap()
}

fn rows(conn: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut st = conn.prepare(sql).unwrap();
    let count = st.column_count();
    st.query_map([], |row| (0..count).map(|i| row.get(i)).collect())
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

fn mark_snapshot(dir: &SnapshotDir, marker: &str) {
    if marker != "none" {
        let conn = Connection::open(dir.current()).unwrap();
        if marker == "denied" {
            conn.execute(
                "UPDATE entries SET flags=flags | 1 WHERE name=X'6d61726b6572'",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO denied SELECT id FROM entries WHERE name=X'6d61726b6572'",
                [],
            )
            .unwrap();
        } else {
            if marker != "file_mount_row" {
                conn.execute("UPDATE entries SET kind=4 WHERE name=X'6d61726b6572'", [])
                    .unwrap();
            }
            if marker == "mount_row" || marker == "file_mount_row" {
                conn.execute("INSERT INTO mounts SELECT id,X'2f6d6f756e74','test',NULL,NULL FROM entries WHERE name=X'6d61726b6572'", []).unwrap();
            }
        }
    }
}

fn compare_case(remove: bool, outside: &str, replacement: &str, external: bool, marker: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("old"), b"payload").unwrap();
    let other = if external {
        tmp.path().join(outside)
    } else {
        root.join(outside)
    };
    fs::hard_link(target.join("old"), &other).unwrap();
    if marker != "none" {
        fs::write(root.join("marker"), b"m").unwrap();
    }
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    drop(snapshot(&root, &dir));
    mark_snapshot(&dir, marker);
    let snap = Snapshot::open_current(&dir).unwrap();
    let (x, _) = resolve(&snap, target.as_os_str().as_bytes()).unwrap();
    fs::remove_file(target.join("old")).unwrap();
    if remove {
        fs::remove_dir(&target).unwrap();
    } else {
        fs::hard_link(&other, target.join(replacement)).unwrap();
        fs::write(target.join("new"), b"new").unwrap();
    }
    let mut loaded = load_tree(&snap).unwrap();
    let replacement = if remove { None } else { Some(scan_at(&target)) };
    let mut paths = Paths::new(&snap.conn);
    let policy = graft(&mut loaded, x.id, x.subtree_end, replacement, &mut paths).unwrap();
    let fin = loaded.tree.finalize_with(&policy);
    let mut meta = WriteMeta::new(3, None, snap.meta.scanned_at);
    meta.refreshes = vec![
        RefreshRow {
            path: b"/prior".to_vec(),
            scanned_at: 1,
            generation: 1,
        },
        RefreshRow {
            path: target.as_os_str().as_bytes().to_vec(),
            scanned_at: 2,
            generation: 2,
        },
        RefreshRow {
            path: target.as_os_str().as_bytes().to_vec(),
            scanned_at: 3,
            generation: 3,
        },
    ];
    let mut reserve = Reserve::new(dir.as_path());
    reserve.min_free = 0;
    let oracle = tmp.path().join("oracle.db");
    writer::write_snapshot(&oracle, loaded.tree, fin, &meta, &reserve).unwrap();
    let roots = load::roots(&snap).unwrap();
    let replacement = if remove { None } else { Some(scan_at(&target)) };
    let stage = StagedTree::refresh(
        &snap,
        roots,
        x.id,
        x.subtree_end,
        replacement,
        &tmp.path().join("stage.tmp"),
        &reserve,
    )
    .unwrap();
    let output = tmp.path().join("staged.db");
    let (_, denied, _) = stage.write(&output, &meta).unwrap();
    assert_eq!(denied.len(), usize::from(marker == "denied"));
    let a = Connection::open(oracle).unwrap();
    let b = Connection::open(output).unwrap();
    for table in ["entries", "hardlinks", "roots", "mounts", "denied"] {
        let sql = format!("SELECT * FROM {table} ORDER BY 1");
        assert_eq!(rows(&a, &sql), rows(&b, &sql), "table {table}");
    }
    let history = "SELECT path,scanned_at,generation FROM refreshes ORDER BY 1,2,3";
    assert_eq!(rows(&a, history), rows(&b, history), "table refreshes");
}

#[test]
fn staged_refresh_matches_in_memory_hardlink_replacement() {
    compare_case(false, "outside", "replacement", false, "none");
}

#[test]
fn staged_refresh_matches_in_memory_removal() {
    compare_case(true, "outside", "replacement", false, "none");
}

#[test]
fn staged_refresh_matches_removed_owner_and_reappearing_owner_path() {
    compare_case(false, "z_outside", "replacement", false, "none");
    compare_case(false, "z_outside", "old", false, "none");
}

#[test]
fn in_memory_refresh_oracle_remains_callable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    drop(snapshot(&root, &dir));
    let bytes = target.as_os_str().as_bytes().to_vec();
    let req = RefreshRequest {
        path: bytes.clone(),
        workers: 1,
        dir,
        on_progress: None,
    };
    let report = refresh_locked(req, bytes, "target").unwrap();
    assert_eq!(report.subtree_entries, 1);
}

#[test]
fn staged_refresh_matches_unrecorded_external_hardlink_zeroing() {
    compare_case(false, "external", "replacement", true, "none");
}

#[test]
fn occupied_stage_path_is_not_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    let snap = snapshot(&root, &dir);
    let (x, _) = resolve(&snap, target.as_os_str().as_bytes()).unwrap();
    let occupied = tmp.path().join("stage.tmp");
    fs::write(&occupied, b"foreign").unwrap();
    let mut reserve = Reserve::new(dir.as_path());
    reserve.min_free = 0;
    let result = StagedTree::refresh(
        &snap,
        load::roots(&snap).unwrap(),
        x.id,
        x.subtree_end,
        Some(scan_at(&target)),
        &occupied,
        &reserve,
    );
    assert!(result.is_err());
    assert_eq!(fs::read(&occupied).unwrap(), b"foreign");
    assert!(dir.current().exists());
}

#[test]
fn public_refresh_publishes_history_and_keeps_previous_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    drop(snapshot(&root, &dir));
    fs::copy(dir.current(), dir.prev()).unwrap();
    let previous = fs::read(dir.prev()).unwrap();
    let oracle_dir = SnapshotDir::new(tmp.path().join("oracle"));
    fs::create_dir_all(oracle_dir.as_path()).unwrap();
    fs::copy(dir.current(), oracle_dir.current()).unwrap();
    fs::copy(dir.prev(), oracle_dir.prev()).unwrap();
    fs::write(target.join("new"), b"new").unwrap();
    let bytes = target.as_os_str().as_bytes().to_vec();
    let result = run_refresh(RefreshRequest {
        path: bytes.clone(),
        workers: 1,
        dir: dir.clone(),
        on_progress: None,
    })
    .unwrap();
    assert_eq!(result.generation, 2);
    assert_eq!(result.subtree_entries, 2);
    let oracle = refresh_locked(
        RefreshRequest {
            path: bytes.clone(),
            workers: 1,
            dir: oracle_dir.clone(),
            on_progress: None,
        },
        bytes.clone(),
        "target",
    )
    .unwrap();
    assert_eq!(result.entries, oracle.entries);
    assert_eq!(result.denied, oracle.denied);
    let current = Snapshot::open_current(&dir).unwrap();
    let expected = Snapshot::open_current(&oracle_dir).unwrap();
    let stable = "SELECT id,parent_id,name,kind,flags,allocated,apparent,mtime,nlink,items,errors,subtree_end FROM entries ORDER BY id";
    assert_eq!(rows(&current.conn, stable), rows(&expected.conn, stable));
    assert_eq!(fs::read(oracle_dir.prev()).unwrap(), previous);
    assert_eq!(current.meta.generation, 2);
    let (history_path, history_generation): (Vec<u8>, i64) = current
        .conn
        .query_row("SELECT path,generation FROM refreshes", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(history_path, bytes);
    assert_eq!(history_generation, 2);
    assert_eq!(fs::read(dir.prev()).unwrap(), previous);
    assert!(!dir.as_path().join("snapshot.stage.tmp").exists());
    assert!(!dir.tmp(2).exists());
}

#[test]
fn public_refresh_retains_multiple_history_rows_and_repeated_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let first = root.join("first");
    let second = root.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    drop(snapshot(&root, &dir));
    let old = root.join("older");
    let conn = Connection::open(dir.current()).unwrap();
    conn.execute(
        "INSERT INTO refreshes VALUES (?1,?2,?3)",
        rusqlite::params![old.as_os_str().as_bytes(), 1_600_000_000_i64, 1_i64],
    )
    .unwrap();
    drop(conn);
    fs::copy(dir.current(), dir.prev()).unwrap();
    let previous = fs::read(dir.prev()).unwrap();
    let before = platform::now_secs();
    for (target, generation) in [(&first, 2), (&second, 3), (&first, 4)] {
        let result = run_refresh(RefreshRequest {
            path: target.as_os_str().as_bytes().to_vec(),
            workers: 1,
            dir: dir.clone(),
            on_progress: None,
        })
        .unwrap();
        assert_eq!(result.generation, generation);
    }
    let after = platform::now_secs();
    let current = Snapshot::open_current(&dir).unwrap();
    assert_eq!(current.meta.generation, 4);
    let history = rows(
        &current.conn,
        "SELECT path,scanned_at,generation FROM refreshes ORDER BY generation",
    );
    assert_eq!(history.len(), 4);
    for (row, (path, generation)) in
        history
            .iter()
            .zip([(&old, 1), (&first, 2), (&second, 3), (&first, 4)])
    {
        assert_eq!(row[0], Value::Blob(path.as_os_str().as_bytes().to_vec()));
        assert_eq!(row[2], Value::Integer(generation));
        let Value::Integer(timestamp) = &row[1] else {
            panic!("invalid timestamp")
        };
        if generation == 1 {
            assert_eq!(*timestamp, 1_600_000_000);
        } else {
            assert!((before..=after).contains(timestamp));
        }
    }
    let status = crate::queries::status(&dir).unwrap();
    assert_eq!(status.generation, Some(4));
    assert_eq!(status.refreshed.len(), 4);
    let repeated = status.refreshed.iter().find(|r| r.generation == 4).unwrap();
    assert_eq!(repeated.path, first.to_string_lossy());
    assert!(status.last_error.is_none());
    let state = state::read(&dir).unwrap().unwrap();
    assert_eq!(state.current_generation, Some(4));
    assert!(state.last_error.is_none());
    assert_eq!(fs::read(dir.prev()).unwrap(), previous);
}

#[test]
fn stage_reserve_failure_keeps_current_and_previous() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    let snap = snapshot(&root, &dir);
    fs::copy(dir.current(), dir.prev()).unwrap();
    let current = fs::read(dir.current()).unwrap();
    let previous = fs::read(dir.prev()).unwrap();
    let (x, _) = resolve(&snap, target.as_os_str().as_bytes()).unwrap();
    let mut reserve = Reserve::new(dir.as_path());
    reserve.min_free = u64::MAX;
    let path = tmp.path().join("stage.tmp");
    assert!(
        StagedTree::refresh(
            &snap,
            load::roots(&snap).unwrap(),
            x.id,
            x.subtree_end,
            Some(scan_at(&target)),
            &path,
            &reserve
        )
        .is_err()
    );
    assert!(!path.exists());
    assert_eq!(fs::read(dir.current()).unwrap(), current);
    assert_eq!(fs::read(dir.prev()).unwrap(), previous);
}

#[test]
fn staged_refresh_preserves_denied_and_optional_mount_rows() {
    compare_case(false, "outside", "replacement", false, "denied");
    compare_case(false, "outside", "replacement", false, "mount_row");
    compare_case(false, "outside", "replacement", false, "mount_missing");
    compare_case(false, "outside", "replacement", false, "file_mount_row");
}

#[test]
fn staged_refresh_accepts_root_target_and_preserves_generation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    drop(snapshot(&root, &dir));
    fs::write(root.join("new"), b"new").unwrap();
    let report = run_refresh(RefreshRequest {
        path: root.as_os_str().as_bytes().to_vec(),
        workers: 1,
        dir: dir.clone(),
        on_progress: None,
    })
    .unwrap();
    assert_eq!(report.subtree_entries, 2);
    assert_eq!(Snapshot::open_current(&dir).unwrap().meta.entries, 2);
}

#[test]
fn staged_refresh_preserves_other_root() {
    let tmp = tempfile::tempdir().unwrap();
    let first = tmp.path().join("first");
    let second = tmp.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(second.join("other"), b"keep").unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    fs::create_dir_all(dir.as_path()).unwrap();
    let mut options = WalkOptions::new(vec![first.clone(), second.clone()]);
    options.workers = 1;
    let mut tree = scan::walk(&options).unwrap();
    let meta = WriteMeta::new(1, None, tree.started_at);
    let fin = tree.finalize();
    let mut reserve = Reserve::new(dir.as_path());
    reserve.min_free = 0;
    writer::write_snapshot(&dir.current(), tree, fin, &meta, &reserve).unwrap();
    fs::write(first.join("new"), b"new").unwrap();
    let report = run_refresh(RefreshRequest {
        path: first.as_os_str().as_bytes().to_vec(),
        workers: 1,
        dir: dir.clone(),
        on_progress: None,
    })
    .unwrap();
    assert_eq!(report.entries, 4);
    let snap = Snapshot::open_current(&dir).unwrap();
    assert_eq!(
        snap.conn
            .query_row("SELECT count(*) FROM roots", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(resolve(&snap, second.join("other").as_os_str().as_bytes()).is_ok());
}

fn missing_root_is_rejected(root_count: usize) {
    let tmp = tempfile::tempdir().unwrap();
    let first = tmp.path().join("first");
    let second = tmp.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let mut roots = vec![first.clone()];
    if root_count == 2 {
        roots.push(second);
    }
    let mut options = WalkOptions::new(roots);
    options.workers = 1;
    let mut tree = scan::walk(&options).unwrap();
    let meta = WriteMeta::new(1, None, tree.started_at);
    let fin = tree.finalize();
    let dir = SnapshotDir::new(tmp.path().join("candidate"));
    let oracle_dir = SnapshotDir::new(tmp.path().join("oracle"));
    fs::create_dir_all(dir.as_path()).unwrap();
    fs::create_dir_all(oracle_dir.as_path()).unwrap();
    let mut reserve = Reserve::new(dir.as_path());
    reserve.min_free = 0;
    writer::write_snapshot(&dir.current(), tree, fin, &meta, &reserve).unwrap();
    fs::copy(dir.current(), dir.prev()).unwrap();
    fs::copy(dir.current(), oracle_dir.current()).unwrap();
    fs::copy(dir.prev(), oracle_dir.prev()).unwrap();
    let current = fs::read(dir.current()).unwrap();
    let previous = fs::read(dir.prev()).unwrap();
    fs::remove_dir(&first).unwrap();
    let bytes = first.as_os_str().as_bytes().to_vec();
    let candidate = run_refresh(RefreshRequest {
        path: bytes.clone(),
        workers: 1,
        dir: dir.clone(),
        on_progress: None,
    })
    .unwrap_err();
    let label = String::from_utf8_lossy(&bytes).into_owned();
    let oracle = refresh_locked(
        RefreshRequest {
            path: bytes.clone(),
            workers: 1,
            dir: oracle_dir.clone(),
            on_progress: None,
        },
        bytes,
        &label,
    )
    .unwrap_err();
    assert_eq!(candidate.code, ErrorCode::Usage);
    assert_eq!(candidate.code, oracle.code);
    assert_eq!(candidate.message, oracle.message);
    for d in [&dir, &oracle_dir] {
        assert_eq!(fs::read(d.current()).unwrap(), current);
        assert_eq!(fs::read(d.prev()).unwrap(), previous);
        assert!(!d.as_path().join("snapshot.stage.tmp").exists());
    }
}

#[test]
fn missing_only_root_is_rejected_without_publishing() {
    missing_root_is_rejected(1);
}

#[test]
fn missing_first_of_two_roots_is_rejected_without_publishing() {
    missing_root_is_rejected(2);
}

#[test]
fn internal_missing_root_rejected_before_stage_creation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    let snap = snapshot(&root, &dir);
    fs::copy(dir.current(), dir.prev()).unwrap();
    let before = fs::read(dir.current()).unwrap();
    let previous = fs::read(dir.prev()).unwrap();
    let (row, _) = resolve(&snap, root.as_os_str().as_bytes()).unwrap();
    let path = tmp.path().join("stage.tmp");
    let mut reserve = Reserve::new(dir.as_path());
    reserve.min_free = 0;
    let error = StagedTree::refresh(
        &snap,
        load::roots(&snap).unwrap(),
        row.id,
        row.subtree_end,
        None,
        &path,
        &reserve,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, ErrorCode::Usage);
    assert!(!path.exists());
    assert_eq!(fs::read(dir.current()).unwrap(), before);
    assert_eq!(fs::read(dir.prev()).unwrap(), previous);
}

#[test]
fn occupied_output_path_is_not_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let target = root.join("target");
    fs::create_dir_all(&target).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snapshot"));
    let snap = snapshot(&root, &dir);
    let (row, _) = resolve(&snap, target.as_os_str().as_bytes()).unwrap();
    let mut reserve = Reserve::new(dir.as_path());
    reserve.min_free = 0;
    let stage_path = tmp.path().join("stage.tmp");
    let stage = StagedTree::refresh(
        &snap,
        load::roots(&snap).unwrap(),
        row.id,
        row.subtree_end,
        Some(scan_at(&target)),
        &stage_path,
        &reserve,
    )
    .unwrap();
    let output = tmp.path().join("occupied.db");
    fs::write(&output, b"foreign output").unwrap();
    let meta = WriteMeta::new(2, None, snap.meta.scanned_at);
    assert!(stage.write(&output, &meta).is_err());
    assert_eq!(fs::read(output).unwrap(), b"foreign output");
    assert!(!stage_path.exists());
    assert!(dir.current().exists());
}
