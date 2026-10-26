use super::{Entry, key, pair, prefix_end, schema, unsupported, validate_path};
use crate::{Error, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use schema::COLUMNS;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationPair {
    pub current: i64,
    pub prev: Option<i64>,
}

/// A reader owns one SQLite read transaction; its current/prev pair cannot tear.
/// Drop releases the WAL pin. Opening arbitrary retired generations is not exposed.
pub struct Reader {
    conn: Connection,
    pub generations: GenerationPair,
    full_scan: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: Vec<u8>,
    pub before: Option<Entry>,
    pub after: Option<Entry>,
}

impl Reader {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.execute_batch("BEGIN")?;
        let app: i64 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if app != schema::APP_ID || version != 2 {
            return Err(unsupported("not a v2 store"));
        }
        let generations = pair(&conn)?.ok_or_else(|| unsupported("v2 store has no scan"))?;
        let full_scan =
            conn.query_row("SELECT full_scan FROM control WHERE singleton=1", [], |r| {
                r.get(0)
            })?;
        Ok(Self {
            conn,
            generations,
            full_scan,
        })
    }
    fn generation(&self, previous: bool) -> Result<i64> {
        if previous {
            self.generations
                .prev
                .ok_or_else(|| Error::new(crate::ErrorCode::NoPrev, "no previous v2 generation"))
        } else {
            Ok(self.generations.current)
        }
    }
    pub fn get(&self, path: &[u8], previous: bool) -> Result<Option<Entry>> {
        validate_path(path)?;
        let g = self.generation(previous)?;
        Ok(self.conn.query_row(&format!("SELECT {COLUMNS} FROM versions WHERE path=?1 AND born<=?2 AND (dead IS NULL OR dead>?2)"),params![path,g],Entry::row).optional()?)
    }
    /// Component-wise name-ordered DFS, independent of stable IDs.
    pub fn subtree(&self, path: &[u8], previous: bool) -> Result<Vec<Entry>> {
        validate_path(path)?;
        let g = self.generation(previous)?;
        let lower = key(path);
        let upper = if lower.is_empty() {
            vec![255]
        } else {
            prefix_end(&lower)
        };
        let mut q=self.conn.prepare(&format!("SELECT {COLUMNS} FROM versions WHERE sort_key>=?1 AND sort_key<?2 AND born<=?3 AND (dead IS NULL OR dead>?3) ORDER BY sort_key"))?;
        Ok(q.query_map(params![lower, upper, g], Entry::row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn children(&self, path: &[u8], previous: bool) -> Result<Vec<Entry>> {
        let Some(parent) = self.get(path, previous)? else {
            return Ok(Vec::new());
        };
        let g = self.generation(previous)?;
        let mut q=self.conn.prepare(&format!("SELECT {COLUMNS} FROM versions WHERE parent=?1 AND born<=?2 AND (dead IS NULL OR dead>?2) ORDER BY sort_key"))?;
        Ok(q.query_map(params![parent.id, g], Entry::row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// Exact-path diff for the experimental API; this is not v1's filtered diff UI.
    /// Includes all metadata changes, including scan timestamps.
    pub fn diff(&self, path: &[u8]) -> Result<Vec<Change>> {
        let mut rows: BTreeMap<Vec<u8>, Change> = BTreeMap::new();
        for e in self.subtree(path, true)? {
            rows.insert(
                e.path.clone(),
                Change {
                    path: e.path.clone(),
                    before: Some(e),
                    after: None,
                },
            );
        }
        for e in self.subtree(path, false)? {
            let change = rows.entry(e.path.clone()).or_insert_with(|| Change {
                path: e.path.clone(),
                before: None,
                after: None,
            });
            change.after = Some(e);
        }
        Ok(rows.into_values().filter(|r| r.before != r.after).collect())
    }
    /// Refreshes since the current full scan, from the same pinned read transaction.
    pub fn refresh_history(&self) -> Result<Vec<(i64, Vec<u8>, i64)>> {
        let mut q=self.conn.prepare("SELECT generation,path,scanned_at FROM refreshes WHERE generation>?1 AND generation<=?2 ORDER BY generation")?;
        Ok(
            q.query_map(params![self.full_scan, self.generations.current], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?,
        )
    }
}
