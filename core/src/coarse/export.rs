//! The model as v1 snapshot rows. Retained nodes become ordinary entries; every non-empty
//! bucket becomes a pseudo entry named `[small items]` that carries the totals of what it
//! folded, so `ls`, `top`, `find`, `status`, `diff` and the GUI work on a coarse snapshot
//! without knowing it is one.

use std::borrow::Cow;
use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::model::{Bucket, Model, Node, invariant};
use super::store::{BucketRow, RootRow, Side};
use crate::error::Result;
use crate::scan::tree::{self, Finalized, Kind, MountRow, NO_PARENT, RootInfo, Tree, flags};

pub struct Built {
    pub tree: Tree,
    pub fin: Finalized,
    pub side: Side,
    /// Paths of the directories that could not be read, for the scan report.
    pub denied: Vec<String>,
}

enum Source<'a> {
    Node(&'a Node),
    Bucket { dir: &'a Node, index: usize },
}

struct Row<'a> {
    source: Source<'a>,
    name: Cow<'a, [u8]>,
    depth: usize,
    /// Preorder index of the parent row.
    parent: Option<usize>,
}

fn bucket_name(dir: &Node, bucket: &Bucket) -> Vec<u8> {
    if dir.buckets.len() == 1 {
        b"[small items]".to_vec()
    } else {
        format!("[small items {}-{}]", bucket.lo, bucket.hi - 1).into_bytes()
    }
}

type Item<'a> = (Source<'a>, Cow<'a, [u8]>);

/// The children of retained directory `dir`, sorted by name: retained nodes and the
/// non-empty buckets.
fn children<'a>(dir: &'a Node, kids: &HashMap<&Path, Vec<&'a Node>>) -> Vec<Item<'a>> {
    let mut out: Vec<Item<'a>> = Vec::new();
    for child in kids.get(dir.path.as_path()).into_iter().flatten() {
        let name = child.name().map(Cow::Borrowed).unwrap_or_default();
        out.push((Source::Node(child), name));
    }
    for (index, bucket) in dir.buckets.iter().enumerate() {
        if bucket.stats.entries > 0 {
            out.push((
                Source::Bucket { dir, index },
                Cow::Owned(bucket_name(dir, bucket)),
            ));
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// All rows in preorder: the roots in their configured order, each followed by its tree.
fn collect<'a>(model: &'a Model) -> Result<Vec<Row<'a>>> {
    let mut kids: HashMap<&Path, Vec<&Node>> = HashMap::new();
    for node in &model.nodes {
        if let Some(parent) = node.parent() {
            kids.entry(parent).or_default().push(node);
        }
    }
    // Roots only: a root inside another root's mount point shares its path with the mount
    // point entry the outer tree keeps there, and the tree to write is the root's.
    let roots: HashMap<&Path, &Node> = model
        .nodes
        .iter()
        .filter(|n| n.root)
        .map(|n| (n.path.as_path(), n))
        .collect();
    let mut rows: Vec<Row<'a>> = Vec::new();
    for root in &model.roots {
        let node = *roots
            .get(root.path.as_path())
            .ok_or_else(|| invariant("root node missing"))?;
        let name = Cow::Borrowed(root.path.as_os_str().as_bytes());
        preorder(node, name, &kids, &mut rows);
    }
    Ok(rows)
}

/// Append the rows of the tree below `root`, depth first, children in name order.
fn preorder<'a>(
    root: &'a Node,
    name: Cow<'a, [u8]>,
    kids: &HashMap<&Path, Vec<&'a Node>>,
    rows: &mut Vec<Row<'a>>,
) {
    let mut stack = vec![(Source::Node(root), name, 0usize, None)];
    while let Some((source, name, depth, parent)) = stack.pop() {
        let at = rows.len();
        let below = match &source {
            Source::Node(n) if n.is_dir() => children(n, kids),
            _ => Vec::new(),
        };
        rows.push(Row {
            source,
            name,
            depth,
            parent,
        });
        for (source, name) in below.into_iter().rev() {
            stack.push((source, name, depth + 1, Some(at)));
        }
    }
}

