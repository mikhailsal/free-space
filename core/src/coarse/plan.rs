//! Which part of the tree a refresh must rescan: a whole retained directory, or only the
//! bucket of a retained directory that holds the target.

use std::collections::HashSet;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::buckets::{find, slot_of};
use super::model::{Node, Stats, invariant};
use super::walk::Scope;
use crate::error::Result;

/// One bucket of a retained directory: the folded children of `parent` whose name hashes
/// into `[lo, hi)`. `old` is what the bucket held before the refresh.
pub struct Slice {
    pub parent: PathBuf,
    pub lo: u32,
    pub hi: u32,
    pub old: Stats,
}

pub enum Plan {
    /// Rescan this retained directory as a whole.
    Retained(PathBuf),
    /// Rescan only the bucket that holds the target.
    Range(Slice),
}

impl Plan {
    /// The retained directory this plan rescans from. It has to exist on disk to be rescanned.
    pub fn region(&self) -> &Path {
        match self {
            Self::Retained(path) => path,
            Self::Range(slice) => &slice.parent,
        }
    }

    /// Entries the rescanned region held: what a refresh of it has to read.
    pub fn entries(&self, nodes: &[Node]) -> u64 {
        match self {
            Self::Retained(path) => nodes
                .iter()
                .find(|n| n.path == *path)
                .map_or(0, |n| n.total.entries),
            Self::Range(slice) => slice.old.entries,
        }
    }

    /// Hardlinked entries the rescanned region held. Their byte ownership depends on paths
    /// outside the region, so a region with links is never refreshed on its own.
    pub fn links(&self, nodes: &[Node]) -> u64 {
        match self {
            Self::Retained(path) => nodes
                .iter()
                .find(|n| n.path == *path)
                .map_or(0, |n| n.total.links),
            Self::Range(slice) => slice.old.links,
        }
    }
}

/// The deepest retained directory that contains `target` (or is it).
pub fn nearest<'a>(nodes: &'a [Node], target: &Path) -> Option<&'a Node> {
    nodes
        .iter()
        .filter(|n| n.is_dir() && target.starts_with(&n.path))
        .max_by_key(|n| n.path.as_os_str().len())
}

pub fn plan(nodes: &[Node], target: &Path) -> Result<Plan> {
    let near = nearest(nodes, target).ok_or_else(|| invariant("no retained ancestor"))?;
    if near.path == target {
        return Ok(Plan::Retained(near.path.clone()));
    }
    let below = target
        .strip_prefix(&near.path)
        .map_err(|_| invariant("target is not below its ancestor"))?;
    let name = below
        .components()
        .next()
        .ok_or_else(|| invariant("target has no component below its ancestor"))?
        .as_os_str()
        .as_bytes();
    let retained_child = nodes
        .iter()
        .any(|n| n.parent() == Some(near.path.as_path()) && n.name() == Some(name));
    if retained_child {
        // A retained child that is not a directory (or a mount): refresh its directory.
        return Ok(Plan::Retained(near.path.clone()));
    }
    let index = find(&near.buckets, slot_of(name))
        .ok_or_else(|| invariant("slot not covered by buckets"))?;
    let bucket = &near.buckets[index];
    Ok(Plan::Range(Slice {
        parent: near.path.clone(),
        lo: bucket.lo,
        hi: bucket.hi,
        old: bucket.stats.clone(),
    }))
}

/// The names a range rescan must leave alone: retained children are separate nodes.
pub fn scope(nodes: &[Node], parent: &Path, lo: u32, hi: u32) -> Scope {
    let skip: HashSet<Vec<u8>> = nodes
        .iter()
        .filter(|n| n.parent() == Some(parent))
        .filter_map(Node::name)
        .filter(|name| (lo..hi).contains(&slot_of(name)))
        .map(<[u8]>::to_vec)
        .collect();
    Scope { lo, hi, skip }
}
