use super::{Entry, Mount, Root, key, metadata, pair, prefix_end, schema, validate_path};
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
    // Last field: release the SQLite read transaction before releasing its lease.
    _lease: std::fs::File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: Vec<u8>,
    pub before: Option<Entry>,
    pub after: Option<Entry>,
}

impl Reader {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_inner(path).map_err(|error| super::files::reader_error(path, error))
    }

    fn open_inner(path: &Path) -> Result<Self> {
        if let Err(e) = std::fs::metadata(path) {
            return Err(if e.kind() == std::io::ErrorKind::NotFound {
                Error::new(
                    crate::ErrorCode::NoSnapshot,
                    "no v2 store at the requested path",
                )
            } else {
                Error::io("stat v2 store", e)
            });
        }
        let path = super::retention::canonical(path)?;
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let schema_error = |e: rusqlite::Error| {
            Error::new(crate::ErrorCode::Schema, format!("invalid v2 store: {e}"))
        };
        let app: i64 = conn
            .pragma_query_value(None, "application_id", |r| r.get(0))
            .map_err(schema_error)?;
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(schema_error)?;
        if app != schema::APP_ID || version != schema::REVISION {
            return Err(Error::new(
                crate::ErrorCode::Schema,
                "not a supported v2 store revision",
            ));
        }
        let lease = super::retention::shared(&path)?;
        super::files::persist_wal(&conn)?;
        conn.execute_batch("BEGIN")?;
        let generations = pair(&conn)?.ok_or_else(|| {
            Error::new(
                crate::ErrorCode::NoSnapshot,
                "v2 store has no published generation",
            )
        })?;
        let full_scan =
            conn.query_row("SELECT full_scan FROM control WHERE singleton=1", [], |r| {
                r.get(0)
            })?;
        Ok(Self {
            conn,
            generations,
            full_scan,
            _lease: lease,
        })
    }
    pub(crate) fn generation(&self, previous: bool) -> Result<i64> {
        if previous {
            self.generations
                .prev
                .ok_or_else(|| Error::new(crate::ErrorCode::NoPrev, "no previous v2 generation"))
        } else {
            Ok(self.generations.current)
        }
    }
    pub(crate) fn with_pin<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        f(self)
    }
    pub(crate) fn connection(&self) -> &Connection {
        &self.conn
    }
    pub fn meta(&self, previous: bool) -> Result<crate::snapshot::reader::Meta> {
        metadata::generation(&self.conn, self.generation(previous)?)
    }
    pub fn get_id(&self, id: i64, previous: bool) -> Result<Option<Entry>> {
        let g = self.generation(previous)?;
        Ok(self.conn.query_row(&format!("SELECT {COLUMNS} FROM versions WHERE id=?1 AND born<=?2 AND (dead IS NULL OR dead>?2)"),params![id,g],Entry::row).optional()?)
    }
    pub fn get(&self, path: &[u8], previous: bool) -> Result<Option<Entry>> {
        validate_path(path)?;
        let g = self.generation(previous)?;
        let Some(scope) = metadata::scope(&self.conn, path, Some(g))? else {
            return Ok(None);
        };
        Ok(self.conn.query_row(&format!("SELECT {COLUMNS} FROM versions WHERE scope=?1 AND path=?2 AND born<=?3 AND (dead IS NULL OR dead>?3)"),params![scope,path,g],Entry::row).optional()?)
    }
    /// Component-wise name-ordered DFS, independent of stable IDs.
    pub fn subtree(&self, path: &[u8], previous: bool) -> Result<Vec<Entry>> {
        validate_path(path)?;
        let g = self.generation(previous)?;
        let Some(scope) = metadata::scope(&self.conn, path, Some(g))? else {
            return Ok(Vec::new());
        };
        let lower = key(path);
        let upper = if lower.is_empty() {
            vec![255]
        } else {
            prefix_end(&lower)
        };
        let mut q=self.conn.prepare(&format!("SELECT {COLUMNS} FROM versions WHERE scope=?1 AND sort_key>=?2 AND sort_key<?3 AND born<=?4 AND (dead IS NULL OR dead>?4) ORDER BY sort_key"))?;
        Ok(q.query_map(params![scope, lower, upper, g], Entry::row)?
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
    pub fn roots(&self, previous: bool) -> Result<Vec<Root>> {
        let g = self.generation(previous)?;
        let mut q=self.conn.prepare(&format!("SELECT {} FROM root_versions WHERE born<=?1 AND (dead IS NULL OR dead>?1) ORDER BY ordinal",metadata::ROOT_COLS))?;
        Ok(q.query_map([g], Root::row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// Entry ID disambiguates a mount placeholder from a configured nested root.
    pub fn mount(&self, entry_id: i64, previous: bool) -> Result<Option<Mount>> {
        let g = self.generation(previous)?;
        Ok(self.conn.query_row(&format!("SELECT {} FROM mount_versions WHERE entry_id=?1 AND born<=?2 AND (dead IS NULL OR dead>?2)",metadata::MOUNT_COLS),params![entry_id,g],Mount::row).optional()?)
    }
    pub fn hardlink_owner(&self, path: &[u8], previous: bool) -> Result<Option<Vec<u8>>> {
        let Some(entry) = self.get(path, previous)? else {
            return Ok(None);
        };
        let Some((dev, ino)) = entry.dev.zip(entry.ino) else {
            return Ok(None);
        };
        let g = self.generation(previous)?;
        Ok(self.conn.query_row("SELECT path FROM versions WHERE dev=?1 AND ino=?2 AND (flags & 2)=0 AND born<=?3 AND (dead IS NULL OR dead>?3) LIMIT 1",params![dev,ino,g],|r|r.get(0)).optional()?)
    }
    /// Stream every denied entry in configured-root/preorder order, including
    /// nested denied entries. The caller chooses its report representation.
    pub fn visit_denied(&self, mut visit: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
        let mut q = self.conn.prepare("SELECT path FROM versions WHERE scope=?1 AND (flags & 1)!=0 AND born<=?2 AND (dead IS NULL OR dead>?2) ORDER BY sort_key,born")?;
        for root in self.roots(false)? {
            let mut rows = q.query(params![root.path, self.generations.current])?;
            while let Some(row) = rows.next()? {
                let path: Vec<u8> = row.get(0)?;
                visit(&path)?;
            }
        }
        Ok(())
    }
    /// Complete disk-spooled scan report; no entry-sized vector or history load.
    pub fn denied_report(&self, dir: &Path) -> Result<crate::scan::denied::DeniedPaths> {
        let mut report = crate::scan::denied::DeniedPaths::on_disk(dir)?;
        self.visit_denied(|path| report.push(path))?;
        Ok(report)
    }
    /// Global incompleteness and capped non-nested denied paths, from the pin.
    pub fn unknown_subtrees(&self, previous: bool, cap: usize) -> Result<(bool, Vec<Vec<u8>>)> {
        let g = self.generation(previous)?;
        let mut q=self.conn.prepare("SELECT e.scope,e.sort_key,e.path FROM versions e JOIN root_versions r ON r.path=e.scope AND r.born<=?1 AND (r.dead IS NULL OR r.dead>?1) WHERE (e.flags & 1)!=0 AND e.born<=?1 AND (e.dead IS NULL OR e.dead>?1) ORDER BY r.ordinal,e.sort_key")?;
        let mut rows = q.query([g])?;
        let mut paths = Vec::new();
        let mut enclosing: Option<(Vec<u8>, Vec<u8>)> = None;
        let mut incomplete = false;
        while let Some(row) = rows.next()? {
            incomplete = true;
            if paths.len() == cap {
                break;
            }
            let scope: Vec<u8> = row.get(0)?;
            let key: Vec<u8> = row.get(1)?;
            if enclosing
                .as_ref()
                .is_some_and(|(s, k)| *s == scope && key.starts_with(k))
            {
                continue;
            }
            paths.push(row.get(2)?);
            enclosing = Some((scope, key));
        }
        Ok((incomplete, paths))
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
