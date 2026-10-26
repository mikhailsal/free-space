//! Conservative read-only leases, bounded row reclamation, explicit checkpoints.
//! An idle Reader can defer reclamation; admission budgets then stop new writes
//! instead of truncating required history or silently growing without bound.
use super::GenerationPair;
use crate::{Error, ErrorCode, Result};
use rusqlite::{Connection, params};
use std::{
    fs::{File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    pub max_records: u64,
    pub max_wal_bytes: u64,
}
#[derive(Debug, Default)]
pub struct MaintenanceReport {
    pub readers_blocking: bool,
    pub examined: usize,
    pub deleted: usize,
    pub retained_rows: u64,
    pub checkpoint_busy: bool,
    pub checkpoint_log_frames: i64,
    pub checkpointed_frames: i64,
    pub wal_bytes: u64,
}

fn lease_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(".readers");
    p.into()
}
pub(super) fn canonical(path: &Path) -> Result<PathBuf> {
    let path = std::fs::canonicalize(path).map_err(|e| Error::io("canonicalize v2 store", e))?;
    if std::fs::metadata(&path)
        .map_err(|e| Error::io("stat v2 store", e))?
        .nlink()
        != 1
    {
        return Err(Error::new(
            ErrorCode::NotSupported,
            "v2 live stores cannot have hardlinked filenames",
        ));
    }
    Ok(path)
}
pub(super) fn create_lease(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(crate::snapshot::fsutil::FILE_MODE)
        .open(lease_path(path))
        .map_err(|e| Error::storage_io("create v2 reader lease", e))?;
    crate::snapshot::fsutil::secure_file(&file)
        .map_err(|e| Error::io("secure v2 reader lease", e))?;
    Ok(())
}

