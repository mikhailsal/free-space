//! `.lock` in the snapshot directory: `scan` and `refresh` hold `LOCK_EX` and
//! write `{op, path, started_at, pid}` into it (T31); `status` probes with
//! `LOCK_SH|LOCK_NB` to report a running scan without blocking.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::SnapshotDir;
use super::fsutil;
use crate::error::{Error, ErrorCode, Result};
use crate::platform::now_secs;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockPayload {
    pub op: String,
    pub path: String,
    pub started_at: i64,
    pub pid: u32,
}

/// A scan or refresh in progress (`status.scanning`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Scanning {
    /// "scan" or "refresh"
    pub op: String,
    pub path: String,
    pub started_at: i64,
    pub elapsed_seconds: i64,
    pub pid: u32,
}

impl Scanning {
    fn from_payload(p: LockPayload) -> Self {
        Scanning {
            elapsed_seconds: (now_secs() - p.started_at).max(0),
            op: p.op,
            path: p.path,
            started_at: p.started_at,
            pid: p.pid,
        }
    }
}

#[derive(Debug)]
pub struct LockGuard {
    file: File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn try_flock(f: &File, op: libc::c_int) -> bool {
    unsafe { libc::flock(f.as_raw_fd(), op | libc::LOCK_NB) == 0 }
}

fn read_payload(f: &mut File) -> Option<LockPayload> {
    let mut s = String::new();
    f.seek(SeekFrom::Start(0)).ok()?;
    f.read_to_string(&mut s).ok()?;
    serde_json::from_str(s.trim()).ok()
}

/// Take the exclusive lock, retrying for 2 s (a concurrent `status` probe holds
/// `LOCK_SH` for microseconds), then fail with exit 6 naming the holder.
pub fn acquire(dir: &SnapshotDir, op: &str, path: &str) -> Result<LockGuard> {
    acquire_with_timeout(dir, op, path, Duration::from_secs(2))
}

pub fn acquire_with_timeout(
    dir: &SnapshotDir,
    op: &str,
    path: &str,
    timeout: Duration,
) -> Result<LockGuard> {
    let p = dir.lock();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(fsutil::FILE_MODE)
        .open(&p)
        .map_err(|e| Error::io(format!("open {}", p.display()), e))?;
    fsutil::secure_file(&file).map_err(|e| Error::io(format!("chmod {}", p.display()), e))?;
    let deadline = Instant::now() + timeout;
    loop {
        if try_flock(&file, libc::LOCK_EX) {
            break;
        }
        if Instant::now() >= deadline {
            let holder = read_payload(&mut file).map(Scanning::from_payload);
            let msg = match &holder {
                Some(h) => format!(
                    "another {} is running: pid {} on {} since {}",
                    h.op,
                    h.pid,
                    h.path,
                    crate::format::local_hm(h.started_at)
                ),
                None => "another scan is running".to_string(),
            };
            let mut e = Error::new(ErrorCode::Lock, msg)
                .with_fix("wait for it to finish: freespace status shows progress");
            e.scanning = holder;
            return Err(e);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let payload = LockPayload {
        op: op.to_string(),
        path: path.to_string(),
        started_at: now_secs(),
        pid: std::process::id(),
    };
    let bytes = serde_json::to_vec(&payload).unwrap_or_default();
    file.set_len(0)
        .and_then(|_| file.seek(SeekFrom::Start(0)))
        .and_then(|_| file.write_all(&bytes))
        .map_err(|e| Error::io(format!("write {}", p.display()), e))?;
    Ok(LockGuard { file })
}

/// `Some` when a scan or refresh holds the lock. Never blocks.
pub fn probe(dir: &SnapshotDir) -> Option<Scanning> {
    let mut f = File::open(dir.lock()).ok()?;
    if try_flock(&f, libc::LOCK_SH) {
        unsafe {
            libc::flock(f.as_raw_fd(), libc::LOCK_UN);
        }
        return None;
    }
    Some(
        read_payload(&mut f)
            .map(Scanning::from_payload)
            .unwrap_or(Scanning {
                op: "scan".into(),
                path: String::new(),
                started_at: now_secs(),
                elapsed_seconds: 0,
                pid: 0,
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_with_lock_and_probe_sees_holder() {
        let d = tempfile::tempdir().unwrap();
        let dir = SnapshotDir::new(d.path());
        assert!(probe(&dir).is_none());
        let g = acquire(&dir, "scan", "/").unwrap();
        let s = probe(&dir).expect("probe sees the scan");
        assert_eq!(s.op, "scan");
        assert_eq!(s.pid, std::process::id());
        let e =
            acquire_with_timeout(&dir, "refresh", "/x", Duration::from_millis(100)).unwrap_err();
        assert_eq!(e.code, ErrorCode::Lock);
        assert_eq!(e.exit(), 6);
        assert!(e.scanning.is_some());
        drop(g);
        assert!(probe(&dir).is_none());
        acquire(&dir, "scan", "/").unwrap();
    }

    #[test]
    fn probe_loop_does_not_make_acquire_fail() {
        let d = tempfile::tempdir().unwrap();
        let dir = SnapshotDir::new(d.path());
        File::create(dir.lock()).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s2 = stop.clone();
        let dir2 = dir.clone();
        let h = std::thread::spawn(move || {
            while !s2.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = probe(&dir2);
            }
        });
        for _ in 0..20 {
            let g = acquire(&dir, "scan", "/").unwrap();
            drop(g);
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        h.join().unwrap();
    }
}
