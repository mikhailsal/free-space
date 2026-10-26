//! Experimental, opt-in generation store. It never opens or migrates v1 snapshots.
//!
//! See `docs/designs/refresh-v2-migration.md` for supported scope and adoption gates.
//! Identity means exact raw path; traversal order and generation are independent.
//! Reader leases, bounded reclamation and explicit admission limits protect history.

mod aggregate;
mod disk;
mod files;
mod hardlinks;
mod metadata;
mod reader;
mod retention;
mod seed;
mod writer;
pub use retention::{MaintenanceReport, RetentionPolicy};
mod staging;
pub use metadata::{Mount, Root};
mod schema;
pub use reader::{Change, GenerationPair, Reader};

use crate::scan::{self, Kind, Tree, WalkOptions};
use crate::{Error, ErrorCode, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use schema::COLUMNS;
pub(crate) const ENTRY_COLUMNS: &str = schema::COLUMNS;
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
    pub(crate) fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
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
    path: PathBuf,
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
        let conn = files::open(path)?;
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
            schema::accounting(&conn)?;
            conn.pragma_update(None, "application_id", schema::APP_ID)?;
            conn.pragma_update(None, "user_version", schema::REVISION)?;
            conn.execute_batch("COMMIT")?;
        } else if app != schema::APP_ID || version != schema::REVISION {
            return Err(unsupported("refusing non-v2 database"));
        }
        let path = retention::canonical(path)?;
        files::secure(&path)?;
        files::persist_wal(&conn)?;
        retention::create_lease(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=0; PRAGMA journal_size_limit=0;",
        )?;
        Ok(Self { conn, path })
    }

    /// Set explicit admission budgets; active/current history is never truncated.
    pub fn configure_retention(&mut self, policy: RetentionPolicy) -> Result<()> {
        retention::configure(&self.conn, policy)
    }
    /// Read the persisted admission budgets before an operational adjustment.
    pub fn retention_policy(&self) -> Result<RetentionPolicy> {
        retention::policy(&self.conn)
    }
    pub fn retained_records(&self) -> Result<u64> {
        retention::rows(&self.conn)
    }
    /// Examine at most `budget` records. Optional checkpoint is explicitly global
    /// WAL work, separately requested, and never part of scan/refresh publication.
    pub fn maintain(&mut self, budget: usize, checkpoint: bool) -> Result<MaintenanceReport> {
        if budget == 0 || budget > 100000 {
            return Err(Error::usage("maintenance budget must be 1..=100000"));
        }
        let Some(_lease) = retention::exclusive(&self.path)? else {
            return Ok(MaintenanceReport {
                readers_blocking: true,
                retained_rows: retention::rows(&self.conn)?,
                wal_bytes: retention::wal_bytes(&self.path)?,
                ..Default::default()
            });
        };
        if let Some(report) = retention::prepare_checkpoint(&self.conn, &self.path, checkpoint)? {
            return Ok(report);
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        retention::admit_wal(&tx, &self.path)?;
        let (examined, deleted) = retention::collect(&tx, pair(&tx)?, budget)?;
        tx.commit()?;
        let (checkpoint_busy, checkpoint_log_frames, checkpointed_frames) = if checkpoint {
            retention::checkpoint(&self.conn)?
        } else {
            (false, 0, 0)
        };
        Ok(MaintenanceReport {
            examined,
            deleted,
            retained_rows: retention::rows(&self.conn)?,
            checkpoint_busy,
            checkpoint_log_frames,
            checkpointed_frames,
            wal_bytes: retention::wal_bytes(&self.path)?,
            readers_blocking: false,
        })
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
        let id = identity(conn, &entry.scope, &entry.path)?;
        ids.insert(entry.id, id);
        entry.parent = entry.parent.map(|p| ids[&p]).or(parent);
        entry.id = id;
    }
    Ok(ids)
}
fn identity(conn: &Connection, scope: &[u8], path: &[u8]) -> Result<i64> {
    conn.prepare_cached(
        "INSERT INTO identities(scope,path) VALUES(?1,?2) ON CONFLICT(scope,path) DO NOTHING",
    )?
    .execute(params![scope, path])?;
    Ok(conn
        .prepare_cached("SELECT id FROM identities WHERE scope=?1 AND path=?2")?
        .query_row(params![scope, path], |r| r.get(0))?)
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
    conn.prepare_cached(&format!("INSERT INTO live({COLUMNS}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)"))?.execute(
        params![e.id,e.parent,e.path,e.sort_key,e.kind.code(),e.flags,e.own_allocated,e.own_apparent,e.own_errors,e.mtime,e.scanned_at,e.allocated,e.apparent,e.items,e.errors,e.min_scanned_at,e.newest_mtime,e.scope,e.nlink,e.dev,e.ino])?;
    conn.prepare_cached(&format!(
        "INSERT INTO versions({COLUMNS},born,dead) SELECT {COLUMNS},?1,NULL FROM live WHERE id=?2"
    ))?
    .execute(params![generation, e.id])?;
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
