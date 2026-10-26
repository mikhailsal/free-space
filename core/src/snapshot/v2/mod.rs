//! Experimental, opt-in generation store. It never opens or migrates v1 snapshots.
//!
//! See `docs/designs/refresh-v2-migration.md` for supported scope and adoption gates.
//! Identity means exact raw path; traversal order and generation are independent.
//! History is retained indefinitely until the separate retention milestone lands.

mod reader;
mod schema;
pub use reader::{Change, GenerationPair, Reader};

use crate::scan::{self, Kind, Tree, WalkOptions};
use crate::{Error, ErrorCode, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use schema::COLUMNS;
use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub parent: Option<i64>,
    pub path: Vec<u8>,
    pub sort_key: Vec<u8>,
    pub kind: Kind,
    pub flags: u8,
    pub own_allocated: i64,
    pub own_apparent: i64,
    pub own_errors: i64,
    pub mtime: i64,
    pub scanned_at: i64,
    pub allocated: i64,
    pub apparent: i64,
    pub items: i64,
    pub errors: i64,
    pub min_scanned_at: i64,
    /// None is distinct from a real pre-epoch file timestamp.
    pub newest_mtime: Option<i64>,
}

impl Entry {
    fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            parent: r.get(1)?,
            path: r.get(2)?,
            sort_key: r.get(3)?,
            kind: Kind::from_code(r.get(4)?),
            flags: r.get(5)?,
            own_allocated: r.get(6)?,
            own_apparent: r.get(7)?,
            own_errors: r.get(8)?,
            mtime: r.get(9)?,
            scanned_at: r.get(10)?,
            allocated: r.get(11)?,
            apparent: r.get(12)?,
            items: r.get(13)?,
            errors: r.get(14)?,
            min_scanned_at: r.get(15)?,
            newest_mtime: r.get(16)?,
        })
    }
}

