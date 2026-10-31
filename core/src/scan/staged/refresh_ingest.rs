//! Disk-backed reconstruction of refresh snapshot own values.
//!
//! The staging connection owns these tables; the source is an immutable snapshot.
//! Keep original snapshot IDs until subtraction and link overrides are complete.

use rusqlite::{Connection, params};

use super::SpaceGuard;
use crate::error::{Error, Result};
use crate::scan::flags;

/// Import every old node before excluding the refreshed subtree.
/// `old_id` is the original preorder ID, not the eventual stage or output ID.
pub(super) fn import_nodes(
    source: &Connection,
    stage: &Connection,
    guard: &mut SpaceGuard,
) -> Result<u64> {
    stage.execute_batch(
        "CREATE TABLE old_raw (
            old_id INTEGER PRIMARY KEY, parent_id INTEGER, name BLOB NOT NULL,
            kind INTEGER NOT NULL, flags INTEGER NOT NULL,
            allocated INTEGER NOT NULL, apparent INTEGER NOT NULL,
            mtime INTEGER NOT NULL, nlink INTEGER NOT NULL,
            errors INTEGER NOT NULL, scanned_at INTEGER NOT NULL
        );",
    )?;
    let mut select = source.prepare_cached(
        "SELECT id,parent_id,name,kind,flags,allocated,apparent,mtime,nlink,errors,scanned_at \
         FROM entries ORDER BY id",
    )?;
    let mut rows = select.query([])?;
    let mut insert =
        stage.prepare_cached("INSERT INTO old_raw VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)")?;
    let mut count = 0u64;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        if id < 0 || id as u64 != count || count >= u64::from(u32::MAX) {
            return Err(Error::generic(
                "snapshot IDs are not dense or exceed raw ID capacity",
            ));
        }
        let flags_in = row.get::<_, i64>(4)? as u8;
        insert.execute(params![
            id,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, i64>(3)? as u8,
            flags_in & !flags::ALIAS,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)? as u32,
            row.get::<_, i64>(9)? as u32,
            row.get::<_, i64>(10)?,
        ])?;
        count += 1;
        guard.tick()?;
    }
    Ok(count)
}

/// Match `refresh::load::subtract_children`: each read observes previous writes.
/// SQLite signed SUM/subtraction cannot represent these unsigned saturating operations.
pub(super) fn subtract_children(
    stage: &Connection,
    count: u64,
    guard: &mut SpaceGuard,
) -> Result<()> {
    let mut child = stage.prepare_cached(
        "SELECT parent_id,allocated,apparent,errors FROM old_raw WHERE old_id=?1",
    )?;
    let mut parent =
        stage.prepare_cached("SELECT allocated,apparent,errors FROM old_raw WHERE old_id=?1")?;
    let mut update = stage
        .prepare_cached("UPDATE old_raw SET allocated=?2,apparent=?3,errors=?4 WHERE old_id=?1")?;
    for id in 0..count {
        guard.tick()?;
        let (p, ca, cp, ce): (Option<i64>, i64, i64, u32) = child.query_row([id as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        let Some(p) = p else { continue };
        let (pa, pp, pe): (i64, i64, u32) =
            parent.query_row([p], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        update.execute(params![
            p,
            (pa as u64).saturating_sub(ca as u64) as i64,
            (pp as u64).saturating_sub(cp as u64) as i64,
            pe.saturating_sub(ce),
        ])?;
    }
    Ok(())
}

/// Preserve the oracle's input iteration order and apply raw-size overrides
/// only after every parent subtraction. Descendant classification is deferred.
pub(super) fn import_links(
    source: &Connection,
    stage: &Connection,
    guard: &mut SpaceGuard,
) -> Result<()> {
    stage.execute_batch(
        "CREATE TABLE old_link (
            seq INTEGER PRIMARY KEY, old_id INTEGER NOT NULL, dev INTEGER NOT NULL,
            ino INTEGER NOT NULL, allocated INTEGER NOT NULL, apparent INTEGER NOT NULL,
            was_owner INTEGER NOT NULL
        );
        CREATE INDEX old_link_key ON old_link(dev,ino,seq);
        CREATE TABLE link_keys(dev INTEGER NOT NULL, ino INTEGER NOT NULL,
            PRIMARY KEY(dev,ino)) WITHOUT ROWID;",
    )?;
    let mut select = source.prepare_cached(
        "SELECT h.entry_id,h.dev,h.ino,h.allocated,h.apparent,(e.flags & 2)=0 \
         FROM hardlinks h JOIN entries e ON e.id=h.entry_id ORDER BY h.entry_id",
    )?;
    let mut rows = select.query([])?;
    let mut insert = stage.prepare_cached("INSERT INTO old_link VALUES (?1,?2,?3,?4,?5,?6,?7)")?;
    let mut update =
        stage.prepare_cached("UPDATE old_raw SET allocated=?2,apparent=?3 WHERE old_id=?1")?;
    let mut seq = 0i64;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let allocated: i64 = row.get(3)?;
        let apparent: i64 = row.get(4)?;
        insert.execute(params![
            seq,
            id,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            allocated,
            apparent,
            row.get::<_, bool>(5)?
        ])?;
        if update.execute(params![id, allocated, apparent])? != 1 {
            return Err(Error::generic("hardlink references missing snapshot entry"));
        }
        stage
            .prepare_cached("INSERT OR IGNORE INTO link_keys VALUES (?1,?2)")?
            .execute(params![row.get::<_, i64>(1)?, row.get::<_, i64>(2)?])?;
        guard.tick()?;
        seq = seq
            .checked_add(1)
            .ok_or_else(|| Error::generic("too many hardlinks"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "refresh_ingest/tests.rs"]
mod tests;
