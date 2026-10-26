//! In-memory tree built by the walker (or loaded from a snapshot for `refresh`)
//! and the post-walk pipeline: DFS preorder, path hashes, hardlink attribution,
//! subtree aggregation. The same pipeline runs for `scan` and `refresh`.

use std::collections::HashMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::platform::FsStats;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Dir,
    Symlink,
    Other,
    /// Mount point under a root: shown, never traversed.
    Mount,
}

impl Kind {
    pub fn code(self) -> u8 {
        match self {
            Kind::File => 0,
            Kind::Dir => 1,
            Kind::Symlink => 2,
            Kind::Other => 3,
            Kind::Mount => 4,
        }
    }

    pub fn from_code(c: u8) -> Kind {
        match c {
            0 => Kind::File,
            1 => Kind::Dir,
            2 => Kind::Symlink,
            4 => Kind::Mount,
            _ => Kind::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Dir => "dir",
            Kind::Symlink => "symlink",
            Kind::Other => "other",
            Kind::Mount => "mount",
        }
    }
}

/// Bits of `entries.flags`.
pub mod flags {
    /// `opendir`/`readdir` of this directory failed: its contents are unknown (D13).
    pub const DENIED: u8 = 1;
    /// Hardlink whose inode is counted under another (lexicographically smaller) path.
    pub const ALIAS: u8 = 2;
    /// `lstat` of this entry failed; sizes are 0.
    pub const STAT_ERROR: u8 = 4;
    /// In-memory only: the entry was dropped by `refresh` (never written).
    pub const REMOVED: u8 = 0x80;
}

pub const NO_PARENT: u32 = u32::MAX;

/// One entry. Before [`Tree::finalize`] the size fields hold the entry's own
/// values; after it they hold subtree aggregates (own values for leaves).
#[derive(Debug, Clone, Copy, Default)]
pub struct Node {
    pub allocated: u64,
    pub apparent: u64,
    pub mtime: i64,
    pub scanned_at: i64,
    pub min_scanned_at: i64,
    pub newest_mtime: i64,
    pub name_off: u64,
    pub parent: u32,
    pub child_start: u32,
    pub child_count: u32,
    pub nlink: u32,
    pub errors: u32,
    pub items: u32,
    pub name_len: u16,
    pub kind: u8,
    pub flags: u8,
}

impl Node {
    pub fn kind(&self) -> Kind {
        Kind::from_code(self.kind)
    }
}

#[derive(Debug, Clone)]
pub struct RootInfo {
    pub path: PathBuf,
    pub dev: u64,
    pub fs_type: String,
    pub stats: FsStats,
}

#[derive(Debug, Clone)]
pub struct MountRow {
    pub raw: u32,
    pub path: PathBuf,
    pub fs_type: String,
    pub stats: Option<FsStats>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardlinkRow {
    /// Preorder id after finalize.
    pub entry_id: u32,
    pub dev: u64,
    pub ino: u64,
    pub allocated: u64,
    pub apparent: u64,
}

/// Subtree aggregate. One `merge` is shared by `scan` and `refresh` (T33).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubtreeAgg {
    pub allocated: u64,
    pub apparent: u64,
    pub items: u64,
    pub errors: u64,
    pub min_scanned_at: i64,
    /// Max over FILE entries of `min(mtime, scanned_at)` (D3/D4/D5); identity `i64::MIN`.
    pub newest_mtime: i64,
}

impl SubtreeAgg {
    pub fn leaf(
        kind: Kind,
        allocated: u64,
        apparent: u64,
        mtime: i64,
        scanned_at: i64,
        errors: u64,
    ) -> Self {
        SubtreeAgg {
            allocated,
            apparent,
            items: 0,
            errors,
            min_scanned_at: scanned_at,
            newest_mtime: if kind == Kind::File {
                mtime.min(scanned_at)
            } else {
                i64::MIN
            },
        }
    }

    pub fn merge(&mut self, child: &SubtreeAgg) {
        self.allocated += child.allocated;
        self.apparent += child.apparent;
        self.items += 1 + child.items;
        self.errors += child.errors;
        self.min_scanned_at = self.min_scanned_at.min(child.min_scanned_at);
        self.newest_mtime = self.newest_mtime.max(child.newest_mtime);
    }

    /// Aggregate view of a node (fields hold aggregates once the fold reached it).
    pub fn of(n: &Node) -> Self {
        SubtreeAgg {
            allocated: n.allocated,
            apparent: n.apparent,
            items: n.items as u64,
            errors: n.errors as u64,
            min_scanned_at: n.min_scanned_at,
            newest_mtime: n.newest_mtime,
        }
    }

