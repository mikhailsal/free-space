//! What a walk is given and what it returns.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use crate::coarse::buckets::slot_of;
use crate::coarse::model::{Bucket, MountMeta, Node};
use crate::platform::{self, MountInfo};
use crate::scan::walk::{dev_string, statvfs_safe};

/// The filesystem boundary around a walk: what is part of the tree and what is a mount.
pub struct Boundary<'a> {
    pub dev: u64,
    /// Mount points on the same device that show another part of it: `st_dev` cannot
    /// tell them apart, so they are matched by path.
    pub bind_mounts: HashSet<PathBuf>,
    pub mounts: &'a [MountInfo],
}

impl<'a> Boundary<'a> {
    pub fn of(top: &Path, mounts: &'a [MountInfo]) -> std::io::Result<Self> {
        let dev = fs::metadata(top)?.dev();
        let bind_mounts = platform::bind_mounts_below(mounts, top, &dev_string(dev))
            .into_iter()
            .collect();
        Ok(Boundary {
            dev,
            bind_mounts,
            mounts,
        })
    }

    pub(super) fn is_mount(&self, md: &fs::Metadata, path: &Path) -> bool {
        md.dev() != self.dev || self.bind_mounts.contains(path)
    }

    pub(super) fn mount_meta(&self, path: &Path) -> MountMeta {
        let fs_type = self
            .mounts
            .iter()
            .rev()
            .find(|m| m.mount_point == path)
            .map(|m| m.fs_type.clone())
            .unwrap_or_default();
        let stats = if statvfs_safe(&fs_type) {
            platform::statvfs(path).ok()
        } else {
            None
        };
        MountMeta { fs_type, stats }
    }
}

/// Limits a walk of one retained directory to the folded children whose name hash falls
/// in `[lo, hi)`. Children that are retained nodes in the snapshot are skipped by name.
pub struct Scope {
    pub lo: u32,
    pub hi: u32,
    pub skip: HashSet<Vec<u8>>,
}

impl Scope {
    pub(super) fn admits(&self, name: &[u8]) -> bool {
        (self.lo..self.hi).contains(&slot_of(name)) && !self.skip.contains(name)
    }
}

pub struct Walk<'a> {
    pub boundary: &'a Boundary<'a>,
    pub threshold: u64,
    pub scope: Option<&'a Scope>,
    /// Entries seen so far, for progress reporting.
    pub seen: &'a AtomicU64,
}

impl Walk<'_> {
    pub(super) fn admits(&self, name: &OsStr) -> bool {
        self.scope.is_none_or(|s| s.admits(name.as_bytes()))
    }
}

/// What a walk found below `top`.
#[derive(Default)]
pub struct Walked {
    /// Retained entries; for a whole-directory walk this includes `top` itself.
    pub nodes: Vec<Node>,
    /// Range walks: the buckets that re-tile exactly the scanned range.
    pub buckets: Vec<Bucket>,
    /// Range walks: the walked directory, re-read. A directory's own size follows its
    /// entries, so a range refresh must update it as well.
    pub parent: Option<Node>,
    /// Range walks: `top` could not be listed, nothing in `buckets` is meaningful.
    pub unreadable: bool,
    /// Entries seen by the second pass.
    pub visited: u64,
}