/// The last row inside the subtree of every row, by preorder id.
fn subtree_ends(rows: &[Row<'_>]) -> Vec<u32> {
    let mut end = vec![0u32; rows.len()];
    let mut open: Vec<usize> = Vec::new();
    for (at, row) in rows.iter().enumerate() {
        while let Some(&top) = open.last() {
            if rows[top].depth < row.depth {
                break;
            }
            end[top] = (at - 1) as u32;
            open.pop();
        }
        open.push(at);
    }
    for top in open {
        end[top] = (rows.len() - 1) as u32;
    }
    end
}

fn clamp(v: u64) -> u32 {
    v.min(u64::from(u32::MAX)) as u32
}

fn entry_of(node: &Node) -> tree::Node {
    let total = &node.total;
    tree::Node {
        allocated: total.allocated,
        apparent: total.apparent,
        mtime: node.mtime,
        scanned_at: node.scanned_at,
        min_scanned_at: total.min_scanned_at.unwrap_or(node.scanned_at),
        newest_mtime: total.newest_mtime.unwrap_or(0),
        nlink: node.nlink,
        errors: clamp(total.errors),
        items: clamp(total.entries.saturating_sub(1)),
        kind: node.kind.code(),
        flags: node.flags,
        ..tree::Node::default()
    }
}

fn entry_of_bucket(dir: &Node, bucket: &Bucket) -> tree::Node {
    let s = &bucket.stats;
    let scanned = s.min_scanned_at.unwrap_or(dir.scanned_at);
    tree::Node {
        allocated: s.allocated,
        apparent: s.apparent,
        mtime: s.newest_mtime.unwrap_or(0),
        scanned_at: scanned,
        min_scanned_at: scanned,
        newest_mtime: s.newest_mtime.unwrap_or(0),
        nlink: 1,
        errors: clamp(s.errors),
        items: clamp(s.entries),
        kind: Kind::Other.code(),
        flags: flags::BUCKET,
        ..tree::Node::default()
    }
}

/// Raw ids: roots are `0..roots` as the writer requires, everything else follows.
fn raw_ids(rows: &[Row<'_>], roots: usize) -> (Vec<u32>, Vec<u32>) {
    let mut order = Vec::with_capacity(rows.len());
    let mut rank = vec![0u32; rows.len()];
    let (mut next_root, mut next_other) = (0u32, roots as u32);
    for (at, row) in rows.iter().enumerate() {
        let raw = if row.depth == 0 {
            next_root += 1;
            next_root - 1
        } else {
            next_other += 1;
            next_other - 1
        };
        order.push(raw);
        rank[raw as usize] = at as u32;
    }
    (order, rank)
}

fn full_path(row: &Row<'_>, scratch: &mut Vec<u8>) {
    scratch.clear();
    match &row.source {
        Source::Node(n) => scratch.extend_from_slice(n.path.as_os_str().as_bytes()),
        Source::Bucket { dir, .. } => {
            scratch.extend_from_slice(dir.path.as_os_str().as_bytes());
            tree::push_component(scratch, &row.name);
        }
    }
}

fn row_entry(row: &Row<'_>, fin: &Finalized, tree: &mut Tree) -> tree::Node {
    let mut entry = match &row.source {
        Source::Node(n) => entry_of(n),
        Source::Bucket { dir, index } => entry_of_bucket(dir, &dir.buckets[*index]),
    };
    let (off, len) = tree.push_name(&row.name);
    entry.name_off = off;
    entry.name_len = len;
    entry.parent = row.parent.map_or(NO_PARENT, |p| fin.order[p]);
    entry
}

/// Fill the tree and the per-row data of `fin`; returns the unreadable directories.
fn fill(rows: &[Row<'_>], tree: &mut Tree, fin: &mut Finalized) -> Vec<String> {
    let mut denied = Vec::new();
    let mut scratch = Vec::new();
    for (at, row) in rows.iter().enumerate() {
        let raw = fin.order[at];
        let entry = row_entry(row, fin, tree);
        full_path(row, &mut scratch);
        fin.path_hash.push(tree::path_hash(&scratch));
        if let Source::Node(n) = &row.source {
            if n.flags & flags::DENIED != 0 {
                fin.denied.push(at as u32);
                denied.push(String::from_utf8_lossy(&scratch).into_owned());
            }
            if let Some(mount) = &n.mount {
                tree.mounts.push(MountRow {
                    raw,
                    path: n.path.clone(),
                    fs_type: mount.fs_type.clone(),
                    stats: mount.stats,
                });
            }
        }
        tree.nodes[raw as usize] = entry;
    }
    denied
}

/// Every bucket of every retained directory, the empty ones included, with the id of its
/// pseudo entry when it has one.
fn side_buckets(rows: &[Row<'_>]) -> Vec<BucketRow> {
    let mut pseudo: HashMap<(usize, usize), u32> = HashMap::new();
    for (at, row) in rows.iter().enumerate() {
        if let (Source::Bucket { index, .. }, Some(parent)) = (&row.source, row.parent) {
            pseudo.insert((parent, *index), at as u32);
        }
    }
    let mut out = Vec::new();
    for (at, row) in rows.iter().enumerate() {
        let Source::Node(n) = &row.source else {
            continue;
        };
        for (index, bucket) in n.buckets.iter().enumerate() {
            out.push(BucketRow {
                parent_id: at as u32,
                entry_id: pseudo.get(&(at, index)).copied(),
                bucket: bucket.clone(),
            });
        }
    }
    out
}

pub fn build(model: &Model, started_at: i64) -> Result<Built> {
    let rows = collect(model)?;
    let (order, rank) = raw_ids(&rows, model.roots.len());
    let mut tree = Tree {
        started_at,
        nodes: vec![tree::Node::default(); rows.len()],
        ..Tree::default()
    };
    let mut fin = Finalized {
        order,
        rank,
        subtree_end: subtree_ends(&rows),
        path_hash: Vec::with_capacity(rows.len()),
        ..Finalized::default()
    };
    let denied = fill(&rows, &mut tree, &mut fin);
    let mut side = Side {
        ppm: model.ppm,
        roots: Vec::new(),
        buckets: side_buckets(&rows),
    };
    for (i, root) in model.roots.iter().enumerate() {
        tree.roots.push(RootInfo {
            path: root.path.clone(),
            dev: 0,
            fs_type: root.fs_type.clone(),
            stats: root.stats,
        });
        side.roots.push(RootRow {
            entry_id: fin.rank[i],
            threshold: root.threshold,
            capacity: root.stats.total,
        });
    }
    Ok(Built {
        tree,
        fin,
        side,
        denied,
    })
}

#[cfg(test)]
mod tests;
