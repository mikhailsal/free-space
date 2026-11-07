//! The tables a coarse snapshot adds to the v1 snapshot format. The v1 tables carry the
//! retained entries and one pseudo entry per non-empty bucket, so every v1 query works on
//! them as they are; these tables hold what a refresh needs and a query does not: the
//! exact tiling of every directory (empty buckets included) and the threshold per root.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::model::{BUCKET_ENTRIES, Bucket, FORMAT, SLOT_BITS, Stats, unreadable};
use crate::error::{Error, Result};
use crate::snapshot::writer;

const CREATE: &str = "
CREATE TABLE coarse_meta (
  format INTEGER NOT NULL,
  ppm INTEGER NOT NULL,
  slot_bits INTEGER NOT NULL,
  bucket_entries INTEGER NOT NULL
);
CREATE TABLE coarse_roots (
  entry_id INTEGER PRIMARY KEY,
  threshold INTEGER NOT NULL,
  capacity INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE coarse_buckets (
  parent_id INTEGER NOT NULL,
  lo INTEGER NOT NULL,
  hi INTEGER NOT NULL,
  entry_id INTEGER,
  allocated INTEGER NOT NULL,
  apparent INTEGER NOT NULL,
  entries INTEGER NOT NULL,
  errors INTEGER NOT NULL,
  links INTEGER NOT NULL,
  min_scanned_at INTEGER,
  newest_mtime INTEGER,
  PRIMARY KEY (parent_id, lo)
) WITHOUT ROWID;
";

/// A bucket of the retained directory with row id `parent_id`; `entry_id` is the id of its
/// pseudo entry, absent while the bucket is empty.
pub struct BucketRow {
    pub parent_id: u32,
    pub entry_id: Option<u32>,
    pub bucket: Bucket,
}

pub struct RootRow {
    pub entry_id: u32,
    pub threshold: u64,
    pub capacity: u64,
}

pub struct Side {
    pub ppm: u32,
    pub roots: Vec<RootRow>,
    pub buckets: Vec<BucketRow>,
}

/// Add the coarse tables to a snapshot the v1 writer has finished.
pub fn write(path: &Path, side: &Side) -> Result<u64> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;")?;
    conn.execute_batch("BEGIN")?;
    conn.execute_batch(CREATE)?;
    conn.execute(
        "INSERT INTO coarse_meta VALUES (?1, ?2, ?3, ?4)",
        params![FORMAT, side.ppm, SLOT_BITS, BUCKET_ENTRIES as i64],
    )?;
    {
        let mut insert = conn.prepare("INSERT INTO coarse_roots VALUES (?1, ?2, ?3)")?;
        for r in &side.roots {
            insert.execute(params![r.entry_id, r.threshold as i64, r.capacity as i64])?;
        }
        let mut insert = conn.prepare(
            "INSERT INTO coarse_buckets VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for row in &side.buckets {
            let s = &row.bucket.stats;
            insert.execute(params![
                row.parent_id,
                row.bucket.lo,
                row.bucket.hi,
                row.entry_id,
                s.allocated as i64,
                s.apparent as i64,
                s.entries as i64,
                s.errors as i64,
                s.links as i64,
                s.min_scanned_at,
                s.newest_mtime,
            ])?;
        }
    }
    conn.execute_batch("COMMIT")?;
    conn.close().map_err(|(_, e)| Error::from(e))?;
    writer::sync_snapshot(path)
}

/// The size below which a coarse snapshot folds entries (the largest threshold of its
/// roots); `None` for a snapshot that is not a coarse one.
pub fn folded_below(conn: &Connection) -> Option<u64> {
    conn.query_row("SELECT MAX(threshold) FROM coarse_roots", [], |r| {
        r.get::<_, Option<i64>>(0)
    })
    .ok()
    .flatten()
    .map(|bytes| u64::try_from(bytes).unwrap_or(0))
}

/// What the coarse tables of an open snapshot say; fails when it is not a coarse snapshot
/// or was written with other slot parameters.
pub fn read(conn: &Connection) -> Result<Side> {
    let meta: Option<(i64, u32, u32, i64)> = conn
        .query_row(
            "SELECT format, ppm, slot_bits, bucket_entries FROM coarse_meta",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|_| unreadable("it was not written by the coarse backend"))?;
    let Some((format, ppm, slot_bits, bucket_entries)) = meta else {
        return Err(unreadable("its coarse tables are empty"));
    };
    if format != FORMAT || slot_bits != SLOT_BITS || bucket_entries != BUCKET_ENTRIES as i64 {
        return Err(unreadable("it was written with other bucket parameters"));
    }
    Ok(Side {
        ppm,
        roots: read_roots(conn)?,
        buckets: read_buckets(conn)?,
    })
}

fn read_roots(conn: &Connection) -> Result<Vec<RootRow>> {
    let mut st =
        conn.prepare("SELECT entry_id, threshold, capacity FROM coarse_roots ORDER BY entry_id")?;
    let rows = st
        .query_map([], |r| {
            Ok(RootRow {
                entry_id: r.get(0)?,
                threshold: r.get::<_, i64>(1)? as u64,
                capacity: r.get::<_, i64>(2)? as u64,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_buckets(conn: &Connection) -> Result<Vec<BucketRow>> {
    let mut st = conn.prepare(
        "SELECT parent_id, lo, hi, entry_id, allocated, apparent, entries, errors, links, min_scanned_at, newest_mtime
         FROM coarse_buckets ORDER BY parent_id, lo",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok(BucketRow {
                parent_id: r.get(0)?,
                entry_id: r.get(3)?,
                bucket: Bucket {
                    lo: r.get(1)?,
                    hi: r.get(2)?,
                    stats: Stats {
                        allocated: r.get::<_, i64>(4)? as u64,
                        apparent: r.get::<_, i64>(5)? as u64,
                        entries: r.get::<_, i64>(6)? as u64,
                        errors: r.get::<_, i64>(7)? as u64,
                        links: r.get::<_, i64>(8)? as u64,
                        min_scanned_at: r.get(9)?,
                        newest_mtime: r.get(10)?,
                    },
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
