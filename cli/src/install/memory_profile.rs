//! Installation-time heuristic; no directory walk or trial scan is performed.
use std::{
    collections::HashSet,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use clap::ValueEnum;
use freespace_core::{
    config::{Backend, Config},
    platform::{
        self, MountInfo,
        memory::{MemoryAvailability, memory_availability},
    },
    scan::walk::statvfs_safe,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Choice {
    Auto,
    True,
    False,
}

pub(super) struct Decision {
    pub(super) low_memory: bool,
    pub(super) reason: String,
}

pub(super) fn resolve(
    choice: Option<Choice>,
    configured: Option<bool>,
    config: &Config,
) -> Decision {
    choose(choice, configured, || {
        automatic(
            config.backend,
            memory_availability(),
            estimate_entries(&config.roots),
        )
    })
}

fn choose(
    choice: Option<Choice>,
    configured: Option<bool>,
    auto: impl FnOnce() -> Decision,
) -> Decision {
    match (choice, configured) {
        (Some(Choice::Auto), _) | (None, None) => auto(),
        (Some(Choice::True), _) => selected(true, "explicit: --low-memory=true"),
        (Some(Choice::False), _) => selected(false, "explicit: --low-memory=false"),
        (None, Some(value)) => selected(
            value,
            &format!("preserved: low_memory={value} from configuration"),
        ),
    }
}

fn selected(low_memory: bool, reason: &str) -> Decision {
    Decision {
        low_memory,
        reason: reason.into(),
    }
}

fn automatic(backend: Backend, memory: MemoryAvailability, entries: Option<u64>) -> Decision {
    let Some(available) = memory.available_bytes else {
        return selected(
            true,
            "auto: available memory is unknown; using disk staging",
        );
    };
    let Some(entries) = entries else {
        return selected(
            true,
            "auto: safe filesystem inode estimate is unknown; using disk staging",
        );
    };
    let per_entry = match backend {
        Backend::V1 => 384,
        Backend::V2 => 1024,
    };
    let Some(estimate) = entries
        .checked_mul(per_entry)
        .and_then(|n| n.checked_add(64 * 1024 * 1024))
    else {
        return selected(true, "auto: memory estimate overflowed; using disk staging");
    };
    let budget = available / 4;
    let low_memory = estimate > budget;
    Decision {
        low_memory,
        reason: format!(
            "auto: conservative in-memory estimate {} quarter-available memory budget",
            if low_memory { "exceeds" } else { "fits" }
        ),
    }
}

#[derive(Clone, Copy)]
struct Inodes {
    dev: u64,
    total: u64,
    free: u64,
}

fn estimate_entries(roots: &[PathBuf]) -> Option<u64> {
    if roots.is_empty() {
        return None;
    }
    let mounts = platform::mountinfo();
    let rows = roots
        .iter()
        .map(|root| root_inodes(root, &mounts))
        .collect::<Option<Vec<_>>>()?;
    sum_entries(&rows)
}

fn root_inodes(root: &Path, mounts: &[MountInfo]) -> Option<Inodes> {
    // Check before touching a root, then recheck its canonical mount for symlinks.
    let mount = platform::mount_for(mounts, root)?;
    if !statvfs_safe(&mount.fs_type) {
        return None;
    }
    let root = std::fs::canonicalize(root).ok()?;
    let mount = platform::mount_for(mounts, &root)?;
    if !statvfs_safe(&mount.fs_type) {
        return None;
    }
    let metadata = std::fs::metadata(&root).ok()?;
    if !metadata.is_dir() {
        return None;
    }
    let stats = platform::statvfs(&root).ok()?;
    Some(Inodes {
        dev: metadata.dev(),
        total: stats.inodes_total,
        free: stats.inodes_free,
    })
}

fn sum_entries(rows: &[Inodes]) -> Option<u64> {
    let mut devices = HashSet::new();
    let mut entries = 0_u64;
    for row in rows {
        if row.total == 0 || row.free > row.total {
            return None;
        }
        if devices.insert(row.dev) {
            entries = entries.checked_add(row.total.checked_sub(row.free)?)?;
        }
    }
    Some(entries)
}

#[cfg(test)]
mod tests;
