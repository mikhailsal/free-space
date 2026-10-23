//! Read-only access to a published snapshot: `immutable=1`, no -wal/-shm files,
//! temp files in memory. Every connection used by queries comes from [`open`].

use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use super::schema::SCHEMA_VERSION;
use super::{SnapshotDir, lock};
use crate::error::{Error, ErrorCode, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub schema_version: i64,
    pub generation: u64,
    pub prev_generation: Option<u64>,
    pub scanned_at: i64,
    pub host: String,
    pub os: String,
    pub snapshot_id: String,
    pub entries: u64,
    pub one_file_system: bool,
    pub freespace_version: String,
}

fn uri_escape(p: &Path) -> String {
    let mut s = String::from("file:");
    for &b in p.as_os_str().as_bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                s.push(b as char)
            }
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s
}

pub fn no_snapshot(dir: &SnapshotDir) -> Error {
    let mut e = Error::new(
        ErrorCode::NoSnapshot,
        format!("no snapshot in {}", dir.path.display()),
    )
    .with_fix(format!("ask a human to run: {}", crate::INSTALL_CMD));
    if let Some(s) = lock::probe(dir) {
        e.message = format!(
            "no snapshot in {} yet: {} of {} running since {}, wait",
            dir.path.display(),
            s.op,
            s.path,
            crate::format::local_hm(s.started_at)
        );
        e.fix = Some("wait for the first scan: freespace status shows progress".into());
        e.scanning = Some(s);
    }
    if let Ok(Some(st)) = super::state::read(dir) {
        e.last_error = st.last_error;
    }
    e
}

pub fn access_error(path: &Path, err: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::SnapshotAccess,
        format!("cannot read {}: {err}", path.display()),
    )
    .with_fix("ask a human to run: sudo freespace install-timer (grants this user read access)")
}

/// Open `path` read-only and immutable, mapping missing/unreadable files to
/// NO_SNAPSHOT / SNAPSHOT_ACCESS.
pub fn open_file(path: &Path) -> Result<Connection> {
    // Probe with open(2) first: SQLite's own errors lose the errno.
    if let Err(e) = std::fs::File::open(path) {
        return Err(match e.kind() {
            ErrorKind::NotFound => Error::new(
                ErrorCode::NoSnapshot,
                format!("no snapshot at {}", path.display()),
            ),
            ErrorKind::PermissionDenied => access_error(path, &e),
            _ => Error::io(format!("open {}", path.display()), e),
        });
    }
    let uri = format!("{}?immutable=1&mode=ro", uri_escape(path));
    let conn = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.execute_batch(
        "PRAGMA temp_store=MEMORY; PRAGMA cache_size=-32768; PRAGMA mmap_size=268435456;",
    )?;
    crate::queries::glob::register(&conn)?;
    Ok(conn)
}

pub fn read_meta(conn: &Connection) -> Result<Meta> {
    let m = conn
        .query_row(
            "SELECT schema_version, generation, prev_generation, scanned_at, host, os, snapshot_id,
                    entries, one_file_system, freespace_version FROM meta",
            [],
            |r| {
                Ok(Meta {
                    schema_version: r.get(0)?,
                    generation: r.get::<_, i64>(1)? as u64,
                    prev_generation: r.get::<_, Option<i64>>(2)?.map(|g| g as u64),
                    scanned_at: r.get(3)?,
                    host: r.get(4)?,
                    os: r.get(5)?,
                    snapshot_id: r.get(6)?,
                    entries: r.get::<_, i64>(7)? as u64,
                    one_file_system: r.get::<_, i64>(8)? != 0,
                    freespace_version: r.get(9)?,
                })
            },
        )
        .optional()
        .map_err(|e| schema_error(&format!("meta unreadable: {e}")))?;
    m.ok_or_else(|| schema_error("meta table is empty"))
}

fn schema_error(detail: &str) -> Error {
    Error::new(
        ErrorCode::Schema,
        format!("snapshot schema mismatch ({detail}), rescan needed"),
    )
    .with_fix("ask a human to run: sudo systemctl start freespace.service (or sudo freespace scan)")
}

/// An open snapshot: connection + meta, schema-checked.
pub struct Snapshot {
    pub conn: Connection,
    pub meta: Meta,
    pub path: std::path::PathBuf,
}

impl Snapshot {
    pub fn open(path: &Path) -> Result<Snapshot> {
        let conn = open_file(path)?;
        let meta = read_meta(&conn)?;
        if meta.schema_version != SCHEMA_VERSION {
            let mut e = schema_error(&format!(
                "file has schema {}, this binary reads {}",
                meta.schema_version, SCHEMA_VERSION
            ));
            if let Some(dir) = path.parent()
                && let Some(s) = lock::probe(&SnapshotDir::new(dir))
            {
                e.message = format!(
                    "snapshot schema {} → {}: rescan running since {}",
                    meta.schema_version,
                    SCHEMA_VERSION,
                    crate::format::local_hm(s.started_at)
                );
                e.scanning = Some(s);
            }
            return Err(e);
        }
        Ok(Snapshot {
            conn,
            meta,
            path: path.to_path_buf(),
        })
    }

    /// Open the published snapshot of `dir`.
    pub fn open_current(dir: &SnapshotDir) -> Result<Snapshot> {
        match Snapshot::open(&dir.current()) {
            Err(e) if e.code == ErrorCode::NoSnapshot => Err(no_snapshot(dir)),
            r => r,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_escapes_specials() {
        assert_eq!(
            uri_escape(Path::new("/var/lib/a b?#")),
            "file:/var/lib/a%20b%3F%23"
        );
    }

    #[test]
    fn missing_is_no_snapshot() {
        let d = tempfile::tempdir().unwrap();
        let dir = SnapshotDir::new(d.path());
        let e = Snapshot::open_current(&dir).err().unwrap();
        assert_eq!(e.code, ErrorCode::NoSnapshot);
        assert_eq!(e.exit(), 8);
    }
}
