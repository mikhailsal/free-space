//! In-memory snapshot state: how a rescanned part is merged back so totals stay exact,
//! and the invariants a snapshot read from disk must satisfy before a refresh trusts it.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::buckets::{find, slot_of, validate};
use super::model::{MAX_NODES, Model, Node, RootMeta, Stats, invariant, large};
use super::walk::{Scope, Walked};
use crate::error::{Error, Result};
use crate::scan::Kind;

fn depth(node: &Node) -> usize {
    node.path
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .filter(|b| **b == b'/')
        .count()
}

/// Index of the retained parent of every node; `None` for scan roots.
///
/// A scan root that is a mount point inside another root shares its path with the mount
/// point entry the outer tree keeps there. Only the root has children, so the path stands
/// for the directory.
fn parents(nodes: &[Node]) -> Result<Vec<Option<usize>>> {
    let mut index: HashMap<&Path, usize> = HashMap::with_capacity(nodes.len());
    for (i, n) in nodes.iter().enumerate() {
        if n.is_dir() || !index.contains_key(n.path.as_path()) {
            index.insert(n.path.as_path(), i);
        }
    }
    nodes
        .iter()
        .map(|n| match n.parent() {
            None => Ok(None),
            Some(parent) => index
                .get(parent)
                .copied()
                .map(Some)
                .ok_or_else(|| invariant("retained node without a retained parent")),
        })
        .collect()
}

/// Totals of every node, derived bottom-up from own bytes, buckets and retained children.
fn derived(nodes: &[Node], parent: &[Option<usize>]) -> Vec<Stats> {
    let mut totals: Vec<Stats> = nodes
        .iter()
        .map(|n| {
            let mut total = n.own();
            total.add(&n.rest());
            total
        })
        .collect();
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_cached_key(|&i| Reverse(depth(&nodes[i])));
    for i in order {
        if let Some(j) = parent[i] {
            let child = totals[i].clone();
            totals[j].add(&child);
        }
    }
    totals
}

pub fn rebuild(nodes: &mut [Node]) -> Result<()> {
    let parent = parents(nodes)?;
    let totals = derived(nodes, &parent);
    for (node, total) in nodes.iter_mut().zip(totals) {
        node.total = total;
    }
    Ok(())
}

fn inconsistent(path: &Path) -> Error {
    Error::generic(format!(
        "the totals stored for {} do not match what is below it",
        path.display()
    ))
    .with_fix("run a full scan: sudo freespace scan")
}

/// Check the totals a snapshot stored against the ones its parts add up to, then adopt the
/// latter (they also carry the hardlink count, which rows do not store).
pub fn settle(nodes: &mut [Node]) -> Result<()> {
    let parent = parents(nodes)?;
    let totals = derived(nodes, &parent);
    for (node, total) in nodes.iter().zip(&totals) {
        let stored = &node.total;
        let same = stored.allocated == total.allocated
            && stored.apparent == total.apparent
            && stored.entries == total.entries
            && stored.errors == total.errors
            && stored.min_scanned_at == total.min_scanned_at
            && stored.newest_mtime.unwrap_or(0) == total.newest_mtime.unwrap_or(0);
        if !same {
            return Err(inconsistent(&node.path));
        }
    }
    for (node, total) in nodes.iter_mut().zip(totals) {
        node.total = total;
    }
    Ok(())
}