fn lock(path: &Path, exclusive: bool) -> Result<Option<File>> {
    let file = File::open(lease_path(path)).map_err(|e| Error::io("open v2 reader lease", e))?;
    let mode = if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_SH
    };
    // flock is tied to this independent open-file description, and crash-safe.
    if unsafe { libc::flock(file.as_raw_fd(), mode | libc::LOCK_NB) } == 0 {
        return Ok(Some(file));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(Error::io("lock v2 reader lease", error))
    }
}
pub(super) fn shared(path: &Path) -> Result<File> {
    lock(path, false)?.ok_or_else(|| {
        Error::new(
            ErrorCode::Lock,
            "v2 maintenance is running; retry opening the reader",
        )
    })
}
pub(super) fn exclusive(path: &Path) -> Result<Option<File>> {
    lock(path, true)
}
pub(super) fn wal_bytes(path: &Path) -> Result<u64> {
    let mut p = path.as_os_str().to_os_string();
    p.push("-wal");
    match std::fs::metadata(p) {
        Ok(m) => Ok(m.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(Error::io("stat v2 WAL", e)),
    }
}
pub(super) fn policy(c: &Connection) -> Result<RetentionPolicy> {
    Ok(c.query_row(
        "SELECT max_records,max_wal_bytes FROM retention WHERE singleton=1",
        [],
        |r| {
            Ok(RetentionPolicy {
                max_records: r.get::<_, i64>(0)? as u64,
                max_wal_bytes: r.get::<_, i64>(1)? as u64,
            })
        },
    )?)
}
pub(super) fn rows(c: &Connection) -> Result<u64> {
    Ok(c.query_row(
        "SELECT retained_rows FROM storage_stats WHERE singleton=1",
        [],
        |r| r.get::<_, i64>(0),
    )? as u64)
}
pub(super) fn admit_wal(c: &Connection, path: &Path) -> Result<()> {
    if wal_bytes(path)? > policy(c)?.max_wal_bytes {
        return Err(capacity("WAL admission threshold reached"));
    }
    Ok(())
}
pub(super) fn admit_rows(c: &Connection, full_scan: bool, generation: i64) -> Result<()> {
    let required = rows(c)?;
    let limit = policy(c)?.max_records;
    if required <= limit {
        return Ok(());
    }
    // Only capacity failures need this root/mount metadata count. The successful
    // refresh path uses the O(1) retained-row counter and never enumerates it.
    let seed_rows:u64=c.query_row("SELECT m.entries+(SELECT count(*) FROM live_roots)+(SELECT count(*) FROM live_mounts)+1 FROM generation_meta m WHERE m.generation=?1",[generation],|r|r.get::<_,i64>(0))? as u64;
    let scans = if full_scan { 1 } else { 2 };
    let temporary = required + scans * seed_rows;
    Err(Error::new(ErrorCode::Space,format!("publication requires {required} retained records; limit is {limit}. Temporary recovery budget: {temporary} records ({} above the limit) for retrying this operation followed by {scans} full scan(s) of the same indexed tree",temporary.saturating_sub(limit)))
        .with_fix(format!("close old readers; set max_records to at least {temporary}; retry this operation; run {scans} full scan(s), then bounded maintenance and checkpoint. Keep prev history through the first rotation. If the filesystem grows, use the updated budget reported by scan. Restore the lower limit after reclamation")))
}

fn capacity(message: &str) -> Error {
    Error::new(
        ErrorCode::Space,
        format!(
            "{message}; close old v2 readers and run bounded maintenance, or explicitly increase retention limits"
        ),
    )
}
pub(super) fn configure(c: &Connection, p: RetentionPolicy) -> Result<()> {
    if p.max_records == 0
        || p.max_records > i64::MAX as u64
        || p.max_wal_bytes == 0
        || p.max_wal_bytes > i64::MAX as u64
    {
        return Err(Error::usage(
            "retention budgets must be positive signed 64-bit values",
        ));
    }
    c.execute(
        "UPDATE retention SET max_records=?1,max_wal_bytes=?2 WHERE singleton=1",
        params![p.max_records as i64, p.max_wal_bytes as i64],
    )?;
    Ok(())
}

struct Pins {
    generations: Vec<i64>,
    history: Vec<(i64, i64)>,
}
impl Pins {
    fn read(c: &Connection, p: Option<GenerationPair>) -> Result<Self> {
        let generations = p
            .map(|p| std::iter::once(p.current).chain(p.prev).collect())
            .unwrap_or_default();
        let mut history = Vec::new();
        for &g in &generations {
            let start = c.query_row(
                "SELECT full_scan FROM generation_meta WHERE generation=?1",
                [g],
                |r| r.get(0),
            )?;
            history.push((start, g));
        }
        Ok(Self {
            generations,
            history,
        })
    }
    fn covers(&self, born: i64, dead: i64) -> bool {
        self.generations.iter().any(|g| born <= *g && *g < dead)
    }
}
const TABLES: [(&str, &str); 6] = [
    ("versions", "id"),
    ("root_versions", "entry_id"),
    ("mount_versions", "entry_id"),
    ("generation_meta", "generation"),
    ("refreshes", "generation"),
    ("identities", "id"),
];
pub(super) fn collect(
    c: &Connection,
    p: Option<GenerationPair>,
    budget: usize,
) -> Result<(usize, usize)> {
    let pins = Pins::read(c, p)?;
    let start: i64 = c.query_row(
        "SELECT next_kind FROM retention WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let mut examined = 0;
    let mut deleted = 0;
    for offset in 0..6 {
        if examined == budget {
            break;
        }
        let kind = ((start + offset) % 6) as usize;
        let (seen, gone) = collect_table(c, &pins, kind, budget - examined)?;
        examined += seen;
        deleted += gone;
    }
    c.execute(
        "UPDATE retention SET next_kind=?1 WHERE singleton=1",
        [(start + 1) % 6],
    )?;
    Ok((examined, deleted))
}
fn collect_table(
    c: &Connection,
    pins: &Pins,
    kind: usize,
    budget: usize,
) -> Result<(usize, usize)> {
    let (table, id) = TABLES[kind];
    let cursor = c.query_row(
        "SELECT k1,k2,k3 FROM gc_cursor WHERE kind=?1",
        [kind as i64],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )?;
    let sql = if kind < 3 {
        format!(
            "SELECT dead,{id},born FROM {table} WHERE dead IS NOT NULL AND (dead,{id},born)>(?1,?2,?3) ORDER BY dead,{id},born LIMIT ?4"
        )
    } else {
        format!(
            "SELECT {id},0,0 FROM {table} WHERE {id}>?1 AND ?2>=0 AND ?3>=0 ORDER BY {id} LIMIT ?4"
        )
    };
    let candidates = {
        let mut q = c.prepare(&sql)?;
        q.query_map(params![cursor.0, cursor.1, cursor.2, budget as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut deleted = 0;
    for &(a, b, d) in &candidates {
        let keep = match kind {
            0..=2 => pins.covers(d, a),
            3 => pins.generations.contains(&a),
            4 => pins
                .history
                .iter()
                .any(|(start, end)| a > *start && a <= *end),
            _ => c.query_row(
                "SELECT EXISTS(SELECT 1 FROM versions WHERE id=?1)",
                [a],
                |r| r.get(0),
            )?,
        };
        if !keep {
            deleted += if kind < 3 {
                c.execute(
                    &format!("DELETE FROM {table} WHERE {id}=?1 AND born=?2"),
                    params![b, d],
                )?
            } else {
                c.execute(&format!("DELETE FROM {table} WHERE {id}=?1"), [a])?
            };
        }
    }
    let next = candidates.last().copied().unwrap_or((0, 0, 0));
    c.execute(
        "UPDATE gc_cursor SET k1=?1,k2=?2,k3=?3 WHERE kind=?4",
        params![next.0, next.1, next.2, kind as i64],
    )?;
    Ok((candidates.len(), deleted))
}
pub(super) fn checkpoint(c: &Connection) -> Result<(bool, i64, i64)> {
    c.busy_timeout(Duration::ZERO)?;
    let result = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
        Ok((r.get::<_, i64>(0)? != 0, r.get(1)?, r.get(2)?))
    });
    c.busy_timeout(Duration::from_secs(5))?;
    Ok(result?)
}

pub(super) fn prepare_checkpoint(
    c: &Connection,
    path: &Path,
    requested: bool,
) -> Result<Option<MaintenanceReport>> {
    if wal_bytes(path)? <= policy(c)?.max_wal_bytes {
        return Ok(None);
    }
    if !requested {
        admit_wal(c, path)?;
    }
    let (busy, log, done) = checkpoint(c)?;
    if !busy {
        return Ok(None);
    }
    Ok(Some(MaintenanceReport {
        checkpoint_busy: true,
        checkpoint_log_frames: log,
        checkpointed_frames: done,
        retained_rows: rows(c)?,
        wal_bytes: wal_bytes(path)?,
        ..Default::default()
    }))
}
