//! Edges of refresh, diff and scan, pinned before core is split (D0 of
//! docs/designs/code-quality-gates.md): a folder deleted since the scan, a path
//! that now resolves elsewhere, a root gone, a file, a path in neither
//! snapshot, two rows of one hash, a scan of no roots. Each refusal by its
//! code, message and fix, and whether it reaches state.json.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use freespace_core::job::{self, ScanRequest};
use freespace_core::queries::{self, diff, params::*};
use freespace_core::refresh::{self, RefreshReport, RefreshRequest};
use freespace_core::snapshot::SnapshotDir;
use freespace_core::snapshot::reader::Snapshot;
use freespace_core::snapshot::state;
use freespace_core::{Error, ErrorCode};

struct Fx {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    dir: SnapshotDir,
}

fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    fs::create_dir(&root).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snap"));
    Fx {
        _tmp: tmp,
        root,
        dir,
    }
}

impl Fx {
    fn p(&self, rel: &str) -> PathBuf {
        if rel.is_empty() {
            self.root.clone()
        } else {
            self.root.join(rel)
        }
    }

    fn s(&self, rel: &str) -> String {
        self.p(rel).to_str().unwrap().to_owned()
    }

    fn file(&self, rel: &str, size: usize) {
        let p = self.p(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, vec![7u8; size]).unwrap();
    }

    fn scan_roots(&self, roots: Vec<PathBuf>) -> Result<job::ScanReport, Error> {
        job::run_scan(ScanRequest {
            roots,
            one_file_system: true,
            workers: 2,
            dir: self.dir.clone(),
            keep_previous: true,
            on_progress: None,
        })
    }

    fn scan(&self) {
        self.scan_roots(vec![self.root.clone()]).unwrap();
    }

    fn refresh(&self, rel: &str) -> Result<RefreshReport, Error> {
        refresh::run_refresh(RefreshRequest {
            path: self.p(rel).as_os_str().as_bytes().to_vec(),
            workers: 2,
            dir: self.dir.clone(),
            on_progress: None,
        })
    }

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
    f.file("a/b/c", 8192);
    f.file("a/d", 4096);
    f.scan();
    fs::remove_dir_all(f.p("a/b")).unwrap();
    let rep = f.refresh("a/b").unwrap();
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
    f.file("a/sub/f", 8192);
    f.file("k/g", 4096);
    f.scan();
    let usage = |rel: &str, message: String| {
        let e = f.refresh(rel).unwrap_err();
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
    f.file("a/x", 4096);
    f.scan();
    f.file("a/y", 4096);
    f.scan();
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
    f.file("a/x", 4096);
    f.file("b/y", 4096);
    f.scan();
    f.file("b/z", 8192);
    f.scan();
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
    let e = f.scan_roots(Vec::new()).unwrap_err();
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