/// The retained nodes form one tree per root, and every other node sits directly below
/// a retained directory. A directory tiles all slots; nothing else has buckets.
pub fn verify(model: &Model) -> Result<()> {
    let root_paths: HashSet<&Path> = model
        .nodes
        .iter()
        .filter(|n| n.root)
        .map(|n| n.path.as_path())
        .collect();
    let mut seen = HashSet::new();
    let mut roots = 0;
    for n in &model.nodes {
        // One node per path, but a root inside another root's mount point shares its path
        // with the mount point entry the outer tree keeps there (and nothing else may).
        if !seen.insert((&n.path, n.root)) {
            return Err(invariant("duplicate node path"));
        }
        if !n.root && n.kind != Kind::Mount && root_paths.contains(n.path.as_path()) {
            return Err(invariant("entry on the path of a scan root"));
        }
        if n.is_dir() {
            validate(&n.buckets)?;
        } else if !n.buckets.is_empty() {
            return Err(invariant("non-directory node has buckets"));
        }
        roots += usize::from(n.root);
    }
    if roots != model.roots.len() {
        return Err(invariant("snapshot roots do not match its root nodes"));
    }
    let parent = parents(&model.nodes)?;
    for (n, p) in model.nodes.iter().zip(parent) {
        if let Some(p) = p
            && !model.nodes[p].is_dir()
        {
            return Err(invariant("node is not below a directory"));
        }
        if n.root && !n.is_dir() {
            return Err(invariant("snapshot root must be a directory"));
        }
    }
    Ok(())
}

pub fn threshold_for(roots: &[RootMeta], path: &Path) -> Result<u64> {
    roots
        .iter()
        .filter(|r| path.starts_with(&r.path))
        .max_by_key(|r| r.path.as_os_str().len())
        .map(|r| r.threshold)
        .ok_or_else(|| invariant("entry outside every scan root"))
}

fn check_cap(nodes: &[Node]) -> Result<()> {
    if nodes.len() > MAX_NODES {
        return Err(Error::usage(format!(
            "more than {MAX_NODES} entries are large enough to keep; raise the folding threshold (--coarse-ppm, coarse_ppm)"
        )));
    }
    Ok(())
}

/// Fold a stats block into the bucket of `parent` that covers `slot`.
fn absorb(parent: &mut Node, slot: u32, stats: &Stats) -> Result<()> {
    let index =
        find(&parent.buckets, slot).ok_or_else(|| invariant("slot not covered by buckets"))?;
    parent.buckets[index].stats.add(stats);
    Ok(())
}

fn slot_in(node: &Node) -> Result<u32> {
    node.name()
        .map(slot_of)
        .ok_or_else(|| invariant("node has no name"))
}

/// The retained directory at `path`: for the path of a nested scan root, the root and not
/// the mount point entry of the outer tree.
fn position(nodes: &[Node], path: &Path) -> Result<usize> {
    nodes
        .iter()
        .position(|n| n.is_dir() && n.path == path)
        .ok_or_else(|| invariant("node missing"))
}

/// The scan root whose tree holds `node`: the deepest root at or above its path. An entry
/// on the very path of a root that is not that root is the mount point the outer tree
/// shows there, so it belongs to the root outside.
fn owner<'a>(roots: &'a [RootMeta], node: &Node) -> Option<&'a Path> {
    roots
        .iter()
        .filter(|r| node.path.starts_with(&r.path) && (node.root || node.path != r.path))
        .max_by_key(|r| r.path.as_os_str().len())
        .map(|r| r.path.as_path())
}

/// Remove what the tree of root `owned_by` holds at or below `top`. A root nested below
/// `top` keeps its own tree.
fn drop_below(model: &mut Model, top: &Path, owned_by: &Path) {
    let Model { roots, nodes, .. } = model;
    nodes.retain(|n| !(n.path.starts_with(top) && owner(roots, n) == Some(owned_by)));
}

/// A scan root nested in another one was a mount point when it was scanned. If a fresh walk
/// finds an ordinary entry on its path, its filesystem is no longer mounted there, and the
/// snapshot cannot hold both that entry and the root.
fn check_mounted(roots: &[RootMeta], fresh: &[Node]) -> Result<()> {
    let unmounted = fresh
        .iter()
        .filter(|n| !n.root && n.kind != Kind::Mount)
        .find_map(|n| roots.iter().find(|r| r.path == n.path));
    match unmounted {
        None => Ok(()),
        Some(root) => Err(Error::usage(format!(
            "{} is no longer a mount point, so the part of the snapshot that holds it cannot be refreshed; run a full scan",
            root.path.display()
        ))),
    }
}

