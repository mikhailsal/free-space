//! A published coarse snapshot read back into the model a refresh works on.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use rusqlite::Connection;

use super::merge::{settle, verify};
use super::model::{Model, MountMeta, Node, RootMeta, Stats, unreadable};
use super::store::{self, Side};
use crate::error::{Error, Result};
use crate::platform::FsStats;
use crate::scan::{Kind, flags};
use crate::snapshot::reader::Snapshot;

/// What a row's stored aggregates give up when its children's are taken away.
#[derive(Default)]
struct Below {
    allocated: u64,
    apparent: u64,
    errors: u64,
}

struct Loaded {
    nodes: Vec<Node>,
    /// Row id → index in `nodes` (`None` for pseudo entries).
    index: Vec<Option<usize>>,
}

pub fn load(snap: &Snapshot) -> Result<Model> {
    let conn = &snap.conn;
    let side = store::read(conn)?;
    let mut loaded = load_entries(conn)?;
    attach_buckets(&mut loaded, &side)?;
    attach_mounts(conn, &mut loaded)?;
    let roots = load_roots(conn, &side, &loaded)?;
    settle(&mut loaded.nodes)?;
    let model = Model {
        roots,
        nodes: loaded.nodes,
        ppm: side.ppm,
    };
    verify(&model)?;
    Ok(model)
}

fn lookup(loaded: &Loaded, id: i64) -> Result<usize> {
    usize::try_from(id)
        .ok()
        .and_then(|i| loaded.index.get(i).copied().flatten())
        .ok_or_else(|| unreadable("a bucket or mount refers to a missing entry"))
}

struct Row {
    id: i64,
    parent: Option<i64>,
    name: Vec<u8>,
    kind: u8,
    flags: u8,
    nlink: u32,
    mtime: i64,
    scanned_at: i64,
    total: Stats,
}

