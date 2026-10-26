//! Edges of refresh, diff and scan, pinned before core is split (D0 of
//! docs/designs/code-quality-gates.md): a folder deleted since the scan, a path
//! that now resolves elsewhere, a root gone, a file, a path in neither
//! snapshot, two rows of one hash, a scan of no roots. Each refusal by its
//! code, message and fix, and whether it reaches state.json.

use std::fs;

use freespace_core::queries::{self, diff, params::*};
use freespace_core::snapshot::reader::Snapshot;
use freespace_core::snapshot::state;
use freespace_core::{Error, ErrorCode};

mod common;
use common::{Fx, fx};

impl Fx {
    fn names(&self, rel: &str) -> Vec<String> {
        let snap = Snapshot::open_current(&self.dir).unwrap();
        let r = queries::ls(
            &snap,
            &LsParams {
                path: Some(self.s(rel)),
                ..Default::default()
            },
        )
        .unwrap();
        r.entries.into_iter().map(|e| e.name).collect()
    }

    fn diff(&self, rel: &str) -> Result<queries::types::DiffResponse, Error> {
        diff::diff(
            &self.dir,
            &DiffParams {
                path: Some(self.s(rel)),
                ..Default::default()
            },
        )
    }

    fn last_error(&self) -> Option<state::LastError> {
        state::read(&self.dir).unwrap().unwrap().last_error
    }
}

#[test]
fn refresh_of_a_deleted_folder_drops_it() {
    let f = fx();
    f.write_file("a/b/c", 8192);
    f.write_file("a/d", 4096);
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
    fs::remove_dir_all(f.p("a/b")).unwrap();
    let rep = f.try_refresh("a/b").unwrap();
    assert!(rep.removed);
    assert_eq!(rep.subtree_entries, 0);
    assert_eq!(rep.generation, 2);
    assert_eq!(f.names("a"), ["d"]);
    let e = queries::ls(
        &Snapshot::open_current(&f.dir).unwrap(),
        &LsParams {
            path: Some(f.s("a/b")),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::PathNotFound, "{}", e.message);
}

/// A permission guard restores access even when an assertion panics.
struct RestorePermissions(std::path::PathBuf, fs::Permissions);

impl Drop for RestorePermissions {
    fn drop(&mut self) {
        fs::set_permissions(&self.0, self.1.clone()).unwrap();
    }
}

#[test]
fn refresh_stat_errors_do_not_publish_deletion() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    for permission_denied in [true, false] {
        if permission_denied && unsafe { libc::geteuid() } == 0 {
            continue; // root can stat through the inaccessible ancestor
        }
        let f = fx();
        f.write_file("a/b/c", 8192);
        f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
        f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
        let snapshot =
            |path: &std::path::Path| (fs::read(path).unwrap(), fs::metadata(path).unwrap().ino());
        let current = snapshot(&f.dir.current());
        let previous = snapshot(&f.dir.prev());
        let before = state::read(&f.dir).unwrap().unwrap();
        let a = f.p("a");
        let restore = if permission_denied {
            let permissions = fs::metadata(&a).unwrap().permissions();
            let restore = RestorePermissions(a.clone(), permissions);
            fs::set_permissions(&a, fs::Permissions::from_mode(0o000)).unwrap();
            Some(restore)
        } else {
            fs::rename(&a, f.p("moved")).unwrap();
            fs::write(&a, b"now a file").unwrap();
            None
        };
        let stat_error = fs::symlink_metadata(f.p("a/b")).unwrap_err();
        assert_eq!(
            stat_error.raw_os_error(),
            Some(if permission_denied {
                libc::EACCES
            } else {
                libc::ENOTDIR
            })
        );
        let result = f.try_refresh("a/b");
        drop(restore);
        let e = result.unwrap_err();
        assert_eq!(e.code, ErrorCode::Generic, "{}", e.message);
        assert!(e.message.contains(&f.s("a/b")), "{}", e.message);
        assert!(e.message.contains(&stat_error.to_string()), "{}", e.message);
        assert_eq!(snapshot(&f.dir.current()), current);
        assert_eq!(snapshot(&f.dir.prev()), previous);
        assert_eq!(f.names("a/b"), ["c"]);
        let mut after = state::read(&f.dir).unwrap().unwrap();
        let failure = after.last_error.take().unwrap();
        assert_eq!(failure.op, "refresh");
        assert_eq!(failure.message, e.message);
        assert_eq!(after, before);
        assert!(!f.dir.progress().exists());

        // Recovery also verifies the failed refresh released its lock.
        if !permission_denied {
            fs::remove_file(&a).unwrap();
            fs::rename(f.p("moved"), &a).unwrap();
        }
        assert_eq!(fs::metadata(f.p("a/b/c")).unwrap().len(), 8192);
        assert_eq!(f.try_refresh("a/b").unwrap().generation, 3);
        assert_eq!(f.last_error(), None);
        assert_eq!(snapshot(&f.dir.prev()), previous);
    }
}

