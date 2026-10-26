//! Experimental, opt-in generation store. It never opens or migrates v1 snapshots.
//!
//! See `docs/designs/refresh-v2-migration.md` for supported scope and adoption gates.
//! Identity means exact raw path; traversal order and generation are independent.
//! History is retained indefinitely until the separate retention milestone lands.

mod aggregate;
mod hardlinks;
mod metadata;
mod reader;
mod staging;
pub use metadata::{Mount, Root};
mod schema;
pub use reader::{Change, GenerationPair, Reader};

use crate::scan::{self, Kind, Tree, WalkOptions};
use crate::{Error, ErrorCode, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use schema::COLUMNS;
use staging::{fold, stage};
use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Configured root; distinguishes nested-root entries from mount placeholders.
    pub scope: Vec<u8>,
    pub nlink: i64,
    pub dev: Option<i64>,
    pub ino: Option<i64>,
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
            scope: r.get(17)?,
            nlink: r.get(18)?,
            dev: r.get(19)?,
            ino: r.get(20)?,
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
    pub touched_inode_groups: usize,
    pub known_inode_members: usize,
    pub sql_changes: u64,
    pub walk: Duration,
    pub transaction: Duration,
}

pub struct Store {
    conn: Connection,
}

struct PreparedRefresh {
    base_generation: i64,
    target: Entry,
    root: Root,
    staged: staging::Staged,
    started: i64,
    walk: Duration,
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
            conn.pragma_update(None, "user_version", schema::REVISION)?;
            conn.execute_batch("COMMIT")?;
        } else if app != schema::APP_ID || version != schema::REVISION {
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

    /// A full scan seeds/rotates the store. Existing root/path identities survive.
    pub fn scan(&mut self, root: &Path, workers: usize) -> Result<WriteReport> {
        self.scan_roots(&[root.to_path_buf()], true, workers)
    }
    /// Root normalization and overlap validation are shared with the v1 walker.
    pub fn scan_roots(
        &mut self,
        roots: &[PathBuf],
        one_file_system: bool,
        workers: usize,
    ) -> Result<WriteReport> {
        if roots.is_empty() {
            return Err(Error::usage("at least one scan root is required"));
        }
        let start = Instant::now();
        let mut opts = WalkOptions::new(roots.to_vec());
        opts.workers = workers;
        opts.one_file_system = one_file_system;
        let tree = scan::walk(&opts).map_err(|e| Error::io("v2 walk", e))?;
        self.seed_tree(tree, one_file_system, start)
    }
    fn seed_tree(
        &mut self,
        tree: Tree,
        one_file_system: bool,
        start: Instant,
    ) -> Result<WriteReport> {
        let mut staged = stage(tree, None)?;
        hardlinks::scan(&mut staged.entries);
        fold(&mut staged.entries)?;
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
        tx.execute(
            "UPDATE root_versions SET dead=?1 WHERE dead IS NULL",
            [generation],
        )?;
        tx.execute(
            "UPDATE mount_versions SET dead=?1 WHERE dead IS NULL",
            [generation],
        )?;
        tx.execute_batch("DELETE FROM live; DELETE FROM live_roots; DELETE FROM live_mounts;")?;
        let ids = assign_ids(&tx, &mut staged.entries, None)?;
        for e in &staged.entries {
            insert(&tx, e, generation)?;
        }
        for mut root in staged.roots {
            root.entry_id = ids[&root.entry_id];
            metadata::insert_root(&tx, &root, generation)?;
        }
        for mut mount in staged.mounts {
            mount.entry_id = ids[&mount.entry_id];
            metadata::insert_mount(&tx, &mount, generation)?;
        }
        tx.execute("INSERT INTO control VALUES(1,?1,?2,?1,?3) ON CONFLICT(singleton) DO UPDATE SET current=excluded.current,prev=excluded.prev,full_scan=excluded.full_scan,one_file_system=excluded.one_file_system",params![generation,current,one_file_system])?;
        tx.commit()?;
        Ok(WriteReport {
            generation,
            old_subtree_entries: old_count as usize,
            new_subtree_entries: staged.entries.len(),
            ancestors: 0,
            touched_inode_groups: 0,
            known_inode_members: 0,
            sql_changes: self.conn.total_changes() - before,
            walk,
            transaction: start.elapsed(),
        })
    }

    /// Replaces an existing directory, including a configured root. Missing
    /// non-root directories are removed; root deletion still requires a full scan.
    pub fn refresh(&mut self, path: &Path, workers: usize) -> Result<WriteReport> {
        let prepared = self.prepare_refresh(path, workers)?;
        self.publish_refresh(prepared)
    }
    fn prepare_refresh(&mut self, path: &Path, workers: usize) -> Result<PreparedRefresh> {
        let bytes = path.as_os_str().as_bytes();
        validate_path(bytes)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let pair = pair(&tx)?.ok_or_else(|| unsupported("seed the store first"))?;
        let target = live_path(&tx, bytes)?
            .ok_or_else(|| Error::usage("path is absent from current generation"))?;
        if target.kind != Kind::Dir {
            return Err(Error::usage(
                "refresh requires a directory within its root filesystem",
            ));
        }
        let mut root = metadata::live_root(&tx, &target.scope)?;
        let one_file_system: bool = tx.query_row(
            "SELECT one_file_system FROM control WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        tx.commit()?;
        let target_meta = match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => Some(m),
            Ok(_) => {
                return Err(Error::usage(
                    "refresh directory replaced by non-directory; refresh its parent",
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::io("refresh path", e)),
        };
        crate::refresh::mounts::check(
            path,
            target_meta.as_ref().map(|m| m.dev()),
            &[root.info()],
            one_file_system,
            &String::from_utf8_lossy(bytes),
        )?;
        if target_meta.is_none() && target.parent.is_none() {
            return Err(Error::usage("root is missing; run a full scan"));
        }
        let started = crate::platform::now_secs();
        let start = Instant::now();
        let staged = if target_meta.is_some() {
            let mut opts = WalkOptions::new(vec![path.to_path_buf()]);
            opts.workers = workers;
            opts.one_file_system = one_file_system;
            stage(
                scan::walk(&opts).map_err(|e| Error::io("v2 refresh walk", e))?,
                Some(&target.scope),
            )?
        } else {
            staging::Staged {
                entries: Vec::new(),
                roots: Vec::new(),
                mounts: Vec::new(),
            }
        };
        if staged.entries.first().is_some_and(|e| e.path != bytes) {
            return Err(Error::usage("refresh path resolves to another location"));
        }
        // Preserve the original root type and refresh its df numbers as v1 does.
        if scan::walk::statvfs_safe(&root.fs_type)
            && let Ok(stats) = crate::platform::statvfs(&root.info().path)
        {
            root.stats = stats;
        }
        if target.parent.is_none()
            && let Some(m) = target_meta
        {
            root.dev = m.dev();
        }
        Ok(PreparedRefresh {
            base_generation: pair.current,
            target,
            root,
            staged,
            started,
            walk: start.elapsed(),
        })
    }
    fn publish_refresh(&mut self, prepared: PreparedRefresh) -> Result<WriteReport> {
        let PreparedRefresh {
            base_generation,
            target,
            root,
            mut staged,
            started,
            walk,
        } = prepared;
        let start = Instant::now();
        let before = self.conn.total_changes();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if pair(&tx)?.map(|p| p.current) != Some(base_generation) {
            return Err(Error::new(
                ErrorCode::Rotated,
                "v2 generation changed during refresh walk; retry against the new generation",
            ));
        }
        let generation = next_generation(Some(base_generation))?;
        let bytes = &target.path;
        let upper = if target.sort_key.is_empty() {
            vec![255]
        } else {
            prefix_end(&target.sort_key)
        };
        let old = {
            let mut q = tx.prepare(&format!(
                "SELECT {COLUMNS} FROM live WHERE scope=?1 AND sort_key>=?2 AND sort_key<?3"
            ))?;
            q.query_map(params![target.scope, target.sort_key, upper], Entry::row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let links = hardlinks::refresh(&tx, &old, &mut staged.entries)?;
        fold(&mut staged.entries)?;
        let mut changes = vec![(
            target.parent,
            aggregate::Delta::between(&target, staged.entries.first()),
        )];
        for (previous, replacement) in links.entries {
            changes.push((
                previous.parent,
                aggregate::Delta::between(&previous, Some(&replacement)),
            ));
            retire(&tx, previous.id, generation)?;
            insert(&tx, &replacement, generation)?;
        }
        for e in &old {
            retire(&tx, e.id, generation)?;
            if e.kind == Kind::Mount {
                metadata::retire_mount(&tx, e.id, generation)?;
            }
        }
        let ids = assign_ids(&tx, &mut staged.entries, target.parent)?;
        for e in &staged.entries {
            insert(&tx, e, generation)?;
        }
        for mut mount in staged.mounts {
            mount.entry_id = ids[&mount.entry_id];
            metadata::insert_mount(&tx, &mount, generation)?;
        }
        let ancestors = aggregate::update(&tx, changes, generation)?;
        metadata::retire_root(&tx, root.entry_id, generation)?;
        metadata::insert_root(&tx, &root, generation)?;
        tx.execute(
            "INSERT INTO refreshes VALUES(?1,?2,?3)",
            params![generation, bytes, started],
        )?;
        tx.execute(
            "UPDATE control SET current=?1 WHERE singleton=1",
            [generation],
        )?;
        tx.commit()?;
        Ok(WriteReport {
            generation,
            old_subtree_entries: old.len(),
            new_subtree_entries: staged.entries.len(),
            ancestors,
            touched_inode_groups: links.groups,
            known_inode_members: links.members,
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
fn size(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::generic("size exceeds v2 integer range"))
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
fn assign_ids(
    conn: &Connection,
    entries: &mut [Entry],
    parent: Option<i64>,
) -> Result<HashMap<i64, i64>> {
    let mut ids = HashMap::with_capacity(entries.len());
    for entry in entries {
        conn.execute(
            "INSERT INTO identities(scope,path) VALUES(?1,?2) ON CONFLICT(scope,path) DO NOTHING",
            params![entry.scope, entry.path],
        )?;
        let id = conn.query_row(
            "SELECT id FROM identities WHERE scope=?1 AND path=?2",
            params![entry.scope, entry.path],
            |r| r.get(0),
        )?;
        ids.insert(entry.id, id);
        entry.parent = entry.parent.map(|p| ids[&p]).or(parent);
        entry.id = id;
    }
    Ok(ids)
}
fn live_path(conn: &Connection, path: &[u8]) -> Result<Option<Entry>> {
    let Some(scope) = metadata::scope(conn, path, None)? else {
        return Ok(None);
    };
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM live WHERE scope=?1 AND path=?2"),
            params![scope, path],
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
    conn.execute(&format!("INSERT INTO live({COLUMNS}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)"),
        params![e.id,e.parent,e.path,e.sort_key,e.kind.code(),e.flags,e.own_allocated,e.own_apparent,e.own_errors,e.mtime,e.scanned_at,e.allocated,e.apparent,e.items,e.errors,e.min_scanned_at,e.newest_mtime,e.scope,e.nlink,e.dev,e.ino])?;
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

#[cfg(test)]
mod tests;