    pub fn store(&self, n: &mut Node) {
        n.allocated = self.allocated;
        n.apparent = self.apparent;
        n.items = self.items.min(u32::MAX as u64) as u32;
        n.errors = self.errors.min(u32::MAX as u64) as u32;
        n.min_scanned_at = self.min_scanned_at;
        n.newest_mtime = self.newest_mtime;
    }

    /// Value written to `entries.newest_mtime`: 0 means "no files inside" (unknown).
    pub fn stored_newest_mtime(&self) -> i64 {
        if self.newest_mtime == i64::MIN {
            0
        } else {
            self.newest_mtime
        }
    }
}

/// Raw tree: ids are assigned by the walker (roots are `0..roots.len()`, the
/// children of a directory are contiguous and sorted by name).
#[derive(Debug, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub names: Vec<u8>,
    pub roots: Vec<RootInfo>,
    /// (raw id, dev, ino) of non-directory entries with `nlink > 1`.
    pub hardlinks: Vec<(u32, u64, u64)>,
    pub mounts: Vec<MountRow>,
    pub started_at: i64,
}

/// Result of [`Tree::finalize`]: the preorder permutation and per-entry data
/// the writer needs. Node size fields are aggregates from here on.
#[derive(Debug, Default)]
pub struct Finalized {
    /// new (preorder) id → raw id
    pub order: Vec<u32>,
    /// raw id → new id
    pub rank: Vec<u32>,
    /// by new id
    pub path_hash: Vec<i64>,
    pub hardlinks: Vec<HardlinkRow>,
    /// new ids with the DENIED flag, ascending
    pub denied: Vec<u32>,
}

/// Hardlink attribution overrides used by `refresh` (T19, OV-11). Keys are (dev, ino).
#[derive(Debug, Default, Clone)]
pub struct LinkPolicy {
    /// Keep this entry as the owner (previous attribution, owner outside the refreshed subtree).
    pub forced_raw: HashMap<(u64, u64), u32>,
    /// Keep the owner at this path if it still exists (owner inside the refreshed subtree).
    pub forced_path: HashMap<(u64, u64), Vec<u8>>,
    /// Count nobody: an unseen link outside the subtree already carries the size.
    pub zeroed: std::collections::HashSet<(u64, u64)>,
}

pub fn path_hash(path: &[u8]) -> i64 {
    xxhash_rust::xxh3::xxh3_64(path) as i64
}

/// Append `name` to `path` with exactly one separator.
pub fn push_component(path: &mut Vec<u8>, name: &[u8]) {
    if !path.ends_with(b"/") {
        path.push(b'/');
    }
    path.extend_from_slice(name);
}

impl Tree {
    pub fn name(&self, n: &Node) -> &[u8] {
        &self.names[n.name_off as usize..n.name_off as usize + n.name_len as usize]
    }

