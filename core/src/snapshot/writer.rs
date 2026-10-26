//! Writes a finalized tree into a fresh SQLite file in one transaction
//! (`journal_mode=OFF`), checking free space every 64 MB of growth (reserve).

use std::path::{Path, PathBuf};
use std::time::Instant;

use rusqlite::{Connection, params};

use super::schema::{CREATE_INDEXES, CREATE_TABLES, SCHEMA_VERSION};
use super::{RESERVE_BYTES, fsutil};
use crate::error::{Error, ErrorCode, Result};
use crate::platform;
use crate::scan::{Finalized, NO_PARENT, Tree, flags};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshRow {
    pub path: Vec<u8>,
    pub scanned_at: i64,
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub struct WriteMeta {
    pub generation: u64,
    pub prev_generation: Option<u64>,
    pub scanned_at: i64,
    pub host: String,
    pub os: String,
    pub one_file_system: bool,
    pub scan_seconds: Option<f64>,
    pub refreshes: Vec<RefreshRow>,
}

impl WriteMeta {
    pub fn new(generation: u64, prev_generation: Option<u64>, scanned_at: i64) -> Self {
        WriteMeta {
            generation,
            prev_generation,
            scanned_at,
            host: platform::hostname(),
            os: platform::os_name(),
            one_file_system: true,
            scan_seconds: None,
            refreshes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WriteStats {
    pub rows: u64,
    pub bytes: u64,
    pub insert_seconds: f64,
    pub index_seconds: f64,
}

/// Free-space guard while the tmp file grows.
#[derive(Debug, Clone)]
pub struct Reserve {
    pub dir: PathBuf,
    pub min_free: u64,
    pub check_every: u64,
}

impl Reserve {
    pub fn new(dir: &Path) -> Self {
        Reserve {
            dir: dir.to_path_buf(),
            min_free: RESERVE_BYTES,
            check_every: 64 * 1024 * 1024,
        }
    }
}

pub(crate) fn reserve_error(have: u64, min_free: u64, dir: &Path) -> Error {
    let mut e = Error::new(
        ErrorCode::Space,
        format!(
            "free space on {} fell to {} while writing the snapshot (reserve {}); write aborted, the old snapshot is kept",
            dir.display(),
            crate::format::size(have),
            crate::format::size(min_free)
        ),
    )
    .with_fix("free some space on that filesystem or move snapshot_dir in /etc/freespace/config.toml");
    e.last_error = Some(crate::snapshot::state::LastError {
        ts: platform::now_secs(),
        op: "reserve".into(),
        message: e.message.clone(),
    });
    e
}

/// Generate a short random id for `meta.snapshot_id`.
pub fn snapshot_id() -> String {
    let mut b = [0u8; 8];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut b);
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn write_snapshot(
    path: &Path,
    tree: &Tree,
    fin: &Finalized,
    meta: &WriteMeta,
    reserve: &Reserve,
) -> Result<WriteStats> {
    let _ = std::fs::remove_file(path);
    let res = write_inner(path, tree, fin, meta, reserve);
    if res.is_err() {
        let _ = std::fs::remove_file(path);
    }
    res
}

fn write_inner(
    path: &Path,
    tree: &Tree,
    fin: &Finalized,
    meta: &WriteMeta,
    reserve: &Reserve,
) -> Result<WriteStats> {
    let conn = open_snapshot(path)?;
    let reserve_hit = install_reserve_guard(&conn, path, reserve)?;
    let map_err = |e: rusqlite::Error| -> Error {
        let have = reserve_hit.load(std::sync::atomic::Ordering::Relaxed);
        if have > 0 {
            reserve_error(have, reserve.min_free, &reserve.dir)
        } else if let rusqlite::Error::SqliteFailure(f, _) = &e
            && f.code == rusqlite::ErrorCode::DiskFull
        {
            let mut err = Error::new(
                ErrorCode::Space,
                format!("disk full while writing {}", path.display()),
            );
            err.last_error = Some(crate::snapshot::state::LastError {
                ts: platform::now_secs(),
                op: "write".into(),
                message: err.message.clone(),
            });
            err
        } else {
            Error::from(e)
        }
    };

    let mut stats = WriteStats::default();
    let t0 = Instant::now();
    conn.execute_batch("BEGIN")?;
    conn.execute_batch(CREATE_TABLES).map_err(map_err)?;
    insert_metadata(&conn, fin.order.len() as u64, meta, &map_err)?;
    insert_entries(&conn, tree, fin, &map_err)?;
    stats.rows = fin.order.len() as u64;
    insert_hardlinks(&conn, fin, &map_err)?;
    insert_roots(&conn, tree, fin, &map_err)?;
    insert_mounts(&conn, tree, fin, &map_err)?;
    insert_denied(&conn, tree, fin, &map_err)?;
    insert_refreshes(&conn, meta, &map_err)?;
    stats.insert_seconds = t0.elapsed().as_secs_f64();
    let t1 = Instant::now();
    for sql in CREATE_INDEXES {
        conn.execute_batch(sql).map_err(map_err)?;
    }
    conn.execute_batch("COMMIT").map_err(map_err)?;
    conn.execute_batch("PRAGMA analysis_limit=1000; ANALYZE;")
        .map_err(map_err)?;
    stats.index_seconds = t1.elapsed().as_secs_f64();
    conn.close().map_err(|(_, e)| map_err(e))?;

    stats.bytes = sync_snapshot(path)?;
    Ok(stats)
}

fn open_snapshot(path: &Path) -> Result<Connection> {
    // Create the file ourselves so the mode is right before SQLite writes a byte.
    let f = std::fs::File::create(path)
        .map_err(|e| Error::io(format!("create {}", path.display()), e))?;
    fsutil::secure_file(&f).map_err(|e| Error::io(format!("chmod {}", path.display()), e))?;
    drop(f);

    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=MEMORY;
         PRAGMA cache_size=-65536; PRAGMA locking_mode=EXCLUSIVE;",
    )?;

    Ok(conn)
}

fn install_reserve_guard(
    conn: &Connection,
    path: &Path,
    reserve: &Reserve,
) -> Result<std::sync::Arc<std::sync::atomic::AtomicU64>> {
    // Reserve check through SQLite's progress handler: covers inserts and index builds.
    let reserve_hit = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let p = path.to_path_buf();
    let r = reserve.clone();
    let hit = reserve_hit.clone();
    let mut last_checked: u64 = 0;
    let mut first = true;
    conn.progress_handler(
        200_000,
        Some(move || {
            let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            if first || len >= last_checked + r.check_every {
                first = false;
                last_checked = len;
                if let Ok(st) = platform::statvfs(&r.dir)
                    && st.avail < r.min_free
                {
                    hit.store(st.avail.max(1), std::sync::atomic::Ordering::Relaxed);
                    return true; // interrupt
                }
            }
            false
        }),
    )?;
    Ok(reserve_hit)
}

pub(crate) fn insert_metadata(
    conn: &Connection,
    entries: u64,
    meta: &WriteMeta,
    map_err: &impl Fn(rusqlite::Error) -> Error,
) -> Result<()> {
    conn.execute(
        "INSERT INTO meta VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            SCHEMA_VERSION,
            meta.generation as i64,
            meta.prev_generation.map(|g| g as i64),
            meta.scanned_at,
            meta.host,
            meta.os,
            snapshot_id(),
            entries as i64,
            meta.one_file_system as i64,
            meta.scan_seconds,
            crate::VERSION,
        ],
    )
    .map_err(map_err)?;
    Ok(())
}

fn insert_entries(
    conn: &Connection,
    tree: &Tree,
    fin: &Finalized,
    map_err: &impl Fn(rusqlite::Error) -> Error,
) -> Result<()> {
    let mut ins = conn
        .prepare(
            "INSERT INTO entries VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        )
        .map_err(map_err)?;
    for (new, &raw) in fin.order.iter().enumerate() {
        let n = &tree.nodes[raw as usize];
        let parent = if n.parent == NO_PARENT {
            None
        } else {
            Some(fin.rank[n.parent as usize] as i64)
        };
        let new = new as i64;
        ins.execute(params![
            new,
            parent,
            tree.name(n),
            fin.path_hash[new as usize],
            n.kind as i64,
            n.flags as i64,
            n.allocated as i64,
            n.apparent as i64,
            n.mtime,
            n.nlink as i64,
            n.items as i64,
            n.errors as i64,
            n.scanned_at,
            new + n.items as i64,
            n.min_scanned_at,
            n.newest_mtime,
        ])
        .map_err(map_err)?;
    }
    Ok(())
}

fn insert_hardlinks(
    conn: &Connection,
    fin: &Finalized,
    map_err: &impl Fn(rusqlite::Error) -> Error,
) -> Result<()> {
    let mut ins = conn
        .prepare("INSERT INTO hardlinks VALUES (?1, ?2, ?3, ?4, ?5)")
        .map_err(map_err)?;
    for h in &fin.hardlinks {
        ins.execute(params![
            h.entry_id as i64,
            h.dev as i64,
            h.ino as i64,
            h.allocated as i64,
            h.apparent as i64
        ])
        .map_err(map_err)?;
    }
    Ok(())
}

fn insert_roots(
    conn: &Connection,
    tree: &Tree,
    fin: &Finalized,
    map_err: &impl Fn(rusqlite::Error) -> Error,
) -> Result<()> {
    let mut ins = conn
        .prepare("INSERT INTO roots VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")
        .map_err(map_err)?;
    for (i, r) in tree.roots.iter().enumerate() {
        use std::os::unix::ffi::OsStrExt;
        ins.execute(params![
            fin.rank[i] as i64,
            r.path.as_os_str().as_bytes(),
            r.fs_type,
            r.stats.used as i64,
            r.stats.total as i64,
            r.stats.avail as i64,
            r.stats.inodes_used().map(|v| v as i64),
        ])
        .map_err(map_err)?;
    }
    Ok(())
}

fn insert_mounts(
    conn: &Connection,
    tree: &Tree,
    fin: &Finalized,
    map_err: &impl Fn(rusqlite::Error) -> Error,
) -> Result<()> {
    let mut ins = conn
        .prepare("INSERT INTO mounts VALUES (?1, ?2, ?3, ?4, ?5)")
        .map_err(map_err)?;
    for m in tree
        .mounts
        .iter()
        .filter(|m| fin.rank[m.raw as usize] != u32::MAX)
    {
        use std::os::unix::ffi::OsStrExt;
        ins.execute(params![
            fin.rank[m.raw as usize] as i64,
            m.path.as_os_str().as_bytes(),
            m.fs_type,
            m.stats.map(|s| s.used as i64),
            m.stats.map(|s| s.total as i64),
        ])
        .map_err(map_err)?;
    }
    Ok(())
}

fn insert_denied(
    conn: &Connection,
    tree: &Tree,
    fin: &Finalized,
    map_err: &impl Fn(rusqlite::Error) -> Error,
) -> Result<()> {
    let mut ins = conn
        .prepare("INSERT INTO denied VALUES (?1)")
        .map_err(map_err)?;
    for &d in &fin.denied {
        debug_assert!(tree.nodes[fin.order[d as usize] as usize].flags & flags::DENIED != 0);
        ins.execute(params![d as i64]).map_err(map_err)?;
    }
    Ok(())
}

fn insert_refreshes(
    conn: &Connection,
    meta: &WriteMeta,
    map_err: &impl Fn(rusqlite::Error) -> Error,
) -> Result<()> {
    let mut ins = conn
        .prepare("INSERT INTO refreshes VALUES (?1, ?2, ?3)")
        .map_err(map_err)?;
    for r in &meta.refreshes {
        ins.execute(params![r.path, r.scanned_at, r.generation as i64])
            .map_err(map_err)?;
    }
    Ok(())
}

pub(crate) fn sync_snapshot(path: &Path) -> Result<u64> {
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| Error::io(format!("open {}", path.display()), e))?;
    f.sync_all()
        .map_err(|e| Error::io(format!("fsync {}", path.display()), e))?;
    fsutil::secure_file(&f).map_err(|e| Error::io(format!("chmod {}", path.display()), e))?;
    Ok(f.metadata().map(|m| m.len()).unwrap_or(0))
}
