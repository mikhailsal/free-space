//! The roots of a coarse scan: resolved, checked against each other and sized.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::model::{RootMeta, threshold};
use crate::error::{Error, ErrorCode, Result};
use crate::platform::{self, MountInfo};
use crate::scan::walk::{normalize_root, validate_root_nesting};
use crate::scan::walk_error;

/// Canonical, de-duplicated roots with the filesystem facts the snapshot records and the
/// folding threshold each one gets.
pub fn prepare(roots: &[PathBuf], ppm: u32, mounts: &[MountInfo]) -> Result<Vec<RootMeta>> {
    if roots.is_empty() {
        return Err(Error::new(ErrorCode::Config, "no roots to scan").with_fix(
            "pass a path (freespace scan /) or set roots in /etc/freespace/config.toml",
        ));
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    for root in roots {
        let path = normalize_root(root).map_err(walk_error)?;
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
        .iter()
        .map(|path| describe(path, &paths, ppm, mounts))
        .collect()
}

fn describe(path: &Path, all: &[PathBuf], ppm: u32, mounts: &[MountInfo]) -> Result<RootMeta> {
    let dev = std::fs::metadata(path)
        .map_err(|e| Error::io(format!("stat {}", path.display()), e))?
        .dev();
    validate_root_nesting(path, all, dev).map_err(walk_error)?;
    let stats = platform::statvfs(path).unwrap_or_default();
    let fs_type = platform::mount_for(mounts, path)
        .map(|mount| mount.fs_type)
        .unwrap_or_default();
    let threshold = threshold(stats.total, ppm)
        .map_err(|e| Error::generic(format!("{}: {}", path.display(), e.message)))?;
    Ok(RootMeta {
        path: path.to_path_buf(),
        fs_type,
        stats,
        threshold,
    })
}
