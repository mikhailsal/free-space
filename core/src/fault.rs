//! Fault-injection points on the publish path (T10, Section 6). Debug builds
//! only; release binaries compile them away.
//!
//! - `FREESPACE_FAULT=<step>`: SIGKILL itself there: no unwinding, no cleanup,
//!   no core dump, exactly what `kill -9` leaves behind.
//! - `FREESPACE_PAUSE=<step>`: write `<snapshot dir>/.pause-<step>` holding the
//!   pid, then wait until the test removes it (at most 60 seconds). A missing
//!   controller fails the scan rather than leaving it paused forever.
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
        pause(&f, std::time::Duration::from_secs(60));
    }
}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn point(_dir: &SnapshotDir, _step: &str) {}

#[cfg(debug_assertions)]
fn pause(path: &std::path::Path, timeout: std::time::Duration) {
    std::fs::write(path, std::process::id().to_string())
        .unwrap_or_else(|error| panic!("fault pause cannot write {}: {error}", path.display()));
    assert!(
        wait_for_resume(path, timeout),
        "fault pause timed out waiting for removal of {}",
        path.display()
    );
}

// A monotonic deadline also bounds an orphan after its harness is SIGKILLed.
// Keep the marker on timeout as evidence; do not publish without the controller.
#[cfg(debug_assertions)]
fn wait_for_resume(path: &std::path::Path, timeout: std::time::Duration) -> bool {
    let started = std::time::Instant::now();
    loop {
        match std::fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => return true,
            Err(error) => panic!("fault pause cannot inspect {}: {error}", path.display()),
        }
        let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
            return false;
        };
        if remaining.is_zero() {
            return false;
        }
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{pause, wait_for_resume};
    use std::time::Duration;

    #[test]
    #[should_panic(expected = "fault pause cannot write")]
    fn marker_write_error_does_not_resume_publication() {
        let dir = tempfile::tempdir().unwrap();
        // EISDIR is deterministic even when the tests run as root.
        pause(dir.path(), Duration::from_secs(3));
    }

    #[test]
    fn abandoned_pause_expires_and_preserves_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(".pause-written");
        std::fs::write(&marker, "orphan").unwrap();
        assert!(!wait_for_resume(&marker, Duration::from_millis(20)));
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "orphan");
    }

    #[test]
    #[should_panic(expected = "fault pause cannot inspect")]
    fn metadata_error_does_not_resume_publication() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-directory");
        std::fs::write(&file, "file").unwrap();
        // ENOTDIR is reproducible without root, unlike EACCES under root.
        wait_for_resume(&file.join(".pause-written"), Duration::from_secs(3));
    }

    #[test]
    fn removing_marker_resumes_before_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(".pause-written");
        std::fs::write(&marker, "controller").unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(20));
                std::fs::remove_file(&marker).unwrap();
            });
            assert!(wait_for_resume(&marker, Duration::from_secs(3)));
        });
    }
}