/// Replace everything below scan root `root` by a fresh walk of it.
pub fn replace_root(model: &mut Model, root: &Path, fresh: Vec<Node>) -> Result<()> {
    check_mounted(&model.roots, &fresh)?;
    drop_below(model, root, root);
    model.nodes.extend(fresh);
    check_cap(&model.nodes)?;
    rebuild(&mut model.nodes)
}

/// Merge the rescan of retained directory `boundary` into the snapshot.
pub fn update(model: &mut Model, boundary: &Path, mut fresh: Vec<Node>) -> Result<()> {
    let prior = model.nodes[position(&model.nodes, boundary)?].clone();
    let owned_by = owner(&model.roots, &prior)
        .ok_or_else(|| invariant("entry outside every scan root"))?
        .to_path_buf();
    let threshold = threshold_for(&model.roots, boundary)?;
    let has_descendants = fresh.len() > 1;
    let top = fresh
        .iter_mut()
        .find(|n| n.path == boundary)
        .ok_or_else(|| invariant("replacement root missing"))?;
    top.root = prior.root;
    let stays = prior.root || top.pinned() || has_descendants || large(&top.total, threshold);
    let folded = top.total.clone();
    check_mounted(&model.roots, &fresh)?;
    drop_below(model, boundary, &owned_by);
    if stays {
        model.nodes.extend(fresh);
    } else {
        let parent = prior.parent().ok_or_else(|| invariant("parent missing"))?;
        let at = position(&model.nodes, parent)?;
        absorb(&mut model.nodes[at], slot_in(&prior)?, &folded)?;
    }
    check_cap(&model.nodes)?;
    rebuild(&mut model.nodes)?;
    prune(model)
}

/// Merge the rescan of one bucket range of `parent`: its buckets are replaced and any
/// child that became large enough joins the snapshot as a retained node.
pub fn update_range(model: &mut Model, parent: &Path, scope: &Scope, fresh: Walked) -> Result<()> {
    let at = position(&model.nodes, parent)?;
    let node = &mut model.nodes[at];
    let index = node
        .buckets
        .iter()
        .position(|b| b.lo == scope.lo && b.hi == scope.hi)
        .ok_or_else(|| invariant("range is not a bucket of its parent"))?;
    node.buckets.splice(index..=index, fresh.buckets);
    validate(&node.buckets)?;
    if let Some(top) = &fresh.parent {
        node.own_allocated = top.own_allocated;
        node.own_apparent = top.own_apparent;
        node.own_errors = top.own_errors;
        node.scanned_at = top.scanned_at;
        node.mtime = top.mtime;
        node.nlink = top.nlink;
    }
    model.nodes.extend(fresh.nodes);
    check_cap(&model.nodes)?;
    rebuild(&mut model.nodes)?;
    prune(model)
}

/// Fold retained nodes that no longer qualify into their parent's buckets, deepest first.
/// A node that still has retained children stays, whatever its size.
pub fn prune(model: &mut Model) -> Result<()> {
    let Model { roots, nodes, .. } = model;
    let parent = parents(nodes)?;
    let mut children = vec![0usize; nodes.len()];
    for p in parent.iter().flatten() {
        children[*p] += 1;
    }
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_cached_key(|&i| Reverse(depth(&nodes[i])));
    let mut removed = vec![false; nodes.len()];
    for i in order {
        let Some(p) = parent[i] else {
            continue;
        };
        let threshold = threshold_for(roots, &nodes[i].path)?;
        if children[i] > 0 || nodes[i].pinned() || large(&nodes[i].total, threshold) {
            continue;
        }
        let (slot, total) = (slot_in(&nodes[i])?, nodes[i].total.clone());
        absorb(&mut nodes[p], slot, &total)?;
        children[p] -= 1;
        removed[i] = true;
    }
    let mut flags = removed.into_iter();
    nodes.retain(|_| !flags.next().unwrap_or(false));
    rebuild(nodes)
}

#[cfg(test)]
mod tests;
