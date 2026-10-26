//! Shared real-tree fixtures for core integration tests.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use freespace_core::Error;
use freespace_core::job::{self, ScanReport, ScanRequest};
use freespace_core::refresh::{self, RefreshReport, RefreshRequest};
use freespace_core::snapshot::SnapshotDir;

pub(super) struct Fx {
    _tmp: tempfile::TempDir,
    pub(super) root: PathBuf,
    pub(super) dir: SnapshotDir,
}

pub(super) fn fx() -> Fx {
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
    pub(super) fn p(&self, rel: &str) -> PathBuf {
        if rel.is_empty() {
            self.root.clone()
        } else {
            self.root.join(rel)
        }
    }

    pub(super) fn s(&self, rel: &str) -> String {
        self.p(rel).to_str().unwrap().to_owned()
    }

    pub(super) fn write_file(&self, rel: &str, size: usize) {
        let p = self.p(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, vec![7u8; size]).unwrap();
    }

    pub(super) fn scan_roots(
        &self,
        roots: Vec<PathBuf>,
        workers: usize,
        keep_previous: bool,
    ) -> Result<ScanReport, Error> {
        job::run_scan(ScanRequest {
            roots,
            one_file_system: true,
            workers,
            dir: self.dir.clone(),
            keep_previous,
            on_progress: None,
        })
    }

    pub(super) fn try_refresh(&self, rel: &str) -> Result<RefreshReport, Error> {
        refresh::run_refresh(RefreshRequest {
            path: self.p(rel).as_os_str().as_bytes().to_vec(),
            workers: 2,
            dir: self.dir.clone(),
            on_progress: None,
        })
    }
}
