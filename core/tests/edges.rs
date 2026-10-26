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
fn diff_refuses_two_rows_of_one_hash() {
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
    let e = f.diff("").unwrap_err();
    assert_eq!(e.code, ErrorCode::HashCollision);
    assert_eq!(
        e.message,
        format!(
            "path hash collision between {} (current) and {} (prev)",
            f.s("a"),
            f.s("other")
        )
    );
    assert_eq!(
        e.fix.as_deref(),
        Some(
            "report this at https://github.com/mikhailsal/free-space/issues; ls and top are unaffected"
        )
    );
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
