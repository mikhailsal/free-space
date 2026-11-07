//! The rescans a refresh picks from: one bucket of a folder, a folder, or a whole root
//! (together with the roots that share its hardlinks), each merged back into the model.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use super::{Part, Rescanned};
use crate::coarse::job::group::{walk_group, walk_of};
use crate::coarse::merge;
use crate::coarse::model::{Model, RootMeta, invariant};
use crate::coarse::plan::{Plan, Slice, scope};
use crate::coarse::walk::{self, Boundary};
use crate::error::{Error, Result};
use crate::platform::{self, MountInfo};
use crate::scan::walk::{statvfs_safe, validate_root_nesting};

pub(super) struct Done {
    pub visited: u64,
    pub rescanned: Rescanned,
}

/// Rescan the part of the snapshot `plan` names, and what it drags in: a part that holds
/// hardlinks is rescanned as the root it belongs to.
pub(super) fn execute(
    model: &mut Model,
    plan: Plan,
    mounts: &[MountInfo],
    seen: &AtomicU64,
) -> Result<Done> {
    let root = model
        .root_of(plan.region())
        .map(|r| r.path.clone())
        .ok_or_else(|| invariant("region outside every scan root"))?;
    if plan.links(&model.nodes) > 0 {
        return rescan_root(model, &root, 0, mounts, seen);
    }
    match plan {
        Plan::Retained(path) if path == root => rescan_root(model, &root, 0, mounts, seen),
        Plan::Retained(path) => rescan_folder(model, &root, path, mounts, seen),
        Plan::Range(slice) => rescan_bucket(model, &root, &slice, mounts, seen),
    }
}

fn boundary<'a>(path: &Path, mounts: &'a [MountInfo]) -> Result<Boundary<'a>> {
    Boundary::of(path, mounts).map_err(|e| Error::io(format!("stat {}", path.display()), e))
}

/// Hardlinked entries the snapshot holds in the tree of `root`.
fn links_of(model: &Model, root: &Path) -> u64 {
    model
        .nodes
        .iter()
        .find(|n| n.root && n.path == root)
        .map_or(0, |n| n.total.links)
}

/// A scan root nested in another one was scanned as a mount point of its own filesystem, and
/// a snapshot never holds two roots of one filesystem inside each other. If the path of a
/// root inside `within` now lies on the filesystem of a root that holds it, that filesystem
/// has been unmounted: the rescan would see an ordinary folder there, and the snapshot would
/// hold it twice, once in the outer tree and once as the root. A full scan puts that right.
/// A root that cannot be examined is left to the walk.
fn check_nested(model: &Model, within: &Path) -> Result<()> {
    let paths: Vec<PathBuf> = model.roots.iter().map(|r| r.path.clone()).collect();
    for root in model.roots.iter().filter(|r| r.path.starts_with(within)) {
        let Ok(meta) = fs::metadata(&root.path) else {
            continue;
        };
        if let Err(e) = validate_root_nesting(&root.path, &paths, meta.dev())
            && e.kind() == ErrorKind::InvalidInput
        {
            return Err(Error::usage(format!(
                "{} is no longer a mount point of its own filesystem, so the part of the snapshot that holds it cannot be refreshed; run a full scan",
                root.path.display()
            )));
        }
    }
    Ok(())
}

/// The other scan roots on the filesystem of `root`: a hardlink can join an entry of one to
/// an entry of another, and the bytes of such an inode are carried by whichever path of
/// all of them sorts first. A root that is gone from disk cannot hold a link any more.
fn peers_of(model: &Model, root: &Path) -> Result<Vec<RootMeta>> {
    let dev = fs::metadata(root)
        .map_err(|e| Error::io(format!("stat {}", root.display()), e))?
        .dev();
    Ok(model
        .roots
        .iter()
        .filter(|r| r.path != root)
        .filter(|r| fs::metadata(&r.path).is_ok_and(|m| m.dev() == dev))
        .cloned()
        .collect())
}

