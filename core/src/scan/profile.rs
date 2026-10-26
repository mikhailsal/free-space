//! Optional benchmark markers. Boundary RSS is a sample, not a phase peak;
//! VmHWM is cumulative. The external benchmark also samples RSS between markers.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

struct Trace {
    file: File,
    started: Instant,
}

static TRACE: OnceLock<Option<Mutex<Trace>>> = OnceLock::new();

/// Diagnostics never change scan success/failure. The benchmark rejects missing
/// or incomplete traces instead of presenting them as valid phase measurements.
pub(crate) fn phase(name: &str) {
    let Some(trace) = TRACE.get_or_init(|| {
        let path = std::env::var_os("FREESPACE_MEMORY_PROFILE")?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .ok()?;
        Some(Mutex::new(Trace {
            file,
            started: Instant::now(),
        }))
    }) else {
        return;
    };
    let Ok(mut trace) = trace.lock() else {
        return;
    };
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return;
    };
    let value = |key: &str| -> Option<u64> {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    let event = serde_json::json!({
        "phase": name, "elapsed_seconds": trace.started.elapsed().as_secs_f64(),
        "rss_kib": value("VmRSS:"), "hwm_kib": value("VmHWM:"),
    });
    let _ = writeln!(trace.file, "{event}");
}
