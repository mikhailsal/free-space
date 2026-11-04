//! In-memory snapshot state: which part of the tree a refresh must rescan, and how a
//! rescanned part is merged back so totals stay exact.
use crate::buckets::{find, slot_of, validate};
use crate::model::{MAX_RECORDS, Node, R, Stats};
use crate::walk::{ResultTree, Scope, keep};
use std::collections::{HashMap, HashSet};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

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

pub fn nearest(nodes: &[Node], target: &Path) -> R<PathBuf> {
    nodes
        .iter()
        .filter(|n| n.kind == 1 && target.starts_with(&n.path))
        .max_by_key(|n| n.path.components().count())
        .map(|n| n.path.clone())
        .ok_or_else(|| "no retained ancestor".into())
}
pub fn plan(nodes: &[Node], target: &Path) -> R<Plan> {
    let near = nearest(nodes, target)?;
    if near == target {
        return Ok(Plan::Retained(near));
    }
    let below = target.strip_prefix(&near)?;
    let name = below
        .components()
        .next()
        .ok_or("target has no component below its ancestor")?
        .as_os_str()
        .as_bytes();
    let parent = nodes
        .iter()
        .find(|n| n.path == near)
        .ok_or("nearest node missing")?;
    let retained_child = nodes
        .iter()
        .any(|n| n.parent.as_deref() == Some(near.as_path()) && n.name() == Some(name));
    if retained_child {
        // A retained non-directory child: refresh its directory.
        return Ok(Plan::Retained(near));
    }
    let index = find(&parent.buckets, slot_of(name)).ok_or("slot not covered by buckets")?;
    let bucket = &parent.buckets[index];
    Ok(Plan::Range(Slice {
        parent: near,
        lo: bucket.lo,
        hi: bucket.hi,
        old: bucket.stats.clone(),
    }))
}
/// The names a range rescan must leave alone: retained children are separate nodes.
pub fn scope(nodes: &[Node], parent: &Path, lo: u32, hi: u32) -> Scope {
    let skip: HashSet<Vec<u8>> = nodes
        .iter()
        .filter(|n| n.parent.as_deref() == Some(parent))
        .filter_map(|n| n.name())
        .filter(|name| (lo..hi).contains(&slot_of(name)))
        .map(<[u8]>::to_vec)
        .collect();
    Scope { lo, hi, skip }
}

fn depth(node: &Node) -> std::cmp::Reverse<usize> {
    std::cmp::Reverse(node.path.components().count())
}
fn lookup(nodes: &[Node]) -> HashMap<PathBuf, usize> {
    nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.path.clone(), i))
        .collect()
}
/// Totals of every node, derived bottom-up from own bytes, buckets and retained children.
fn derived(nodes: &[Node]) -> R<Vec<Stats>> {
    let index = lookup(nodes);
    let mut totals = Vec::with_capacity(nodes.len());
    for n in nodes {
        let mut t = n.own();
        t.add(&n.rest()?)?;
        totals.push(t);
    }
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|i| depth(&nodes[*i]));
    for i in order {
        if let Some(parent) = &nodes[i].parent {
            let j = *index.get(parent).ok_or("retained node missing parent")?;
            let child = totals[i].clone();
            totals[j].add(&child)?;
        }
    }
    Ok(totals)
}
pub fn rebuild(nodes: &mut [Node]) -> R<()> {
    let totals = derived(nodes)?;
    for (n, t) in nodes.iter_mut().zip(totals) {
        n.total = t;
    }
    Ok(())
}
/// The retained nodes form one tree: a single root, and every other node sits directly
/// below a retained directory.
fn check_structure(nodes: &[Node]) -> R<()> {
    let index = lookup(nodes);
    let mut roots = 0;
    for n in nodes {
        if n.kind > 2 {
            return Err("unknown node kind".into());
        }
        let Some(parent) = &n.parent else {
            roots += 1;
            continue;
        };
        let owner = index
            .get(parent)
            .map(|i| &nodes[*i])
            .ok_or("retained node missing parent")?;
        if owner.kind != 1 || n.path.parent() != Some(parent.as_path()) {
            return Err("node is not a child of its parent directory".into());
        }
    }
    if roots != 1 {
        return Err("snapshot must have exactly one root".into());
    }
    Ok(())
}
/// A snapshot read from disk must satisfy the invariants refresh relies on.
pub fn verify(nodes: &[Node]) -> R<()> {
    let mut seen = HashSet::new();
    for n in nodes {
        if !seen.insert(&n.path) {
            return Err("duplicate node path".into());
        }
        if n.kind == 1 {
            validate(&n.buckets)?;
        } else if !n.buckets.is_empty() {
            return Err("non-directory node has buckets".into());
        }
    }
    check_structure(nodes)?;
    let totals = derived(nodes)?;
    if nodes.iter().zip(&totals).any(|(n, t)| n.total != *t) {
        return Err("snapshot totals do not match their components".into());
    }
    Ok(())
}
/// Fold a stats block into the bucket of `parent` that covers `slot`.
fn absorb(parent: &mut Node, slot: u32, stats: &Stats) -> R<()> {
    let index = find(&parent.buckets, slot).ok_or("slot not covered by buckets")?;
    parent.buckets[index].stats.add(stats)
}
fn slot_in(node: &Node) -> R<u32> {
    node.name()
        .map(slot_of)
        .ok_or_else(|| "node has no name".into())
}
fn check_cap(nodes: &[Node]) -> R<()> {
    if nodes.len() > MAX_RECORDS {
        return Err("retained record cap exceeded".into());
    }
    Ok(())
}

