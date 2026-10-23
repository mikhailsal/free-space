//! `progress.json`, rewritten once per second while a scan runs (D14):
//! entries seen vs. the used-inode count of the roots' filesystems, clamped to
//! 99% until publish.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{SnapshotDir, fsutil};
use crate::platform::now_secs;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Progress {
    pub op: String,
    pub path: String,
    /// "walk", "write" or "publish"
    pub stage: String,
    pub started_at: i64,
    pub updated_at: i64,
    pub elapsed_seconds: i64,
    pub entries_seen: u64,
    /// Used inodes of the scanned filesystems; null when unavailable.
    pub entries_expected: Option<u64>,
    /// 0–99 until publish; null without `entries_expected`.
    pub percent: Option<u8>,
}

pub fn percent(seen: u64, expected: Option<u64>) -> Option<u8> {
    let e = expected?;
    if e == 0 {
        return Some(0);
    }
    Some(((seen.saturating_mul(100)) / e).min(99) as u8)
}

/// Called after each progress.json write (CLI prints it on stderr).
pub type ProgressCallback = Box<dyn Fn(&Progress) + Send>;

pub const STAGE_WALK: u8 = 0;
pub const STAGE_WRITE: u8 = 1;
pub const STAGE_PUBLISH: u8 = 2;

fn stage_name(s: u8) -> &'static str {
    match s {
        STAGE_WALK => "walk",
        STAGE_WRITE => "write",
        _ => "publish",
    }
}

pub struct ProgressWriter {
    stop: Arc<AtomicBool>,
    pub stage: Arc<AtomicU8>,
    handle: Option<JoinHandle<()>>,
    dir: SnapshotDir,
    last: Arc<Mutex<Option<Progress>>>,
}

impl ProgressWriter {
    pub fn start(
        dir: &SnapshotDir,
        op: &str,
        path: &str,
        seen: Arc<AtomicU64>,
        expected: Option<u64>,
        on_tick: Option<ProgressCallback>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stage = Arc::new(AtomicU8::new(STAGE_WALK));
        let last = Arc::new(Mutex::new(None));
        let started_at = now_secs();
        let (dir2, op, path) = (dir.clone(), op.to_string(), path.to_string());
        let (stop2, stage2, last2) = (stop.clone(), stage.clone(), last.clone());
        let handle = std::thread::spawn(move || {
            let mut first = true;
            loop {
                if !first {
                    for _ in 0..10 {
                        if stop2.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                first = false;
                let now = now_secs();
                let s = seen.load(Ordering::Relaxed);
                let p = Progress {
                    op: op.clone(),
                    path: path.clone(),
                    stage: stage_name(stage2.load(Ordering::Relaxed)).to_string(),
                    started_at,
                    updated_at: now,
                    elapsed_seconds: now - started_at,
                    entries_seen: s,
                    entries_expected: expected,
                    percent: percent(s, expected),
                };
                if let Ok(b) = serde_json::to_vec(&p) {
                    let _ = fsutil::write_atomic(&dir2.progress(), &b);
                }
                if let Some(cb) = &on_tick {
                    cb(&p);
                }
                *last2.lock().unwrap() = Some(p);
            }
        });
        ProgressWriter {
            stop,
            stage,
            handle: Some(handle),
            dir: dir.clone(),
            last,
        }
    }

    pub fn set_stage(&self, s: u8) {
        self.stage.store(s, Ordering::Relaxed);
    }

    pub fn last(&self) -> Option<Progress> {
        self.last.lock().unwrap().clone()
    }
}

impl Drop for ProgressWriter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let _ = std::fs::remove_file(self.dir.progress());
    }
}

pub fn read(dir: &SnapshotDir) -> Option<Progress> {
    let b = std::fs::read(dir.progress()).ok()?;
    serde_json::from_slice(&b).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_below_100_until_publish() {
        assert_eq!(percent(50, Some(100)), Some(50));
        // hardlinks and other filesystems' inodes can push seen past expected
        assert_eq!(percent(150, Some(100)), Some(99));
        assert_eq!(percent(10, None), None);
    }

    #[test]
    fn writes_and_removes_file() {
        let d = tempfile::tempdir().unwrap();
        let dir = SnapshotDir::new(d.path());
        let seen = Arc::new(AtomicU64::new(7));
        let w = ProgressWriter::start(&dir, "scan", "/", seen, Some(10), None);
        let mut ok = false;
        for _ in 0..50 {
            if let Some(p) = read(&dir) {
                assert_eq!(p.entries_seen, 7);
                assert_eq!(p.percent, Some(70));
                ok = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(ok);
        drop(w);
        assert!(read(&dir).is_none());
    }
}
