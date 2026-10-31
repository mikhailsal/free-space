//! Optional benchmark markers. Boundary RSS is a sample, not a phase peak;
//! VmHWM is cumulative. The external benchmark also samples RSS between markers.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use super::tree::{Node, Tree};

struct Trace {
    file: File,
    started: Instant,
    previous_buffers: Option<(usize, usize)>,
}

static TRACE: OnceLock<Option<Mutex<Trace>>> = OnceLock::new();

/// Diagnostics never change scan success/failure. The benchmark rejects missing
/// or incomplete traces instead of presenting them as valid phase measurements.
pub(crate) fn phase(name: &str) {
    record(name, None);
}

/// Record vector capacities and whether their buffers moved since the last allocation event.
pub(crate) fn allocation(name: &str, tree: &Tree) {
    record(name, Some(tree));
}

fn record(name: &str, tree: Option<&Tree>) {
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
            previous_buffers: None,
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
    let mut event = serde_json::json!({
        "phase": name, "elapsed_seconds": trace.started.elapsed().as_secs_f64(),
        "rss_kib": value("VmRSS:"), "hwm_kib": value("VmHWM:"),
    });
    if let Some(tree) = tree {
        let buffers = (tree.nodes.as_ptr() as usize, tree.names.as_ptr() as usize);
        let previous = trace.previous_buffers.replace(buffers);
        event["node_len"] = tree.nodes.len().into();
        event["node_capacity"] = tree.nodes.capacity().into();
        event["name_len"] = tree.names.len().into();
        event["name_capacity"] = tree.names.capacity().into();
        event["node_size_bytes"] = std::mem::size_of::<Node>().into();
        event["node_buffer_changed"] = previous.is_some_and(|p| p.0 != buffers.0).into();
        event["name_buffer_changed"] = previous.is_some_and(|p| p.1 != buffers.1).into();
    }
    let _ = writeln!(trace.file, "{event}");
}
