//! Publish protocol (one for `scan` and `refresh`), split into steps that are
//! tested alone (T8):
//! 1. `precheck` before writing; 2. tmp written + fsynced by the writer;
//! 3. `rotate_prev` (scan with keep_previous only): link current → prev.tmp → rename to prev;
//! 4. `publish_current`: rename tmp → snapshot.db, fsync the directory.

use std::path::Path;

use super::{RESERVE_BYTES, SnapshotDir, fsutil};
use crate::error::{Error, ErrorCode, Result};
use crate::format;
use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrecheckInfo {
    pub need: u64,
    pub avail: u64,
    pub estimate: u64,
    pub factor: u64,
}

/// Bytes per entry used for the size estimate before the first snapshot exists
/// (OV-4: plan said 100 B; measured ~135 B with indexes, see docs/benchmarks.md).
pub const BYTES_PER_ENTRY_ESTIMATE: u64 = 150;

/// `free ≥ N × max(size(current), 150 B × entries) + 512 MB`.
pub fn required_space(
    current_size: u64,
    entries: Option<u64>,
    used: u64,
    factor: u64,
) -> (u64, u64) {
    let estimate = match entries {
        Some(n) => current_size.max(BYTES_PER_ENTRY_ESTIMATE * n),
        None => current_size.max((1u64 << 30).max(used / 50)),
    };
    (factor * estimate + RESERVE_BYTES, estimate)
}

pub fn precheck(dir: &SnapshotDir, entries: Option<u64>, factor: u64) -> Result<PrecheckInfo> {
    let st = platform::statvfs(dir.as_path()).map_err(|e| {
        Error::new(
            ErrorCode::Space,
            format!("statvfs {} failed: {e}", dir.path.display()),
        )
    })?;
    let current = std::fs::metadata(dir.current())
        .map(|m| m.len())
        .unwrap_or(0);
    let (need, estimate) = required_space(current, entries, st.used, factor);
    let info = PrecheckInfo {
        need,
        avail: st.avail,
        estimate,
        factor,
    };
    if st.avail < need {
        return Err(Error::new(
            ErrorCode::Space,
            format!(
                "not enough space for a new snapshot in {}: need {} ({} × {} + {} reserve), have {}",
                dir.path.display(),
                format::size(need),
                factor,
                format::size(estimate),
                format::size(RESERVE_BYTES),
                format::size(st.avail)
            ),
        )
        .with_fix(
            "free space on that filesystem, set keep_previous = false, or move snapshot_dir in /etc/freespace/config.toml",
        ));
    }
    Ok(info)
}

/// Remove leftovers of an interrupted run (called under the lock).
pub fn cleanup_tmp(dir: &SnapshotDir) {
    if let Ok(rd) = std::fs::read_dir(dir.as_path()) {
        for e in rd.flatten() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            if n.ends_with(".tmp") && (n.starts_with("snapshot.") || n.ends_with(".json.tmp")) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn step_err(step: &str, a: &Path, b: &Path, e: std::io::Error) -> Error {
    let mut err = Error::generic(format!(
        "publish step {step} failed ({} → {}): {e}",
        a.display(),
        b.display()
    ));
    err.last_error = Some(crate::snapshot::state::LastError {
        ts: platform::now_secs(),
        op: "publish".into(),
        message: err.message.clone(),
    });
    err
}

/// Hardlink (not copy) the current snapshot to prev: readers of the old inode
/// keep reading it. No-op without a current snapshot.
pub fn rotate_prev(dir: &SnapshotDir) -> Result<bool> {
    let cur = dir.current();
    if !cur.exists() {
        return Ok(false);
    }
    let tmp = dir.prev_tmp();
    let _ = std::fs::remove_file(&tmp);
    std::fs::hard_link(&cur, &tmp).map_err(|e| step_err("link", &cur, &tmp, e))?;
    std::fs::rename(&tmp, dir.prev()).map_err(|e| step_err("rename-prev", &tmp, &dir.prev(), e))?;
    Ok(true)
}

pub fn publish_current(dir: &SnapshotDir, tmp: &Path) -> Result<()> {
    let cur = dir.current();
    std::fs::rename(tmp, &cur).map_err(|e| step_err("rename", tmp, &cur, e))?;
    fsutil::fsync_dir(dir.as_path())
        .map_err(|e| step_err("fsync-dir", dir.as_path(), dir.as_path(), e))?;
    Ok(())
}

pub fn drop_prev(dir: &SnapshotDir) {
    let _ = std::fs::remove_file(dir.prev());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn required_space_formula() {
        // first scan: no current file, 7M entries → 3 × 7M × 150 B + 512 MB
        let (need, est) = required_space(0, Some(7_000_000), 0, 3);
        assert_eq!(est, 7_000_000 * BYTES_PER_ENTRY_ESTIMATE);
        assert_eq!(need, 3 * est + RESERVE_BYTES);
        // current bigger than the estimate wins
        let (_, est) = required_space(1_200_000_000, Some(7_000_000), 0, 2);
        assert_eq!(est, 1_200_000_000);
        // unknown entries: max(1 GB, 2% of used)
        let (_, est) = required_space(0, None, 400 << 30, 3);
        assert_eq!(est, (400u64 << 30) / 50);
    }

    #[test]
    fn precheck_fails_when_need_exceeds_avail() {
        let d = tempfile::tempdir().unwrap();
        let dir = SnapshotDir::new(d.path());
        let e = precheck(&dir, Some(u64::MAX / 1000), 3).unwrap_err();
        assert_eq!(e.code, ErrorCode::Space);
        assert!(precheck(&dir, Some(10), 3).is_ok());
    }

    #[test]
    fn rotate_then_publish_keeps_old_inode_as_prev() {
        let d = tempfile::tempdir().unwrap();
        let dir = SnapshotDir::new(d.path());
        assert!(!rotate_prev(&dir).unwrap());
        std::fs::write(dir.current(), b"gen1").unwrap();
        let ino1 = std::fs::metadata(dir.current()).unwrap().ino();
        let tmp = dir.tmp(2);
        std::fs::write(&tmp, b"gen2").unwrap();
        assert!(rotate_prev(&dir).unwrap());
        // crash window between steps: prev and current are one inode
        assert_eq!(std::fs::metadata(dir.prev()).unwrap().ino(), ino1);
        assert_eq!(std::fs::metadata(dir.current()).unwrap().ino(), ino1);
        publish_current(&dir, &tmp).unwrap();
        assert_eq!(std::fs::read(dir.current()).unwrap(), b"gen2");
        assert_eq!(std::fs::read(dir.prev()).unwrap(), b"gen1");
        assert!(!tmp.exists());
        assert!(!dir.prev_tmp().exists());
    }

    #[test]
    fn cleanup_removes_only_our_tmp_files() {
        let d = tempfile::tempdir().unwrap();
        let dir = SnapshotDir::new(d.path());
        std::fs::write(dir.tmp(5), b"x").unwrap();
        std::fs::write(dir.prev_tmp(), b"x").unwrap();
        std::fs::write(d.path().join("state.json.tmp"), b"x").unwrap();
        std::fs::write(d.path().join("other.tmp"), b"x").unwrap();
        cleanup_tmp(&dir);
        assert!(!dir.tmp(5).exists());
        assert!(!dir.prev_tmp().exists());
        assert!(!d.path().join("state.json.tmp").exists());
        assert!(d.path().join("other.tmp").exists());
    }
}