    pub fn push_name(&mut self, name: &[u8]) -> (u64, u16) {
        let off = self.names.len() as u64;
        let len = name.len().min(u16::MAX as usize);
        self.names.extend_from_slice(&name[..len]);
        (off, len as u16)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn children(&self, n: &Node) -> std::ops::Range<u32> {
        n.child_start..n.child_start + n.child_count
    }

    /// DFS preorder (children in name order), path hashes, hardlink attribution
    /// (smallest full path keeps the size, others become ALIAS with 0), then the
    /// reverse-preorder fold that turns own values into subtree aggregates.
    pub fn finalize(&mut self) -> Finalized {
        self.finalize_with(&LinkPolicy::default())
    }

    pub fn finalize_with(&mut self, policy: &LinkPolicy) -> Finalized {
        let walk = self.build_preorder(policy);
        let hardlinks = self.attribute_hardlinks(policy, &walk);
        self.aggregate_subtrees(&walk.order);
        let denied = self.denied_rows(&walk.rank);

        Finalized {
            order: walk.order,
            rank: walk.rank,
            path_hash: walk.path_hash,
            hardlinks,
            denied,
        }
    }

    fn build_preorder(&self, policy: &LinkPolicy) -> Preorder {
        let n = self.nodes.len();
        let mut order = Vec::with_capacity(n);
        let mut rank = vec![u32::MAX; n];
        let mut path_hash_v = Vec::with_capacity(n);

        let link_of: HashMap<u32, (u64, u64)> = self
            .hardlinks
            .iter()
            .map(|&(raw, dev, ino)| (raw, (dev, ino)))
            .collect();
        // (dev, ino) → (winner raw id, its full path)
        let mut best: HashMap<(u64, u64), (u32, Vec<u8>)> = HashMap::new();
        let mut forced_found: HashMap<(u64, u64), u32> = HashMap::new();

        let mut path: Vec<u8> = Vec::with_capacity(4096);
        let mut depth_len: Vec<usize> = Vec::new();
        let mut stack: Vec<(u32, u32)> = Vec::new(); // (raw, depth)
        for r in (0..self.roots.len() as u32).rev() {
            stack.push((r, 0));
        }
        while let Some((raw, depth)) = stack.pop() {
            let node = self.nodes[raw as usize];
            let depth = depth as usize;
            if depth == 0 {
                path.clear();
                path.extend_from_slice(self.name(&node));
            } else {
                path.truncate(depth_len[depth - 1]);
                push_component(&mut path, self.name(&node));
            }
            depth_len.truncate(depth);
            depth_len.push(path.len());

            rank[raw as usize] = order.len() as u32;
            order.push(raw);
            path_hash_v.push(path_hash(&path));

            if let Some(&key) = link_of.get(&raw) {
                record_hardlink_candidate(key, raw, &path, policy, &mut best, &mut forced_found);
            }
            self.push_live_children(&node, depth as u32 + 1, &mut stack);
        }

        Preorder {
            order,
            rank,
            path_hash: path_hash_v,
            best,
            forced_found,
        }
    }

    fn attribute_hardlinks(&mut self, policy: &LinkPolicy, walk: &Preorder) -> Vec<HardlinkRow> {
        let winner = |key: &(u64, u64)| -> Option<u32> {
            if policy.zeroed.contains(key) {
                return None;
            }
            if let Some(&r) = policy.forced_raw.get(key)
                && walk.rank[r as usize] != u32::MAX
            {
                return Some(r);
            }
            if let Some(&r) = walk.forced_found.get(key) {
                return Some(r);
            }
            walk.best.get(key).map(|b| b.0)
        };
        let mut hardlinks = Vec::with_capacity(self.hardlinks.len());
        for &(raw, dev, ino) in &self.hardlinks {
            if walk.rank[raw as usize] == u32::MAX {
                continue; // dropped by refresh
            }
            let owner = winner(&(dev, ino));
            let node = &mut self.nodes[raw as usize];
            hardlinks.push(HardlinkRow {
                entry_id: walk.rank[raw as usize],
                dev,
                ino,
                allocated: node.allocated,
                apparent: node.apparent,
            });
            if owner != Some(raw) {
                node.flags |= flags::ALIAS;
                node.allocated = 0;
                node.apparent = 0;
            }
        }
        hardlinks.sort_by_key(|h| h.entry_id);
        hardlinks
    }

    fn aggregate_subtrees(&mut self, order: &[u32]) {
        // Fold in place: leaf aggregates first, then children into parents in
        // reverse preorder (every descendant has a larger preorder id).
        for nd in self.nodes.iter_mut() {
            SubtreeAgg::leaf(
                nd.kind(),
                nd.allocated,
                nd.apparent,
                nd.mtime,
                nd.scanned_at,
                nd.errors as u64,
            )
            .store(nd);
        }
        for &raw in order.iter().rev() {
            let child = self.nodes[raw as usize];
            if child.parent != NO_PARENT {
                let p = &mut self.nodes[child.parent as usize];
                let mut agg = SubtreeAgg::of(p);
                agg.merge(&SubtreeAgg::of(&child));
                agg.store(p);
            }
        }
    }

    fn denied_rows(&mut self, rank: &[u32]) -> Vec<u32> {
        let mut denied = Vec::new();
        for (raw, nd) in self.nodes.iter_mut().enumerate() {
            if nd.newest_mtime == i64::MIN {
                nd.newest_mtime = 0;
            }
            if nd.flags & flags::DENIED != 0 && rank[raw] != u32::MAX {
                denied.push(rank[raw]);
            }
        }
        denied.sort_unstable();

        denied
    }

    fn push_live_children(&self, node: &Node, depth: u32, stack: &mut Vec<(u32, u32)>) {
        for child in self.children(node).rev() {
            if self.nodes[child as usize].flags & flags::REMOVED == 0 {
                stack.push((child, depth));
            }
        }
    }
}

struct Preorder {
    order: Vec<u32>,
    rank: Vec<u32>,
    path_hash: Vec<i64>,
    best: HashMap<(u64, u64), (u32, Vec<u8>)>,
    forced_found: HashMap<(u64, u64), u32>,
}

fn record_hardlink_candidate(
    key: (u64, u64),
    raw: u32,
    path: &Vec<u8>,
    policy: &LinkPolicy,
    best: &mut HashMap<(u64, u64), (u32, Vec<u8>)>,
    forced_found: &mut HashMap<(u64, u64), u32>,
) {
    if policy
        .forced_path
        .get(&key)
        .is_some_and(|fp| fp.as_slice() == path)
    {
        forced_found.insert(key, raw);
    }
    match best.get_mut(&key) {
        Some((winner, winner_path)) if path.as_slice() < winner_path.as_slice() => {
            *winner = raw;
            winner_path.clear();
            winner_path.extend_from_slice(path);
        }
        Some(_) => {}
        None => {
            best.insert(key, (raw, path.clone()));
        }
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