fn read_rows(conn: &Connection) -> Result<Vec<Row>> {
    let mut st = conn.prepare(
        "SELECT id, parent_id, name, kind, flags, allocated, apparent, mtime, nlink, items, errors,
                scanned_at, min_scanned_at, newest_mtime FROM entries ORDER BY id",
    )?;
    let rows = st
        .query_map([], |r| {
            let newest: i64 = r.get(13)?;
            Ok(Row {
                id: r.get(0)?,
                parent: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                flags: r.get(4)?,
                nlink: r.get::<_, i64>(8)?.clamp(0, i64::from(u32::MAX)) as u32,
                mtime: r.get(7)?,
                scanned_at: r.get(11)?,
                total: Stats {
                    allocated: r.get::<_, i64>(5)?.max(0) as u64,
                    apparent: r.get::<_, i64>(6)?.max(0) as u64,
                    entries: r.get::<_, i64>(9)?.max(0) as u64 + 1,
                    errors: r.get::<_, i64>(10)?.max(0) as u64,
                    links: 0,
                    min_scanned_at: Some(r.get(12)?),
                    newest_mtime: (newest != 0).then_some(newest),
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn load_entries(conn: &Connection) -> Result<Loaded> {
    let rows = read_rows(conn)?;
    let mut loaded = Loaded {
        nodes: Vec::new(),
        index: vec![None; rows.len()],
    };
    let mut below: Vec<Below> = Vec::new();
    let mut paths: Vec<Option<PathBuf>> = vec![None; rows.len()];
    for (expected, row) in rows.iter().enumerate() {
        if row.id != expected as i64 {
            return Err(unreadable("entry ids are not contiguous"));
        }
        let parent = row.parent.map(|p| usize::try_from(p).unwrap_or(usize::MAX));
        if let Some(p) = parent {
            let slot = below_of(&loaded, p)?;
            if let Some(slot) = slot {
                below[slot].allocated += row.total.allocated;
                below[slot].apparent += row.total.apparent;
                below[slot].errors += row.total.errors;
            }
        }
        if row.flags & flags::BUCKET != 0 {
            continue;
        }
        let path = match parent {
            None => PathBuf::from(OsStr::from_bytes(&row.name)),
            Some(p) => paths
                .get(p)
                .and_then(Option::as_ref)
                .ok_or_else(|| unreadable("an entry precedes its parent"))?
                .join(OsStr::from_bytes(&row.name)),
        };
        paths[expected] = Some(path.clone());
        loaded.index[expected] = Some(loaded.nodes.len());
        loaded.nodes.push(Node {
            path,
            root: parent.is_none(),
            kind: Kind::from_code(row.kind),
            flags: row.flags,
            nlink: row.nlink,
            mtime: row.mtime,
            scanned_at: row.scanned_at,
            own_allocated: 0,
            own_apparent: 0,
            own_errors: 0,
            total: row.total.clone(),
            buckets: Vec::new(),
            mount: None,
        });
        below.push(Below::default());
    }
    own_values(&mut loaded.nodes, &below)?;
    Ok(loaded)
}

fn below_of(loaded: &Loaded, parent: usize) -> Result<Option<usize>> {
    match loaded.index.get(parent) {
        Some(slot) => Ok(*slot),
        None => Err(unreadable("an entry refers to a missing parent")),
    }
}

/// A node's own bytes are what its aggregate holds beyond its children (buckets included).
fn own_values(nodes: &mut [Node], below: &[Below]) -> Result<()> {
    for (node, below) in nodes.iter_mut().zip(below) {
        let take = |total: u64, part: u64| {
            total
                .checked_sub(part)
                .ok_or_else(|| unreadable("an entry is smaller than what it contains"))
        };
        node.own_allocated = take(node.total.allocated, below.allocated)?;
        node.own_apparent = take(node.total.apparent, below.apparent)?;
        node.own_errors = take(node.total.errors, below.errors)?;
    }
    Ok(())
}

fn attach_buckets(loaded: &mut Loaded, side: &Side) -> Result<()> {
    for row in &side.buckets {
        let at = lookup(loaded, i64::from(row.parent_id))?;
        loaded.nodes[at].buckets.push(row.bucket.clone());
    }
    Ok(())
}

fn attach_mounts(conn: &Connection, loaded: &mut Loaded) -> Result<()> {
    let mut st = conn.prepare("SELECT entry_id, fs_type, fs_used, fs_total FROM mounts")?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, fs_type, used, total) in rows {
        let at = lookup(loaded, id)?;
        let stats = match (used, total) {
            (Some(used), Some(total)) => Some(FsStats {
                used: used.max(0) as u64,
                total: total.max(0) as u64,
                ..FsStats::default()
            }),
            _ => None,
        };
        loaded.nodes[at].mount = Some(Box::new(MountMeta { fs_type, stats }));
    }
    Ok(())
}

fn load_roots(conn: &Connection, side: &Side, loaded: &Loaded) -> Result<Vec<RootMeta>> {
    let mut st = conn.prepare(
        "SELECT entry_id, path, fs_type, fs_used, fs_total, fs_avail, inodes_used FROM roots ORDER BY entry_id",
    )?;
    let rows = st
        .query_map([], |r| {
            let inodes_used: Option<i64> = r.get(6)?;
            let (used, total) = (
                r.get::<_, i64>(3)?.max(0) as u64,
                r.get::<_, i64>(4)?.max(0) as u64,
            );
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, String>(2)?,
                FsStats {
                    total,
                    used,
                    free: total.saturating_sub(used),
                    avail: r.get::<_, i64>(5)?.max(0) as u64,
                    // `inodes_used()` reads back what was written
                    inodes_total: inodes_used.unwrap_or(0).max(0) as u64,
                    inodes_free: 0,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut roots = Vec::new();
    for (id, path, fs_type, stats) in rows {
        let threshold = side
            .roots
            .iter()
            .find(|r| i64::from(r.entry_id) == id)
            .map(|r| r.threshold)
            .ok_or_else(|| unreadable("a root has no folding threshold"))?;
        let at = lookup(loaded, id)?;
        if !loaded.nodes[at].root {
            return Err(unreadable("a root entry has a parent"));
        }
        roots.push(RootMeta {
            path: PathBuf::from(OsStr::from_bytes(&path)),
            fs_type,
            stats,
            threshold,
        });
    }
    if roots.is_empty() {
        return Err(Error::generic("the coarse snapshot has no roots"));
    }
    Ok(roots)
}

#[cfg(test)]
mod tests;