/// Logical work counters, not a claim about physical writes or rows visited.
#[derive(Debug)]
pub struct WriteReport {
    pub generation: i64,
    pub old_subtree_entries: usize,
    pub new_subtree_entries: usize,
    pub ancestors: usize,
    pub sql_changes: u64,
    pub walk: Duration,
    pub transaction: Duration,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Use a dedicated file. Refuses any pre-existing foreign SQLite database.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let app: i64 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if app == 0 && version == 0 {
            let count: i64 =
                conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
            if count != 0 {
                return Err(unsupported("refusing non-v2 database"));
            }
            conn.execute_batch("BEGIN IMMEDIATE")?;
            // Recheck after taking the writer lock; another creator may have won.
            let count: i64 =
                conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
            if count != 0 {
                return Err(unsupported("database was initialized concurrently; reopen"));
            }
            conn.execute_batch(schema::CREATE)?;
            conn.pragma_update(None, "application_id", schema::APP_ID)?;
            conn.pragma_update(None, "user_version", 2)?;
            conn.execute_batch("COMMIT")?;
        } else if app != schema::APP_ID || version != 2 {
            return Err(unsupported("refusing non-v2 database"));
        }
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=0;",
        )?;
        Ok(Self { conn })
    }

    /// Diagnostic observer for locality experiments. Counts SQLite VM instructions;
    /// enabling it adds per-instruction overhead, so measure latency separately.
    pub fn observe_vm_steps(
        &self,
        counter: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    ) -> Result<()> {
        if let Some(counter) = counter {
            self.conn.progress_handler(
                1,
                Some(move || {
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    false
                }),
            )?;
        } else {
            self.conn.progress_handler(0, None::<fn() -> bool>)?;
        }
        Ok(())
    }

    /// A full scan seeds/rotates the store. Existing path identities survive.
    pub fn scan(&mut self, root: &Path, workers: usize) -> Result<WriteReport> {
        let start = Instant::now();
        let root = std::fs::canonicalize(root).map_err(|e| Error::io("canonicalize root", e))?;
        let tree = walk(&root, workers)?;
        let root_dev = tree.roots[0].dev;
        let mut entries = stage(tree)?;
        let walk = start.elapsed();
        let start = Instant::now();
        let before = self.conn.total_changes();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = pair(&tx)?.map(|p| p.current);
        let generation = next_generation(current)?;
        let old_count: i64 = tx.query_row("SELECT count(*) FROM live", [], |r| r.get(0))?;
        tx.execute(
            "UPDATE versions SET dead=?1 WHERE dead IS NULL",
            [generation],
        )?;
        tx.execute("DELETE FROM live", [])?;
        assign_ids(&tx, &mut entries, None)?;
        for e in &entries {
            insert(&tx, e, generation)?;
        }
        tx.execute("INSERT INTO control VALUES(1,?1,?2,?3,?1,?4) ON CONFLICT(singleton) DO UPDATE SET current=excluded.current,prev=excluded.prev,root=excluded.root,full_scan=excluded.full_scan,root_dev=excluded.root_dev",
            params![generation,current,root.as_os_str().as_bytes(),root_dev as i64])?;
        tx.commit()?;
        Ok(WriteReport {
            generation,
            old_subtree_entries: old_count as usize,
            new_subtree_entries: entries.len(),
            ancestors: 0,
            sql_changes: self.conn.total_changes() - before,
            walk,
            transaction: start.elapsed(),
        })
    }

    /// Replaces an existing non-root directory, or removes it if absent on disk.
    /// All database work is confined to old/new subtree rows and ancestor paths.
    pub fn refresh(&mut self, path: &Path, workers: usize) -> Result<WriteReport> {
        let bytes = path.as_os_str().as_bytes();
        validate_path(bytes)?;
        // Lock before walking: competing writers cannot publish against a stale base.
        let before = self.conn.total_changes();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pair = pair(&tx)?.ok_or_else(|| unsupported("seed the store first"))?;
        let generation = next_generation(Some(pair.current))?;
        let target = live_path(&tx, bytes)?
            .ok_or_else(|| Error::usage("path is absent from current generation"))?;
        if target.parent.is_none() || target.kind != Kind::Dir {
            return Err(unsupported("refresh requires a non-root directory"));
        }
        let (root, root_dev): (Vec<u8>, i64) = tx.query_row(
            "SELECT root,root_dev FROM control WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let root = Path::new(std::ffi::OsStr::from_bytes(&root));
        if crate::platform::visible_mountinfo().iter().any(|m| {
            m.mount_point != root
                && m.mount_point.starts_with(root)
                && path.starts_with(&m.mount_point)
        }) {
            return Err(unsupported("v2 refresh cannot cross a mount boundary"));
        }
        // Reject symlinked ancestors, rather than following a path outside the root.
        let mut ancestor = target.parent;
        while let Some(id) = ancestor {
            let entry = live_id(&tx, id)?;
            let p = Path::new(std::ffi::OsStr::from_bytes(&entry.path));
            let meta =
                std::fs::symlink_metadata(p).map_err(|e| Error::io("refresh ancestor", e))?;
            if !meta.is_dir() || meta.dev() != root_dev as u64 {
                return Err(unsupported("refresh ancestor is no longer a directory"));
            }
            ancestor = entry.parent;
        }
        let start = Instant::now();
        let mut entries = match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() && m.dev() == root_dev as u64 => stage(walk(path, workers)?)?,
            Ok(_) => return Err(unsupported("refresh directory replaced by non-directory")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(Error::io("refresh path", e)),
        };
        if entries.first().is_some_and(|e| e.path != bytes) {
            return Err(unsupported("refresh path resolved to another location"));
        }
        let walk = start.elapsed();
        let start = Instant::now();
        let upper = prefix_end(&target.sort_key);
        let old_ids = {
            let mut q = tx.prepare("SELECT id FROM live WHERE sort_key>=?1 AND sort_key<?2")?;
            q.query_map(params![target.sort_key, upper], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for id in &old_ids {
            retire(&tx, *id, generation)?;
        }
        assign_ids(&tx, &mut entries, target.parent)?;
        for e in &entries {
            insert(&tx, e, generation)?;
        }
        let replacement = entries.first();
        let mut old_child = target;
        let mut new_child = replacement.cloned();
        let mut ancestors = 0;
        while let Some(id) = old_child.parent {
            let old_parent = live_id(&tx, id)?;
            let mut parent = old_parent.clone();
            parent.allocated = replace_sum(
                parent.allocated,
                old_child.allocated,
                new_child.as_ref().map_or(0, |e| e.allocated),
            )?;
            parent.apparent = replace_sum(
                parent.apparent,
                old_child.apparent,
                new_child.as_ref().map_or(0, |e| e.apparent),
            )?;
            parent.errors = replace_sum(
                parent.errors,
                old_child.errors,
                new_child.as_ref().map_or(0, |e| e.errors),
            )?;
            parent.items = replace_sum(
                parent.items,
                add(old_child.items, 1)?,
                new_child
                    .as_ref()
                    .map(|e| add(e.items, 1))
                    .transpose()?
                    .unwrap_or(0),
            )?;
            let min:Option<i64>=tx.query_row("SELECT min_scanned_at FROM live WHERE parent=?1 ORDER BY min_scanned_at LIMIT 1",[id],|r|r.get(0)).optional()?;
            let newest:Option<i64>=tx.query_row("SELECT newest_mtime FROM live WHERE parent=?1 AND newest_mtime IS NOT NULL ORDER BY newest_mtime DESC LIMIT 1",[id],|r|r.get(0)).optional()?;
            parent.min_scanned_at = parent.scanned_at.min(min.unwrap_or(parent.scanned_at));
            parent.newest_mtime = newest;
            retire(&tx, id, generation)?;
            insert(&tx, &parent, generation)?;
            old_child = old_parent;
            new_child = Some(parent);
            ancestors += 1;
        }
        let scanned_at = crate::platform::now_secs();
        tx.execute(
            "INSERT INTO refreshes VALUES(?1,?2,?3)",
            params![generation, bytes, scanned_at],
        )?;
        tx.execute(
            "UPDATE control SET current=?1 WHERE singleton=1",
            [generation],
        )?;
        tx.commit()?;
        Ok(WriteReport {
            generation,
            old_subtree_entries: old_ids.len(),
            new_subtree_entries: entries.len(),
            ancestors,
            sql_changes: self.conn.total_changes() - before,
            walk,
            transaction: start.elapsed(),
        })
    }
}

fn unsupported(message: &str) -> Error {
    Error::new(ErrorCode::NotSupported, message)
}
fn next_generation(current: Option<i64>) -> Result<i64> {
    current
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| Error::generic("generation overflow"))
}
fn add(a: i64, b: i64) -> Result<i64> {
    a.checked_add(b)
        .ok_or_else(|| Error::generic("aggregate overflow"))
}
fn replace_sum(total: i64, old: i64, new: i64) -> Result<i64> {
    let value = total
        .checked_sub(old)
        .filter(|v| *v >= 0)
        .ok_or_else(|| Error::generic("invalid aggregate delta"))?;
    add(value, new)
}
fn size(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::generic("size exceeds v2 integer range"))
}
fn walk(root: &Path, workers: usize) -> Result<Tree> {
    let mut opts = WalkOptions::new(vec![root.to_path_buf()]);
    opts.workers = workers;
    scan::walk(&opts).map_err(|e| Error::io("v2 walk", e))
}
fn validate_path(path: &[u8]) -> Result<()> {
    if !path.starts_with(b"/")
        || path.contains(&0)
        || (path != b"/"
            && path[1..]
                .split(|b| *b == b'/')
                .any(|c| c.is_empty() || c == b"." || c == b".."))
    {
        return Err(Error::usage(
            "v2 paths must be normalized absolute raw paths",
        ));
    }
    Ok(())
}
// Each component byte is encoded as big-endian (byte + 1), followed by 0,0.
// This preserves raw component order, with a terminator below every byte.
fn key(path: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(path.len() * 2 + 2);
    for component in path.split(|b| *b == b'/').filter(|c| !c.is_empty()) {
        for b in component {
            key.extend_from_slice(&(*b as u16 + 1).to_be_bytes());
        }
        key.extend_from_slice(&[0, 0]);
    }
    key
}
fn prefix_end(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    // Every non-root key ends in 0,0; no overflow or ambiguous '/' boundary.
    *end.last_mut().expect("non-root subtree key") += 1;
    end
}
fn stage(tree: Tree) -> Result<Vec<Entry>> {
    if tree.roots.len() != 1 || !tree.hardlinks.is_empty() || !tree.mounts.is_empty() {
        return Err(unsupported(
            "v2 experiment supports one root without hardlinks or mounts",
        ));
    }
    let mut out: Vec<Entry> = Vec::with_capacity(tree.nodes.len());
    let mut stack: Vec<(u32, Option<usize>)> = vec![(0u32, None)];
    while let Some((raw, parent)) = stack.pop() {
        let n = tree.nodes[raw as usize];
        if n.kind() == Kind::Mount || (n.kind() != Kind::Dir && n.nlink > 1) {
            return Err(unsupported("v2 hardlinks/mounts are not implemented"));
        }
        let path = if let Some(p) = parent {
            let mut path = out[p].path.clone();
            crate::scan::tree::push_component(&mut path, tree.name(&n));
            path
        } else {
            tree.name(&n).to_vec()
        };
        validate_path(&path)?;
        let id = out.len();
        out.push(Entry {
            id: id as i64,
            parent: parent.map(|p| p as i64),
            sort_key: key(&path),
            path,
            kind: n.kind(),
            flags: n.flags,
            own_allocated: size(n.allocated)?,
            own_apparent: size(n.apparent)?,
            own_errors: n.errors as i64,
            mtime: n.mtime,
            scanned_at: n.scanned_at,
            allocated: size(n.allocated)?,
            apparent: size(n.apparent)?,
            items: 0,
            errors: n.errors as i64,
            min_scanned_at: n.scanned_at,
            newest_mtime: (n.kind() == Kind::File).then_some(n.mtime.min(n.scanned_at)),
        });
        for child in (n.child_start..n.child_start + n.child_count).rev() {
            stack.push((child, Some(id)));
        }
    }
    for i in (0..out.len()).rev() {
        if let Some(p) = out[i].parent {
            let c = out[i].clone();
            let p = &mut out[p as usize];
            p.allocated = add(p.allocated, c.allocated)?;
            p.apparent = add(p.apparent, c.apparent)?;
            p.items = add(p.items, add(c.items, 1)?)?;
            p.errors = add(p.errors, c.errors)?;
            p.min_scanned_at = p.min_scanned_at.min(c.min_scanned_at);
            p.newest_mtime = p.newest_mtime.max(c.newest_mtime);
        }
    }
    Ok(out)
}
fn assign_ids(conn: &Connection, entries: &mut [Entry], parent: Option<i64>) -> Result<()> {
    let mut ids = HashMap::with_capacity(entries.len());
    for entry in entries {
        conn.execute(
            "INSERT INTO identities(path) VALUES(?1) ON CONFLICT(path) DO NOTHING",
            [&entry.path],
        )?;
        let id = conn.query_row(
            "SELECT id FROM identities WHERE path=?1",
            [&entry.path],
            |r| r.get(0),
        )?;
        ids.insert(entry.id, id);
        entry.parent = entry.parent.map(|p| ids[&p]).or(parent);
        entry.id = id;
    }
    Ok(())
}
fn live_path(conn: &Connection, path: &[u8]) -> Result<Option<Entry>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM live WHERE path=?1"),
            [path],
            Entry::row,
        )
        .optional()?)
}
fn live_id(conn: &Connection, id: i64) -> Result<Entry> {
    Ok(conn.query_row(
        &format!("SELECT {COLUMNS} FROM live WHERE id=?1"),
        [id],
        Entry::row,
    )?)
}
fn retire(conn: &Connection, id: i64, generation: i64) -> Result<()> {
    conn.execute(
        "UPDATE versions SET dead=?1 WHERE id=?2 AND dead IS NULL",
        params![generation, id],
    )?;
    conn.execute("DELETE FROM live WHERE id=?1", [id])?;
    Ok(())
}
fn insert(conn: &Connection, e: &Entry, generation: i64) -> Result<()> {
    conn.execute(&format!("INSERT INTO live({COLUMNS}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)"),
        params![e.id,e.parent,e.path,e.sort_key,e.kind.code(),e.flags,e.own_allocated,e.own_apparent,e.own_errors,e.mtime,e.scanned_at,e.allocated,e.apparent,e.items,e.errors,e.min_scanned_at,e.newest_mtime])?;
    conn.execute(&format!("INSERT INTO versions({COLUMNS},born,dead) SELECT {COLUMNS},?1,NULL FROM live WHERE id=?2"),params![generation,e.id])?;
    Ok(())
}
fn pair(conn: &Connection) -> Result<Option<GenerationPair>> {
    Ok(conn
        .query_row(
            "SELECT current,prev FROM control WHERE singleton=1",
            [],
            |r| {
                Ok(GenerationPair {
                    current: r.get(0)?,
                    prev: r.get(1)?,
                })
            },
        )
        .optional()?)
}
