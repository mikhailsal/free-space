//! Differential contract tests for the opt-in disk-backed full scan.

use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::symlink;
use std::path::Path;

use freespace_core::job::{ScanRequest, run_scan};
use freespace_core::queries::{self, params::FindParams};
use freespace_core::refresh::{RefreshRequest, run_refresh};
use freespace_core::scan::{WalkOptions, staged::StagedTree};
use freespace_core::snapshot::{SnapshotDir, reader::Snapshot, writer::Reserve};
use rusqlite::types::Value;

fn scan(roots: &[&Path], dir: &SnapshotDir, low_memory: bool) -> freespace_core::job::ScanReport {
    run_scan(ScanRequest {
        roots: roots.iter().map(|p| p.to_path_buf()).collect(),
        one_file_system: true,
        workers: 4,
        low_memory,
        dir: dir.clone(),
        keep_previous: true,
        on_progress: None,
    })
    .unwrap()
}

fn rows(s: &Snapshot, sql: &str) -> Vec<Vec<Value>> {
    let mut stmt = s.conn.prepare(sql).unwrap();
    let count = stmt.column_count();
    stmt.query_map([], |row| {
        (0..count)
            .map(|i| row.get(i))
            .collect::<rusqlite::Result<Vec<Value>>>()
    })
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}

