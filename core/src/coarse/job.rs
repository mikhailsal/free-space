//! Scan and refresh of the coarse backend. Both follow v1's protocol (lock, walk, free
//! space check, temporary snapshot, publish, `state.json`) around the coarse walker and
//! the model export, so a crash, a full disk or a concurrent reader behaves as it does
//! for v1 snapshots.

mod group;
mod refresh;
mod scan;

use std::path::Path;

pub use refresh::{Part, RefreshOutcome, RefreshRequest, Rescanned, run_refresh};
pub use scan::{ScanOutcome, ScanRequest, run_scan};

use super::export::Built;
use super::model::RootMeta;
use super::store;
use crate::error::{Error, ErrorCode, Result};
use crate::format;
use crate::platform;
use crate::snapshot::writer::{self, Reserve, WriteMeta, WriteStats};
use crate::snapshot::{SnapshotDir, publish, state};

/// Write the exported snapshot: the v1 tables first, then the coarse side tables into the
/// same file. A snapshot without its side tables must never be published.
fn write_tmp(
    path: &Path,
    built: Built,
    meta: &WriteMeta,
    reserve: &Reserve,
) -> Result<(WriteStats, Vec<String>)> {
    let Built {
        tree,
        fin,
        side,
        denied,
    } = built;
    let mut stats = writer::write_snapshot(path, tree, fin, meta, reserve)?;
    match store::write(path, &side) {
        Ok(bytes) => {
            stats.bytes = bytes;
            Ok((stats, denied))
        }
        Err(e) => {
            let _ = std::fs::remove_file(path);
            Err(e)
        }
    }
}

/// The free-space check shared by scan and refresh; a failure is recorded as the
/// `precheck` step that failed.
fn precheck(dir: &SnapshotDir, rows: u64, factor: u64) -> Result<()> {
    publish::precheck(dir, Some(rows), factor).map_err(|mut e| {
        e.last_error = Some(state::LastError {
            ts: platform::now_secs(),
            op: "precheck".into(),
            message: e.message.clone(),
        });
        e
    })?;
    Ok(())
}

/// Remember a failed operation for `status`. Conditions the user caused or can retry
/// at once (a busy lock, a bad argument, an unknown path) are not failures of the backend.
fn record_failure(dir: &SnapshotDir, op: &str, e: &Error) {
    if matches!(
        e.code,
        ErrorCode::Lock | ErrorCode::Usage | ErrorCode::PathNotFound
    ) {
        return;
    }
    let (op, msg) = match &e.last_error {
        Some(last) => (last.op.clone(), last.message.clone()),
        None => (op.to_string(), e.message.clone()),
    };
    state::record_error(dir, &op, &msg);
}

/// The size below which entries are folded away: the largest threshold of any root.
pub(crate) fn floor_of(roots: &[RootMeta]) -> u64 {
    roots.iter().map(|r| r.threshold).max().unwrap_or(0)
}

/// What `status` shows after a successful operation: unreadable folders, and the fact that
/// this snapshot is a coarse one.
fn warnings(denied: &[String], folded_below: u64) -> Vec<String> {
    let mut out: Vec<String> = crate::job::denied_warning(denied).into_iter().collect();
    out.push(format!(
        "coarse snapshot: entries smaller than {} are folded into [small items] rows",
        format::size(folded_below)
    ));
    out
}

/// How a scan changed the generations beyond the current one; a refresh leaves them be.
struct ScanState {
    prev_generation: Option<u64>,
    seconds: f64,
}

/// Record a published generation in `state.json`.
fn record_success(
    dir: &SnapshotDir,
    generation: u64,
    warnings: Vec<String>,
    scan: Option<ScanState>,
) -> Result<()> {
    let mut st = state::read(dir).ok().flatten().unwrap_or_default();
    st.last_error = None;
    st.last_success_ts = Some(platform::now_secs());
    st.current_generation = Some(generation);
    st.warnings = warnings;
    if let Some(scan) = scan {
        st.prev_generation = scan.prev_generation;
        st.last_scan_seconds = Some(scan.seconds);
    }
    state::write(dir, &st).map_err(|e| Error::io("write state.json", e))
}
