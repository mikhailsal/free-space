use std::os::unix::ffi::OsStrExt;

use super::common::refreshed;
use crate::queries::common::lossy;
use crate::queries::types::{FsOut, PrevOut, RootOut, StatusResponse};
use crate::snapshot::{SnapshotDir, lock, progress, state, v2::Reader};
use crate::{Result, platform};

pub fn status(reader: &Reader, dir: &SnapshotDir) -> Result<StatusResponse> {
    status_page(Some(reader), dir)
}

/// The web first-run response can be built before a generation exists.
pub fn status_page(reader: Option<&Reader>, dir: &SnapshotDir) -> Result<StatusResponse> {
    let scanning = lock::probe(dir);
    let progress = scanning
        .as_ref()
        .filter(|operation| operation.op != "maintain")
        .and_then(|_| progress::read(dir));
    let (state, state_error) = match state::read(dir) {
        Ok(state) => (state, None),
        Err(error) => (None, Some(error)),
    };
    let mut response = StatusResponse {
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
        last_error: state.as_ref().and_then(|s| s.last_error.clone()),
        last_success_ts: state.as_ref().and_then(|s| s.last_success_ts),
        warnings: state
            .as_ref()
            .map(|s| s.warnings.clone())
            .unwrap_or_default(),
        state_error,
        scanning,
        progress,
    };
    if let Some(reader) = reader {
        reader.with_pin(|reader| populate(reader, &mut response))?;
    }
    Ok(response)
}

fn populate(reader: &Reader, response: &mut StatusResponse) -> Result<()> {
    let meta = reader.meta(false)?;
    response.host = meta.host;
    response.os = meta.os;
    response.generation = Some(meta.generation);
    response.prev_generation = meta.prev_generation;
    response.schema_version = Some(meta.schema_version);
    response.snapshot_id = Some(meta.snapshot_id);
    response.scanned_at = Some(meta.scanned_at);
    response.entries = Some(meta.entries);
    response.refreshed = refreshed(reader)?;
    let (incomplete, unknown) = reader.unknown_subtrees(false, 100)?;
    response.incomplete = incomplete;
    response.unknown_subtrees = unknown.iter().map(|p| lossy(p)).collect();
    if reader.generations.prev.is_some() {
        let prev = reader.meta(true)?;
        response.prev = Some(PrevOut {
            generation: prev.generation,
            scanned_at: prev.scanned_at,
        });
    }
    let mut oldest = meta.scanned_at;
    for root in reader.roots(false)? {
        let entry = reader
            .get_id(root.entry_id, false)?
            .ok_or_else(|| crate::Error::generic("root entry missing"))?;
        oldest = oldest.min(entry.min_scanned_at);
        let live = platform::statvfs(std::path::Path::new(std::ffi::OsStr::from_bytes(
            &root.path,
        )))
        .ok();
        let path = lossy(&root.path);
        response.fs.push(FsOut {
            path: path.clone(),
            fs_type: root.fs_type.clone(),
            total: live.map_or(root.stats.total, |s| s.total),
            used: live.map_or(root.stats.used, |s| s.used),
            avail: live.map(|s| s.avail),
        });
        response.roots.push(RootOut {
            path,
            fs_type: root.fs_type,
            allocated: entry.allocated as u64,
            items: entry.items as u64,
            fs_used: root.stats.used,
            fs_total: root.stats.total,
            fs_avail: root.stats.avail,
        });
    }
    response.age_seconds = Some((platform::now_secs() - oldest).max(0));
    Ok(())
}
