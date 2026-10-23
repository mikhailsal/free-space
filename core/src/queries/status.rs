//! `status`: snapshot identity, freshness, roots and filesystems, last error,
//! running scan. Must stay ≤ 50 ms: no systemd calls, no full-table scans.

use std::os::unix::ffi::OsStrExt;

use rusqlite::params;

use super::common::{lossy, unknown_subtrees};
use super::diff::refreshed;
use super::types::{FsOut, PrevOut, RootOut, StatusResponse};
use crate::error::{ErrorCode, Result};
use crate::platform::{self, now_secs};
use crate::snapshot::reader::Snapshot;
use crate::snapshot::{SnapshotDir, lock, progress, state};

pub fn status(dir: &SnapshotDir) -> Result<StatusResponse> {
    build(dir, false)
}

/// `/api/status`: like [`status`], but answers before the first snapshot too
/// (`generation: null`), so the page can show the first-run state with
/// `scanning` and `last_error` instead of an error.
pub fn status_page(dir: &SnapshotDir) -> Result<StatusResponse> {
    build(dir, true)
}

fn build(dir: &SnapshotDir, allow_missing: bool) -> Result<StatusResponse> {
    let scanning = lock::probe(dir);
    let prog = if scanning.is_some() {
        progress::read(dir)
    } else {
        None
    };
    let (st, state_error) = match state::read(dir) {
        Ok(s) => (s, None),
        Err(e) => (None, Some(e)),
    };
    let mut resp = StatusResponse {
        host: platform::hostname(),
        os: platform::os_name(),
        version: crate::VERSION.into(),
        snapshot_dir: dir.path.display().to_string(),
        generation: None,
        prev_generation: None,
        schema_version: None,
        snapshot_id: None,
        scanned_at: None,
        age_seconds: None,
        entries: None,
        prev: None,
        refreshed: Vec::new(),
        roots: Vec::new(),
        fs: Vec::new(),
        incomplete: false,
        unknown_subtrees: Vec::new(),
        last_error: st.as_ref().and_then(|s| s.last_error.clone()),
        last_success_ts: st.as_ref().and_then(|s| s.last_success_ts),
        warnings: st.as_ref().map(|s| s.warnings.clone()).unwrap_or_default(),
        state_error,
        scanning,
        progress: prog,
    };
    let snap = match Snapshot::open_current(dir) {
        Ok(s) => s,
        Err(e) if e.code == ErrorCode::NoSnapshot && (allow_missing || resp.scanning.is_some()) => {
            return Ok(resp);
        }
        Err(e) => return Err(e),
    };
    let m = &snap.meta;
    resp.host = m.host.clone();
    resp.os = m.os.clone();
    resp.generation = Some(m.generation);
    resp.prev_generation = m.prev_generation;
    resp.schema_version = Some(m.schema_version);
    resp.snapshot_id = Some(m.snapshot_id.clone());
    resp.scanned_at = Some(m.scanned_at);
    resp.entries = Some(m.entries);
    resp.refreshed = refreshed(&snap)?;
    let (incomplete, unknown) = unknown_subtrees(&snap.conn, 100)?;
    resp.incomplete = incomplete;
    resp.unknown_subtrees = unknown;

    let mut st = snap.conn.prepare_cached(
        "SELECT r.path, r.fs_type, e.allocated, e.items, r.fs_used, r.fs_total, r.fs_avail, e.min_scanned_at
         FROM roots r JOIN entries e ON e.id = r.entry_id ORDER BY r.entry_id",
    )?;
    let rows = st
        .query_map(params![], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut oldest = m.scanned_at;
    for (path, fs_type, alloc, items, used, total, avail, min_sc) in rows {
        oldest = oldest.min(min_sc);
        let p = lossy(&path);
        let live = platform::statvfs(std::path::Path::new(std::ffi::OsStr::from_bytes(&path))).ok();
        resp.fs.push(FsOut {
            path: p.clone(),
            fs_type: fs_type.clone(),
            total: live.map(|s| s.total).unwrap_or(total as u64),
            used: live.map(|s| s.used).unwrap_or(used as u64),
            avail: live.map(|s| s.avail),
        });
        resp.roots.push(RootOut {
            path: p,
            fs_type,
            allocated: alloc as u64,
            items: items as u64,
            fs_used: used as u64,
            fs_total: total as u64,
            fs_avail: avail as u64,
        });
    }
    resp.age_seconds = Some((now_secs() - oldest).max(0));
    if let Ok(prev) = Snapshot::open(&dir.prev()) {
        resp.prev = Some(PrevOut {
            generation: prev.meta.generation,
            scanned_at: prev.meta.scanned_at,
        });
    }
    Ok(resp)
}