fn assert_equivalent(a: &Snapshot, b: &Snapshot) {
    // Scan times and live filesystem free-space measurements necessarily differ.
    for sql in [
        "SELECT id,parent_id,name,path_hash,kind,flags,allocated,apparent,mtime,nlink,items,errors,subtree_end,newest_mtime FROM entries ORDER BY id",
        "SELECT * FROM hardlinks ORDER BY entry_id",
        "SELECT entry_id,path,fs_type FROM roots ORDER BY entry_id",
        "SELECT entry_id,path,fs_type FROM mounts ORDER BY entry_id",
        "SELECT * FROM denied ORDER BY entry_id",
        "SELECT * FROM refreshes ORDER BY path",
    ] {
        assert_eq!(rows(a, sql), rows(b, sql), "{sql}");
    }
    assert_eq!(a.meta.entries, b.meta.entries);
    assert_eq!(
        b.conn
            .query_row::<String, _, _>("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        b.conn
            .query_row::<i64, _, _>(
                "SELECT count(*) FROM entries e WHERE
        e.min_scanned_at > e.scanned_at OR e.subtree_end != e.id + e.items OR
        (e.parent_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM entries p WHERE p.id=e.parent_id
        AND e.id>p.id AND e.subtree_end<=p.subtree_end))",
                [],
                |r| r.get(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn disk_scan_preserves_raw_names_preorder_and_full_path_hardlink_ownership() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tree");
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir_all(root.join("empty")).unwrap();
    fs::write(root.join("a/file"), vec![7; 17000]).unwrap();
    fs::File::open(root.join("a/file"))
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
        .unwrap();
    // '/' sorts after '-', although DFS visits a/file before a-link.
    fs::hard_link(root.join("a/file"), root.join("a-link")).unwrap();
    fs::write(
        root.join(std::ffi::OsString::from_vec(b"bad-\xff\n".to_vec())),
        b"bytes",
    )
    .unwrap();
    symlink("a/file", root.join("symlink")).unwrap();
    fs::hard_link(root.join("symlink"), root.join("symlink-alias")).unwrap();
    let fast = SnapshotDir::new(tmp.path().join("fast"));
    let disk = SnapshotDir::new(tmp.path().join("disk"));
    let old = scan(&[&root], &fast, false);
    let new = scan(&[&root], &disk, true);
    assert_eq!(old.entries, new.entries);
    assert_eq!(old.errors, new.errors);
    assert!(!disk.path.join("snapshot.stage.tmp").exists());
    let old = Snapshot::open_current(&fast).unwrap();
    let new = Snapshot::open_current(&disk).unwrap();
    assert_equivalent(&old, &new);
    // Leaves are complete on insertion, including file time clamping and
    // non-file sentinel conversion, even for zero-sized hardlink aliases.
    let invalid_leaves: i64 = new
        .conn
        .query_row(
            "SELECT count(*) FROM entries WHERE kind != 1 AND (items != 0 OR subtree_end != id
         OR min_scanned_at != scanned_at OR newest_mtime !=
         CASE WHEN kind = 0 THEN min(mtime, scanned_at) ELSE 0 END)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(invalid_leaves, 0);
    let aliases: i64 = new.conn.query_row(
        "SELECT count(*) FROM entries WHERE (flags & 2) != 0 AND allocated = 0 AND apparent = 0",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(aliases, 2);
    let empty: (i64, i64) = new
        .conn
        .query_row(
            "SELECT items, subtree_end - id FROM entries WHERE name = ?1 AND kind = 1",
            [b"empty".as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(empty, (0, 0));
}

#[test]
fn disk_scan_keeps_root_order_and_attributes_links_across_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    let z = tmp.path().join("z");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&z).unwrap();
    fs::write(z.join("payload"), b"shared").unwrap();
    fs::hard_link(z.join("payload"), a.join("payload")).unwrap();
    let fast = SnapshotDir::new(tmp.path().join("fast"));
    let disk = SnapshotDir::new(tmp.path().join("disk"));
    scan(&[&z, &a, &z], &fast, false);
    scan(&[&z, &a, &z], &disk, true);
    assert_equivalent(
        &Snapshot::open_current(&fast).unwrap(),
        &Snapshot::open_current(&disk).unwrap(),
    );
}

#[test]
fn disk_scan_handles_wide_directories_and_pending_directory_jobs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tree");
    fs::create_dir(&root).unwrap();
    for index in 0..3000 {
        fs::write(root.join(format!("file-{index:04}")), b"").unwrap();
    }
    for index in 0..200 {
        fs::create_dir(root.join(format!("dir-{index:04}"))).unwrap();
    }
    let fast = SnapshotDir::new(tmp.path().join("fast"));
    let disk = SnapshotDir::new(tmp.path().join("disk"));
    scan(&[&root], &fast, false);
    assert_eq!(scan(&[&root], &disk, true).entries, 3201);
    assert_equivalent(
        &Snapshot::open_current(&fast).unwrap(),
        &Snapshot::open_current(&disk).unwrap(),
    );
}

#[test]
fn disk_snapshots_support_queries_refresh_and_generation_rotation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tree");
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("sub/file"), b"before").unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snap"));
    assert_eq!(scan(&[&root], &dir, true).generation, 1);
    fs::write(root.join("sub/file"), b"after and larger").unwrap();
    assert_eq!(scan(&[&root], &dir, true).generation, 2);
    assert_eq!(Snapshot::open(&dir.prev()).unwrap().meta.generation, 1);
    fs::write(root.join("sub/new"), b"new").unwrap();
    let refreshed = run_refresh(RefreshRequest {
        path: root.join("sub").as_os_str().as_bytes().to_vec(),
        workers: 1,
        dir: dir.clone(),
        on_progress: None,
    })
    .unwrap();
    assert_eq!(refreshed.generation, 3);
    assert_eq!(Snapshot::open(&dir.prev()).unwrap().meta.generation, 1);
    let snapshot = Snapshot::open_current(&dir).unwrap();
    let found = queries::find(
        &snapshot,
        &FindParams {
            path: Some(root.to_str().unwrap().into()),
            kind: Some("file".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(found.entries.len(), 2);
    let next = scan(&[&root], &dir, true);
    assert_eq!(next.generation, 4);
    assert_eq!(next.prev_generation, Some(3));
}

#[test]
fn staging_reserve_failure_and_bad_roots_leave_current_snapshot_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tree");
    fs::create_dir(&root).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snap"));
    scan(&[&root], &dir, true);
    let before = fs::read(dir.current()).unwrap();
    let stage = dir.path.join("snapshot.stage.tmp");
    let mut reserve = Reserve::new(&dir.path);
    reserve.min_free = u64::MAX;
    assert!(StagedTree::walk(&WalkOptions::new(vec![root.clone()]), &stage, &reserve).is_err());
    assert!(!stage.exists());
    let error = run_scan(ScanRequest {
        roots: vec![root.join("absent")],
        one_file_system: true,
        workers: 1,
        low_memory: true,
        dir: dir.clone(),
        keep_previous: true,
        on_progress: None,
    })
    .unwrap_err();
    assert_eq!(error.code, freespace_core::ErrorCode::Usage);
    assert_eq!(fs::read(dir.current()).unwrap(), before);
    assert!(!stage.exists());
}

#[test]
fn disk_scan_reports_every_denied_directory_without_truncation() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tree");
    fs::create_dir(&root).unwrap();
    for id in 0..30 {
        let dir = root.join(format!("denied-{id:02}"));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(dir, fs::Permissions::from_mode(0o0)).unwrap();
    }
    let fast = SnapshotDir::new(tmp.path().join("fast"));
    let disk = SnapshotDir::new(tmp.path().join("disk"));
    let a = scan(&[&root], &fast, false);
    let b = scan(&[&root], &disk, true);
    assert_eq!(a.errors, 30);
    assert_eq!(b.errors, 30);
    assert_eq!(
        serde_json::to_value(&a.denied).unwrap(),
        serde_json::to_value(&b.denied).unwrap()
    );
    assert_eq!(a.denied.warning(), b.denied.warning());
    assert_equivalent(
        &Snapshot::open_current(&fast).unwrap(),
        &Snapshot::open_current(&disk).unwrap(),
    );
    for id in 0..30 {
        fs::set_permissions(
            root.join(format!("denied-{id:02}")),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
}

/// Equal full paths occur when a cross-mount scan includes a nested root on a
/// different device. Tree assigns the inode to the first root's DFS occurrence.
#[test]
fn disk_hardlink_path_ties_follow_root_order_across_overlapping_mounts() {
    use std::os::unix::fs::MetadataExt;
    let fixture = tempfile::tempdir_in("/dev/shm").unwrap();
    let device = Path::new("/dev");
    if fs::metadata(device).unwrap().dev() == fs::metadata(fixture.path()).unwrap().dev() {
        return; // This host does not have the cross-device overlap used by the fixture.
    }
    fs::write(fixture.path().join("a"), vec![1; 8192]).unwrap();
    fs::hard_link(fixture.path().join("a"), fixture.path().join("b")).unwrap();
    let inode = fs::metadata(fixture.path().join("a")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    for (order, roots) in [[device, fixture.path()], [fixture.path(), device]]
        .iter()
        .enumerate()
    {
        let mut results = Vec::new();
        for low_memory in [false, true] {
            let dir = SnapshotDir::new(temp.path().join(format!("{order}-{low_memory}")));
            run_scan(ScanRequest {
                roots: roots.iter().map(|p| p.to_path_buf()).collect(),
                one_file_system: false,
                workers: 1,
                low_memory,
                dir: dir.clone(),
                keep_previous: false,
                on_progress: None,
            })
            .unwrap();
            let snapshot = Snapshot::open_current(&dir).unwrap();
            let sql = format!(
                "SELECT r.path,e.name,e.allocated,e.flags,h.allocated FROM hardlinks h
                JOIN entries e ON e.id=h.entry_id JOIN roots r JOIN entries rt ON rt.id=r.entry_id
                WHERE h.dev={} AND h.ino={} AND e.id BETWEEN rt.id AND rt.subtree_end
                ORDER BY r.entry_id,e.name",
                inode.dev() as i64,
                inode.ino() as i64
            );
            results.push(rows(&snapshot, &sql));
        }
        assert_eq!(results[0].len(), 4);
        assert_eq!(results[0], results[1], "original root order {order}");
        assert_eq!(
            results[1][0][2],
            Value::Integer((inode.blocks() * 512) as i64)
        );
        assert!(
            results[1]
                .iter()
                .skip(1)
                .all(|row| row[2] == Value::Integer(0))
        );
    }
}