#[test]
fn refresh_refuses_what_it_cannot_rescan_in_place() {
    let f = fx();
    f.write_file("a/sub/f", 8192);
    f.write_file("k/g", 4096);
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
    let usage = |rel: &str, message: String| {
        let e = f.try_refresh(rel).unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (ErrorCode::Usage, message.as_str())
        );
        assert_eq!(e.fix, None);
    };
    usage("k/g", format!("{} is not a directory", f.s("k/g")));

    // a/ moved to b/ and a symlink left in its place
    fs::rename(f.p("a"), f.p("b")).unwrap();
    std::os::unix::fs::symlink("b", f.p("a")).unwrap();
    usage(
        "a/sub",
        format!(
            "{} resolves to {} on disk (symlink?); refresh the real path",
            f.s("a/sub"),
            f.s("b/sub")
        ),
    );

    let gone = f.root.with_extension("gone");
    fs::rename(&f.root, &gone).unwrap();
    usage(
        "",
        format!("root {} no longer exists; run a full scan", f.s("")),
    );
    fs::rename(&gone, &f.root).unwrap();

    // a usage error is the caller's, not the run's: nothing in state.json
    assert_eq!(f.last_error(), None);
    assert_eq!(Snapshot::open_current(&f.dir).unwrap().meta.generation, 1);
}

#[test]
fn diff_of_a_path_in_neither_snapshot() {
    let f = fx();
    f.write_file("a/x", 4096);
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
    f.write_file("a/y", 4096);
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
    let e = f.diff("nope").unwrap_err();
    assert_eq!(e.code, ErrorCode::PathNotFound);
    assert_eq!(
        e.message,
        format!(
            "{} is in neither the current nor the previous snapshot",
            f.s("nope")
        )
    );
    assert_eq!(e.fix.as_deref(), Some("freespace ls /"));
}

#[test]
fn diff_joins_same_hash_by_raw_name() {
    let f = fx();
    f.write_file("a/x", 4096);
    f.write_file("b/y", 4096);
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
    f.write_file("b/z", 8192);
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
    // the previous snapshot's a/ renamed, its path hash kept: one hash, two names
    let prev = rusqlite::Connection::open(f.dir.prev()).unwrap();
    let n = prev
        .execute(
            "UPDATE entries SET name = ?1 WHERE name = ?2",
            rusqlite::params![b"other".as_slice(), b"a".as_slice()],
        )
        .unwrap();
    assert_eq!(n, 1);
    drop(prev);
    let response = f.diff("").unwrap();
    let a = response
        .entries
        .iter()
        .find(|e| e.path == f.s("a"))
        .unwrap();
    let other = response
        .entries
        .iter()
        .find(|e| e.path == f.s("other"))
        .unwrap();
    assert_eq!(a.status, queries::types::DiffStatus::New);
    assert_eq!(other.status, queries::types::DiffStatus::Gone);
}

