//! Reconstruct the stored snapshot as the in-memory tree used by refresh.

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::platform::FsStats;
use crate::scan::{MountRow, NO_PARENT, Node, RootInfo, Tree, flags};
use crate::snapshot::reader::Snapshot;

/// Old hardlink row: (raw id, dev, ino, allocated, apparent, was_owner).
pub(super) type OldLink = (u32, u64, u64, u64, u64, bool);

pub struct Loaded {
    pub tree: Tree,
    /// snapshot id → raw id
    pub raw_of: Vec<u32>,
    pub links: Vec<OldLink>,
}

/// Load a published snapshot into a walker-shaped tree (contiguous, name-ordered
/// child ranges) with own values per node.
pub fn load_tree(snap: &Snapshot) -> Result<Loaded> {
    let conn = &snap.conn;
    let n = entry_count(conn)?;
    let (parent, child_count) = load_parents(conn, n)?;
    let (block_start, raw_of) = make_layout(&parent, &child_count);
    let mut tree = load_nodes(
        conn,
        snap.meta.scanned_at,
        n,
        &parent,
        &child_count,
        &block_start,
        &raw_of,
    )?;
    subtract_children(&mut tree, &parent, &raw_of);
    let links = load_links(conn, &raw_of, &mut tree)?;
    load_roots(conn, &mut tree)?;
    load_mounts(conn, &raw_of, &mut tree)?;
    Ok(Loaded {
        tree,
        raw_of,
        links,
    })
}

fn entry_count(conn: &rusqlite::Connection) -> Result<usize> {
    let n: i64 = conn.query_row("SELECT count(*) FROM entries", [], |r| r.get(0))?;
    Ok(n as usize)
}

fn load_parents(conn: &rusqlite::Connection, n: usize) -> Result<(Vec<u32>, Vec<u32>)> {
    let mut parent = Vec::with_capacity(n);
    let mut child_count = vec![0u32; n];
    let mut st = conn.prepare("SELECT id, parent_id FROM entries ORDER BY id")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        if id as usize != parent.len() {
            return Err(Error::generic("snapshot ids are not dense"));
        }
        match r.get::<_, Option<i64>>(1)? {
            Some(p) => {
                parent.push(p as u32);
                child_count[p as usize] += 1;
            }
            None => parent.push(NO_PARENT),
        }
    }
    Ok((parent, child_count))
}

fn make_layout(parent: &[u32], child_count: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let n = parent.len();
    let roots = parent.iter().filter(|&&p| p == NO_PARENT).count() as u32;
    let mut block_start = vec![0u32; n];
    let mut next = roots;
    for id in 0..n {
        if child_count[id] > 0 {
            block_start[id] = next;
            next += child_count[id];
        }
    }
    let mut fill = block_start.clone();
    let mut raw_of = vec![0u32; n];
    let mut root_idx = 0u32;
    for id in 0..n {
        let p = parent[id];
        raw_of[id] = if p == NO_PARENT {
            root_idx += 1;
            root_idx - 1
        } else {
            let raw = fill[p as usize];
            fill[p as usize] += 1;
            raw
        };
    }
    (block_start, raw_of)
}

fn load_nodes(
    conn: &rusqlite::Connection,
    started_at: i64,
    n: usize,
    parent: &[u32],
    child_count: &[u32],
    block_start: &[u32],
    raw_of: &[u32],
) -> Result<Tree> {
    let mut tree = Tree {
        started_at,
        ..Default::default()
    };
    tree.nodes.resize(n, Node::default());
    let mut st = conn.prepare("SELECT id, name, kind, flags, allocated, apparent, mtime, nlink, errors, scanned_at FROM entries ORDER BY id")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let id = r.get::<_, i64>(0)? as usize;
        let name: Vec<u8> = r.get(1)?;
        let (off, len) = tree.push_name(&name);
        let raw = raw_of[id] as usize;
        let p = parent[id];
        tree.nodes[raw] = Node {
            allocated: r.get::<_, i64>(4)? as u64,
            apparent: r.get::<_, i64>(5)? as u64,
            mtime: r.get(6)?,
            scanned_at: r.get(9)?,
            name_off: off,
            name_len: len,
            parent: if p == NO_PARENT {
                NO_PARENT
            } else {
                raw_of[p as usize]
            },
            child_start: if child_count[id] > 0 {
                block_start[id]
            } else {
                0
            },
            child_count: child_count[id],
            nlink: r.get::<_, i64>(7)? as u32,
            errors: r.get::<_, i64>(8)? as u32,
            kind: r.get::<_, i64>(2)? as u8,
            flags: r.get::<_, i64>(3)? as u8 & !flags::ALIAS,
            ..Default::default()
        };
    }
    Ok(tree)
}