/// Merge the rescan of retained directory `boundary` into the snapshot.
pub fn update(old: &mut Vec<Node>, boundary: &Path, mut fresh: Vec<Node>, threshold: u64) -> R<()> {
    let prior = old
        .iter()
        .find(|n| n.path == boundary)
        .ok_or("boundary missing")?
        .clone();
    let top = fresh
        .iter_mut()
        .find(|n| n.path == boundary)
        .ok_or("replacement root missing")?;
    top.parent = prior.parent.clone();
    let fold = !keep(&top.total, threshold) && prior.parent.is_some();
    let folded = top.total.clone();
    old.retain(|n| !n.path.starts_with(boundary));
    if fold {
        if fresh.len() != 1 {
            return Err("small subtree contains retained descendant".into());
        }
        let slot = slot_in(&prior)?;
        let parent = old
            .iter_mut()
            .find(|n| Some(&n.path) == prior.parent.as_ref())
            .ok_or("parent missing")?;
        absorb(parent, slot, &folded)?;
    } else {
        old.extend(fresh);
    }
    check_cap(old)?;
    rebuild(old)?;
    prune(old, threshold)
}
/// Merge the rescan of one bucket range of `parent`: its buckets are replaced and any
/// child that became large enough joins the snapshot as a retained node.
pub fn update_range(
    old: &mut Vec<Node>,
    parent: &Path,
    scope: &Scope,
    fresh: ResultTree,
    threshold: u64,
) -> R<()> {
    let node = old
        .iter_mut()
        .find(|n| n.path == parent)
        .ok_or("range parent missing")?;
    let at = node
        .buckets
        .iter()
        .position(|b| b.lo == scope.lo && b.hi == scope.hi)
        .ok_or("range is not a bucket of its parent")?;
    node.buckets.splice(at..=at, fresh.buckets);
    validate(&node.buckets)?;
    if let Some(top) = &fresh.parent {
        node.own_allocated = top.own_allocated;
        node.own_apparent = top.own_apparent;
        node.own_errors = top.own_errors;
        node.scanned_at = top.scanned_at;
    }
    old.extend(fresh.nodes);
    check_cap(old)?;
    rebuild(old)?;
    prune(old, threshold)
}
/// Fold retained nodes that no longer qualify into their parent's buckets, deepest first.
pub fn prune(nodes: &mut Vec<Node>, threshold: u64) -> R<()> {
    let mut candidates: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.parent.is_some() && !keep(&n.total, threshold))
        .map(|(i, _)| i)
        .collect();
    candidates.sort_by_key(|i| depth(&nodes[*i]));
    let index = lookup(nodes);
    let mut children: HashMap<PathBuf, usize> = HashMap::new();
    for node in nodes.iter() {
        if let Some(parent) = &node.parent {
            *children.entry(parent.clone()).or_default() += 1;
        }
    }
    let mut removed = HashSet::new();
    for i in candidates {
        if children.get(&nodes[i].path).copied().unwrap_or(0) != 0 {
            return Err("small ancestor contains retained child".into());
        }
        let parent = nodes[i].parent.as_ref().ok_or("parent missing")?.clone();
        let j = *index.get(&parent).ok_or("parent missing")?;
        let (total, slot) = (nodes[i].total.clone(), slot_in(&nodes[i])?);
        absorb(&mut nodes[j], slot, &total)?;
        *children.get_mut(&parent).ok_or("child count missing")? -= 1;
        removed.insert(nodes[i].path.clone());
    }
    nodes.retain(|n| !removed.contains(&n.path));
    rebuild(nodes)
}
/// Hardlinked entries anywhere in the snapshot (the root total includes every region).
pub fn links(nodes: &[Node]) -> u64 {
    nodes
        .iter()
        .find(|n| n.parent.is_none())
        .map_or(0, |n| n.total.links)
}
