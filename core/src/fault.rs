//! Fault-injection points on the publish path (T10, Section 6). Debug builds
//! only; release binaries compile them away.
//!
//! - `FREESPACE_FAULT=<step>`: SIGKILL itself there: no unwinding, no cleanup,
//!   no core dump, exactly what `kill -9` leaves behind.
//! - `FREESPACE_PAUSE=<step>`: write `<snapshot dir>/.pause-<step>` holding the
//!   pid, then wait until the test removes it (a pause point, not a sleep).
//!
//! Steps in order: `walked`, `prechecked`, `written`, `linked`, `rotated`,
//! `published`.

use crate::snapshot::SnapshotDir;

#[cfg(debug_assertions)]
pub fn point(dir: &SnapshotDir, step: &str) {
    if std::env::var("FREESPACE_FAULT").is_ok_and(|v| v == step) {
        // SAFETY: kill(2) on our own pid; it does not return
        unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
    }
    if std::env::var("FREESPACE_PAUSE").is_ok_and(|v| v == step) {
        let f = dir.as_path().join(format!(".pause-{step}"));
        if std::fs::write(&f, std::process::id().to_string()).is_ok() {
            while f.exists() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn point(_dir: &SnapshotDir, _step: &str) {}