/// Rescan scan root `root`. `spent` is what the attempts that led here already read.
///
/// The bytes of a hardlinked inode belong to the first path among all roots of its
/// filesystem, so a root with links is rescanned together with the roots beside it. A root
/// with none (before and after) has nothing to settle with them.
pub(super) fn rescan_root(
    model: &mut Model,
    root: &Path,
    spent: u64,
    mounts: &[MountInfo],
    seen: &AtomicU64,
) -> Result<Done> {
    check_nested(model, root)?;
    let peers = peers_of(model, root)?;
    if !peers.is_empty() && links_of(model, root) > 0 {
        return rescan_together(model, root, peers, spent, mounts, seen);
    }
    let threshold = merge::threshold_for(&model.roots, root)?;
    let boundary = boundary(root, mounts)?;
    let w = walk_of(&boundary, threshold, seen);
    let (walked, links) = walk::scan(root, true, &w)?;
    let visited = spent + walked.visited;
    if links && !peers.is_empty() {
        return rescan_together(model, root, peers, visited, mounts, seen);
    }
    merge::replace_root(model, root, walked.nodes)?;
    Ok(Done {
        visited,
        rescanned: Rescanned {
            path: root.to_path_buf(),
            part: Part::Root,
            with: Vec::new(),
        },
    })
}

/// Rescan `root` and `peers` with one table of hardlink owners.
fn rescan_together(
    model: &mut Model,
    root: &Path,
    peers: Vec<RootMeta>,
    spent: u64,
    mounts: &[MountInfo],
    seen: &AtomicU64,
) -> Result<Done> {
    let own = model
        .roots
        .iter()
        .find(|r| r.path == root)
        .cloned()
        .ok_or_else(|| invariant("path is not a scan root"))?;
    let group: Vec<RootMeta> = std::iter::once(own).chain(peers).collect();
    for meta in &group {
        check_nested(model, &meta.path)?;
    }
    let mut visited = spent;
    for (meta, found) in group.iter().zip(walk_group(&group, mounts, seen, None)?) {
        visited += found.visited;
        merge::replace_root(model, &meta.path, found.nodes)?;
    }
    Ok(Done {
        visited,
        rescanned: Rescanned {
            path: root.to_path_buf(),
            part: Part::Root,
            with: group.into_iter().skip(1).map(|r| r.path).collect(),
        },
    })
}

fn rescan_folder(
    model: &mut Model,
    root: &Path,
    path: PathBuf,
    mounts: &[MountInfo],
    seen: &AtomicU64,
) -> Result<Done> {
    check_nested(model, &path)?;
    let threshold = merge::threshold_for(&model.roots, &path)?;
    let boundary = boundary(&path, mounts)?;
    let w = walk_of(&boundary, threshold, seen);
    let (walked, links) = walk::scan(&path, false, &w)?;
    if links {
        return rescan_root(model, root, walked.visited, mounts, seen);
    }
    let visited = walked.visited;
    merge::update(model, &path, walked.nodes)?;
    Ok(Done {
        visited,
        rescanned: Rescanned {
            path,
            part: Part::Folder,
            with: Vec::new(),
        },
    })
}

fn rescan_bucket(
    model: &mut Model,
    root: &Path,
    slice: &Slice,
    mounts: &[MountInfo],
    seen: &AtomicU64,
) -> Result<Done> {
    let threshold = merge::threshold_for(&model.roots, &slice.parent)?;
    let boundary = boundary(&slice.parent, mounts)?;
    let range = scope(&model.nodes, &slice.parent, slice.lo, slice.hi);
    let mut w = walk_of(&boundary, threshold, seen);
    w.scope = Some(&range);
    let (walked, links) = walk::scan(&slice.parent, false, &w)?;
    if walked.unreadable {
        // the folder cannot be listed any more: it is kept as an unreadable folder
        return rescan_folder(model, root, slice.parent.clone(), mounts, seen);
    }
    if links {
        return rescan_root(model, root, walked.visited, mounts, seen);
    }
    let visited = walked.visited;
    merge::update_range(model, &slice.parent, &range, walked)?;
    Ok(Done {
        visited,
        rescanned: Rescanned {
            path: slice.parent.clone(),
            part: Part::Bucket,
            with: Vec::new(),
        },
    })
}

/// A refresh usually follows a cleanup: the free space shown "at scan" should move with it.
pub(super) fn update_fs_stats(model: &mut Model, target: &Path) {
    let Some(root) = model
        .roots
        .iter_mut()
        .filter(|r| target.starts_with(&r.path))
        .max_by_key(|r| r.path.as_os_str().len())
    else {
        return;
    };
    if statvfs_safe(&root.fs_type)
        && let Ok(stats) = platform::statvfs(&root.path)
    {
        root.stats = stats;
    }
}

#[cfg(test)]
mod tests;