#[test]
fn diff_keeps_gone_rows_with_duplicate_hashes() {
    let f = fx();
    f.write_file("a/x", 4096);
    f.write_file("b/y", 4096);
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
    fs::remove_dir_all(f.p("b")).unwrap();
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();

    // Make the previous b/ hash collide with a/. The exact-name join must
    // still consume a/ and retain b/ as a gone row.
    let prev = rusqlite::Connection::open(f.dir.prev()).unwrap();
    let hash: i64 = prev
        .query_row(
            "SELECT path_hash FROM entries WHERE name = ?1",
            [b"a".as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    let n = prev
        .execute(
            "UPDATE entries SET path_hash = ?1 WHERE name = ?2",
            rusqlite::params![hash, b"b".as_slice()],
        )
        .unwrap();
    assert_eq!(n, 1);
    drop(prev);

    let response = f.diff("").unwrap();
    assert!(
        response
            .entries
            .iter()
            .any(|e| e.path == f.s("a") && e.status == queries::types::DiffStatus::Same)
    );
    assert!(
        response
            .entries
            .iter()
            .any(|e| e.path == f.s("b") && e.status == queries::types::DiffStatus::Gone)
    );
}

#[test]
fn diff_selects_duplicate_key_candidate_with_matching_parent() {
    let f = fx();
    f.write_file("a/x/file", 4096);
    f.write_file("b/x/file", 4096);
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
    fs::remove_dir_all(f.p("b")).unwrap();
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();

    // At depth 2 both previous rows are named x. Force their path hashes to
    // match, leaving parent hash as the discriminator for a/x.
    let prev = rusqlite::Connection::open(f.dir.prev()).unwrap();
    let hash: i64 = prev
        .query_row(
            "SELECT x.path_hash FROM entries x JOIN entries p ON p.id = x.parent_id WHERE x.name = ?1 AND p.name = ?2",
            rusqlite::params![b"x".as_slice(), b"a".as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    let n = prev
        .execute(
            "UPDATE entries SET path_hash = ?1 WHERE name = ?2 AND kind = 1",
            rusqlite::params![hash, b"x".as_slice()],
        )
        .unwrap();
    assert_eq!(n, 2);
    drop(prev);

    let response = diff::diff(
        &f.dir,
        &DiffParams {
            path: Some(f.s("")),
            depth: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        response
            .entries
            .iter()
            .any(|e| e.path == f.s("a/x") && e.status == queries::types::DiffStatus::Same)
    );
    assert!(
        response
            .entries
            .iter()
            .any(|e| e.path == f.s("b/x") && e.status == queries::types::DiffStatus::Gone)
    );
}

#[test]
fn diff_matches_repeated_names_by_exact_parent_path() {
    let f = fx();
    f.write_file("a/x/file", 4096);
    f.write_file("b/x/file", 4096);
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();
    f.scan_roots(vec![f.root.clone()], 2, true).unwrap();

    // Force both the child and parent hashes to collide in both snapshots.
    // The matcher must pair each repeated x/ with its actual raw parent path.
    for path in [f.dir.current(), f.dir.prev()] {
        let conn = rusqlite::Connection::open(path).unwrap();
        let parent_hash: i64 = conn
            .query_row(
                "SELECT path_hash FROM entries WHERE name = ?1 AND kind = 1",
                [b"a".as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        let child_hash: i64 = conn
            .query_row(
                "SELECT x.path_hash FROM entries x JOIN entries p ON p.id = x.parent_id WHERE x.name = ?1 AND p.name = ?2",
                rusqlite::params![b"x".as_slice(), b"a".as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            conn.execute(
                "UPDATE entries SET path_hash = ?1 WHERE name IN (?2, ?3) AND kind = 1",
                rusqlite::params![parent_hash, b"a".as_slice(), b"b".as_slice()],
            )
            .unwrap(),
            2
        );
        assert_eq!(
            conn.execute(
                "UPDATE entries SET path_hash = ?1 WHERE name = ?2 AND kind = 1",
                rusqlite::params![child_hash, b"x".as_slice()],
            )
            .unwrap(),
            2
        );
    }

    let response = diff::diff(
        &f.dir,
        &DiffParams {
            path: Some(f.s("")),
            depth: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    for path in ["a/x", "b/x"] {
        assert!(
            response
                .entries
                .iter()
                .any(|e| { e.path == f.s(path) && e.status == queries::types::DiffStatus::Same })
        );
    }
}

#[test]
fn a_scan_of_no_roots_is_a_config_error_kept_in_state() {
    let f = fx();
    let e = f.scan_roots(Vec::new(), 2, true).unwrap_err();
    assert_eq!(e.code, ErrorCode::Config);
    assert_eq!(e.message, "no roots to scan");
    assert_eq!(
        e.fix.as_deref(),
        Some("pass a path (freespace scan /) or set roots in /etc/freespace/config.toml")
    );
    let le = f.last_error().unwrap();
    assert_eq!(
        (le.op.as_str(), le.message.as_str()),
        ("scan", "no roots to scan")
    );
    assert!(Snapshot::open_current(&f.dir).is_err());
}