fn subtract_children(tree: &mut Tree, parent: &[u32], raw_of: &[u32]) {
    for id in 0..parent.len() {
        let p = parent[id];
        if p == NO_PARENT {
            continue;
        }
        let c = tree.nodes[raw_of[id] as usize];
        let pn = &mut tree.nodes[raw_of[p as usize] as usize];
        pn.allocated = pn.allocated.saturating_sub(c.allocated);
        pn.apparent = pn.apparent.saturating_sub(c.apparent);
        pn.errors = pn.errors.saturating_sub(c.errors);
    }
}

fn load_links(
    conn: &rusqlite::Connection,
    raw_of: &[u32],
    tree: &mut Tree,
) -> Result<Vec<OldLink>> {
    let mut links = Vec::new();
    let mut st = conn.prepare("SELECT h.entry_id, h.dev, h.ino, h.allocated, h.apparent, (e.flags & 2) = 0 FROM hardlinks h JOIN entries e ON e.id = h.entry_id ORDER BY h.entry_id")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let raw = raw_of[r.get::<_, i64>(0)? as usize];
        let (dev, ino) = (r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)? as u64);
        let (a, ap) = (r.get::<_, i64>(3)? as u64, r.get::<_, i64>(4)? as u64);
        let owner: bool = r.get(5)?;
        let nd = &mut tree.nodes[raw as usize];
        nd.allocated = a;
        nd.apparent = ap;
        tree.hardlinks.push((raw, dev, ino));
        links.push((raw, dev, ino, a, ap, owner));
    }
    Ok(links)
}

fn load_roots(conn: &rusqlite::Connection, tree: &mut Tree) -> Result<()> {
    let mut st = conn.prepare("SELECT entry_id, path, fs_type, fs_used, fs_total, fs_avail, inodes_used FROM roots ORDER BY entry_id")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let path: Vec<u8> = r.get(1)?;
        let used = r.get::<_, i64>(3)? as u64;
        let total = r.get::<_, i64>(4)? as u64;
        let inodes_used: Option<i64> = r.get(6)?;
        tree.roots.push(RootInfo {
            path: PathBuf::from(std::ffi::OsStr::from_bytes(&path)),
            dev: 0,
            fs_type: r.get(2)?,
            stats: FsStats {
                total,
                used,
                free: total.saturating_sub(used),
                avail: r.get::<_, i64>(5)? as u64,
                inodes_total: inodes_used.unwrap_or(0) as u64,
                inodes_free: 0,
            },
        });
    }
    Ok(())
}

fn load_mounts(conn: &rusqlite::Connection, raw_of: &[u32], tree: &mut Tree) -> Result<()> {
    let mut st = conn.prepare("SELECT entry_id, path, fs_type, fs_used, fs_total FROM mounts")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let path: Vec<u8> = r.get(1)?;
        let used: Option<i64> = r.get(3)?;
        let total: Option<i64> = r.get(4)?;
        tree.mounts.push(MountRow {
            raw: raw_of[r.get::<_, i64>(0)? as usize],
            path: PathBuf::from(std::ffi::OsStr::from_bytes(&path)),
            fs_type: r.get(2)?,
            stats: match (used, total) {
                (Some(u), Some(t)) => Some(FsStats {
                    used: u as u64,
                    total: t as u64,
                    ..Default::default()
                }),
                _ => None,
            },
        });
    }
    Ok(())
}
