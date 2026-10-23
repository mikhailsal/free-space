//! Read-only snapshot connections for `serve`, keyed by the inode of
//! `snapshot.db` (OV-6): every request stats the file first, a new inode drops
//! the pool, and idle connections close after 5 s, so a rotated-out generation
//! stays open for one in-flight request at most.

use std::os::unix::fs::MetadataExt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use freespace_core::error::Result;
use freespace_core::snapshot::SnapshotDir;
use freespace_core::snapshot::reader::Snapshot;

const MAX_IDLE: usize = 4;
pub const IDLE: Duration = Duration::from_secs(5);

type Key = Option<(u64, u64)>;

pub struct Pool {
    dir: SnapshotDir,
    inner: Mutex<Inner>,
}

struct Inner {
    key: Key,
    idle: Vec<(Snapshot, Instant)>,
}

impl Pool {
    pub fn new(dir: SnapshotDir) -> Pool {
        Pool {
            dir,
            inner: Mutex::new(Inner {
                key: None,
                idle: Vec::new(),
            }),
        }
    }

    fn key(&self) -> Key {
        std::fs::metadata(self.dir.current())
            .ok()
            .map(|m| (m.dev(), m.ino()))
    }

    /// A connection to the current snapshot and the inode it was taken under.
    pub fn get(&self) -> Result<(Snapshot, Key)> {
        let key = self.key();
        {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if g.key != key {
                g.idle.clear();
                g.key = key;
            }
            if let Some((s, _)) = g.idle.pop() {
                return Ok((s, key));
            }
        }
        // Errors (no snapshot, no access, schema) come from the open itself.
        Snapshot::open_current(&self.dir).map(|s| (s, key))
    }

    /// Return a connection; dropped if the snapshot rotated meanwhile.
    pub fn put(&self, s: Snapshot, key: Key) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if key.is_some() && g.key == key && g.idle.len() < MAX_IDLE {
            g.idle.push((s, Instant::now()));
        }
    }

    /// Close connections idle for longer than [`IDLE`].
    pub fn reap(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.idle.retain(|(_, t)| t.elapsed() < IDLE);
    }

    #[cfg(test)]
    pub fn idle(&self) -> usize {
        self.inner.lock().unwrap().idle.len()
    }
}
