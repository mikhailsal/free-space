//! Snapshot directory: files, modes, lock, state, progress, writer, publish
//! protocol and the read-only reader.

pub mod fsutil;
pub mod lock;
pub mod progress;
pub mod publish;
pub mod reader;
pub mod schema;
pub mod state;
pub mod v2;
pub mod writer;

use std::path::{Path, PathBuf};

pub const CURRENT: &str = "snapshot.db";
pub const PREV: &str = "snapshot.prev.db";
pub const PREV_TMP: &str = "snapshot.prev.tmp";
pub const LOCK: &str = ".lock";
pub const STATE: &str = "state.json";
pub const PROGRESS: &str = "progress.json";

/// Minimum free space kept on the snapshot filesystem (precheck and reserve).
pub const RESERVE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotDir {
    pub path: PathBuf,
}

impl SnapshotDir {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        SnapshotDir { path: path.into() }
    }

    pub fn current(&self) -> PathBuf {
        self.path.join(CURRENT)
    }

    pub fn prev(&self) -> PathBuf {
        self.path.join(PREV)
    }

    pub fn prev_tmp(&self) -> PathBuf {
        self.path.join(PREV_TMP)
    }

    pub fn tmp(&self, generation: u64) -> PathBuf {
        self.path.join(format!("snapshot.{generation}.tmp"))
    }

    pub fn lock(&self) -> PathBuf {
        self.path.join(LOCK)
    }

    pub fn state(&self) -> PathBuf {
        self.path.join(STATE)
    }

    pub fn progress(&self) -> PathBuf {
        self.path.join(PROGRESS)
    }

    pub fn as_path(&self) -> &Path {
        &self.path
    }
}
