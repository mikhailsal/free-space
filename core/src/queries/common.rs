//! Rows, path reconstruction, path resolution and the response header.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use super::params::hex_encode;
use super::types::{EntryOut, Header, MountOut};
use crate::error::{Error, ErrorCode, Result};
use crate::platform::now_secs;
use crate::scan::tree::{path_hash, push_component};
use crate::scan::{Kind, flags};
use crate::snapshot::reader::Snapshot;

pub const COLS: &str = "id, parent_id, name, kind, flags, allocated, apparent, mtime, nlink, items, errors, \
                        scanned_at, subtree_end, min_scanned_at, newest_mtime";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: Vec<u8>,
    pub kind: u8,
    pub flags: u8,
    pub allocated: i64,
    pub apparent: i64,
    pub mtime: i64,
    pub nlink: i64,
    pub items: i64,
    pub errors: i64,
    pub scanned_at: i64,
    pub subtree_end: i64,
    pub min_scanned_at: i64,
    pub newest_mtime: i64,
}

impl Row {
    pub fn from_sql(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
        Ok(Row {
            id: r.get(0)?,
            parent_id: r.get(1)?,
            name: r.get(2)?,
            kind: r.get::<_, i64>(3)? as u8,
            flags: r.get::<_, i64>(4)? as u8,
            allocated: r.get(5)?,
            apparent: r.get(6)?,
            mtime: r.get(7)?,
            nlink: r.get(8)?,
            items: r.get(9)?,
            errors: r.get(10)?,
            scanned_at: r.get(11)?,
            subtree_end: r.get(12)?,
            min_scanned_at: r.get(13)?,
            newest_mtime: r.get(14)?,
        })
    }

    pub fn kind(&self) -> Kind {
        Kind::from_code(self.kind)
    }

    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir.code()
    }

    /// The date `--older-than`/`--newer-than` compare: `newest_mtime` for
    /// directories (D2/D18), `mtime` for everything else.
    pub fn filter_date(&self) -> i64 {
        if self.is_dir() {
            self.newest_mtime
        } else {
            self.mtime
        }
    }
}

pub fn get_row(conn: &Connection, id: i64) -> Result<Option<Row>> {
    let mut st = conn.prepare_cached(&format!("SELECT {COLS} FROM entries WHERE id = ?1"))?;
    Ok(st.query_row(params![id], Row::from_sql).optional()?)
}

/// Full paths by id, memoized along parent chains.
pub struct Paths<'c> {
    conn: &'c Connection,
    cache: HashMap<i64, Vec<u8>>,
}

impl<'c> Paths<'c> {
    pub fn new(conn: &'c Connection) -> Self {
        Paths {
            conn,
            cache: HashMap::new(),
        }
    }

    pub fn remember(&mut self, id: i64, path: Vec<u8>) {
        self.cache.insert(id, path);
    }

    pub fn path(&mut self, id: i64) -> Result<Vec<u8>> {
        if let Some(p) = self.cache.get(&id) {
            return Ok(p.clone());
        }
        let mut st = self
            .conn
            .prepare_cached("SELECT parent_id, name FROM entries WHERE id = ?1")?;
        let mut chain: Vec<(i64, Vec<u8>)> = Vec::new();
        let mut cur = id;
        let mut base: Vec<u8> = Vec::new();
        loop {
            if let Some(p) = self.cache.get(&cur) {
                base = p.clone();
                break;
            }
            let (parent, name): (Option<i64>, Vec<u8>) = st
                .query_row(params![cur], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?
                .ok_or_else(|| Error::generic(format!("entry {cur} missing from the snapshot")))?;
            chain.push((cur, name));
            match parent {
                Some(p) => cur = p,
                None => break,
            }
        }
        // chain is child → ancestor; the last element is a root (its name is the full path)
        let mut path = base;
        while let Some((cid, name)) = chain.pop() {
            if path.is_empty() {
                path = name;
            } else {
                push_component(&mut path, &name);
            }
            self.cache.insert(cid, path.clone());
        }
        Ok(path)
    }
}

/// Lexical normalization: absolute, no `.`/`..`/empty components, no trailing slash.
pub fn normalize(input: &[u8]) -> Result<Vec<u8>> {
    if input.first() != Some(&b'/') {
        return Err(Error::usage(format!(
            "path must be absolute: {:?}",
            String::from_utf8_lossy(input)
        )));
    }
    let mut parts: Vec<&[u8]> = Vec::new();
    for c in input.split(|&b| b == b'/') {
        match c {
            b"" | b"." => {}
            b".." => {
                parts.pop();
            }
            _ => parts.push(c),
        }
    }
    let mut out = Vec::with_capacity(input.len());
    for p in parts {
        out.push(b'/');
        out.extend_from_slice(p);
    }
    if out.is_empty() {
        out.push(b'/');
    }
    Ok(out)
}

fn under(root: &[u8], p: &[u8]) -> bool {
    root == b"/" || p == root || (p.starts_with(root) && p.get(root.len()) == Some(&b'/'))
}

#[derive(Debug, Clone)]
pub struct RootRow {
    pub id: i64,
    pub end: i64,
    pub path: Vec<u8>,
}

pub fn roots(conn: &Connection) -> Result<Vec<RootRow>> {
    let mut st = conn.prepare_cached(
        "SELECT r.entry_id, e.subtree_end, r.path FROM roots r JOIN entries e ON e.id = r.entry_id ORDER BY r.entry_id",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok(RootRow {
                id: r.get(0)?,
                end: r.get(1)?,
                path: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Find the entry for `path` inside the root with the longest matching prefix,
/// by `path_hash`, verifying the reconstructed path.
pub fn find_path(conn: &Connection, path: &[u8]) -> Result<Option<Row>> {
    let rs = roots(conn)?;
    let Some(root) = rs
        .iter()
        .filter(|r| under(&r.path, path))
        .max_by_key(|r| r.path.len())
    else {
        return Ok(None);
    };
    if path == root.path.as_slice() {
        return get_row(conn, root.id);
    }
    let mut st = conn.prepare_cached(&format!(
        "SELECT {COLS} FROM entries WHERE path_hash = ?1 AND id BETWEEN ?2 AND ?3"
    ))?;
    let cands = st
        .query_map(params![path_hash(path), root.id, root.end], Row::from_sql)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut paths = Paths::new(conn);
    for c in cands {
        if paths.path(c.id)? == path {
            return Ok(Some(c));
        }
    }
    Ok(None)
}

pub fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// `path_raw` is present only when the UTF-8 rendering loses bytes.
pub fn raw_if_lossy(b: &[u8]) -> Option<String> {
    if std::str::from_utf8(b).is_ok() {
        None
    } else {
        Some(hex_encode(b))
    }
}

/// Render a path as one safe POSIX shell word when it can be pasted intact.
pub fn shell_path_word(path: &[u8]) -> Option<String> {
    let path = std::str::from_utf8(path).ok()?;
    if path.chars().any(char::is_control) {
        return None;
    }
    Some(format!("'{}'", path.replace('\'', "'\\''")))
}

/// Render a path as safe POSIX shell arguments for a copyable CLI hint.
/// Paths that cannot be pasted intact travel through `--path-raw`.
pub fn shell_path_args(path: &[u8]) -> String {
    shell_path_word(path).unwrap_or_else(|| format!("--path-raw {}", hex_encode(path)))
}

/// Resolve user input to an entry; PATH_NOT_FOUND names the nearest ancestor that exists.
pub fn resolve(snap: &Snapshot, input: &[u8]) -> Result<(Row, Vec<u8>)> {
    let path = normalize(input)?;
    if let Some(r) = find_path(&snap.conn, &path)? {
        return Ok((r, path));
    }
    let rs = roots(&snap.conn)?;
    let shown = lossy(&path);
    if !rs.iter().any(|r| under(&r.path, &path)) {
        let names: Vec<String> = rs.iter().map(|r| lossy(&r.path)).collect();
        return Err(Error::new(
            ErrorCode::PathNotFound,
            format!(
                "{shown} is outside the scanned roots ({})",
                names.join(", ")
            ),
        )
        .with_fix(format!("use a path under {}", names.join(" or "))));
    }
    let mut anc = path.clone();
    while let Some(pos) = anc.iter().rposition(|&b| b == b'/') {
        anc.truncate(pos.max(1));
        if find_path(&snap.conn, &anc)?.is_some() {
            break;
        }
        if pos <= 1 {
            break;
        }
    }
    Err(Error::new(
        ErrorCode::PathNotFound,
        format!(
            "{shown} is not in the snapshot from {} {} (created later, removed, or on another filesystem)",
            crate::format::local_date(snap.meta.scanned_at),
            crate::format::local_hm(snap.meta.scanned_at)
        ),
    )
        .with_fix(format!("freespace ls {}", shell_path_args(&anc))))
}

/// Topmost DENIED directories of the whole snapshot (D13: any DENIED is an anomaly).
pub fn unknown_subtrees(conn: &Connection, cap: usize) -> Result<(bool, Vec<String>)> {
    let mut st = conn.prepare_cached(
        "SELECT d.entry_id, e.subtree_end FROM denied d JOIN entries e ON e.id = d.entry_id ORDER BY d.entry_id",
    )?;
    let rows = st
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Ok((false, Vec::new()));
    }
    let mut paths = Paths::new(conn);
    let mut out = Vec::new();
    let mut end = -1i64;
    for (id, sub_end) in rows {
        if id <= end {
            continue;
        }
        end = sub_end;
        if out.len() < cap {
            out.push(lossy(&paths.path(id)?));
        }
    }
    Ok((true, out))
}

pub fn header(snap: &Snapshot, focus: Option<&Row>) -> Result<Header> {
    let (incomplete, unknown) = unknown_subtrees(&snap.conn, 100)?;
    let oldest = focus
        .map(|r| r.min_scanned_at)
        .unwrap_or(snap.meta.scanned_at);
    Ok(Header {
        host: snap.meta.host.clone(),
        os: snap.meta.os.clone(),
        generation: snap.meta.generation,
        prev_generation: snap.meta.prev_generation,
        scanned_at: snap.meta.scanned_at,
        age_seconds: (now_secs() - oldest).max(0),
        schema_version: snap.meta.schema_version,
        snapshot_id: snap.meta.snapshot_id.clone(),
        incomplete,
        unknown_subtrees: unknown,
    })
}

pub fn check_snapshot_id(snap: &Snapshot, want: Option<&str>) -> Result<()> {
    match want {
        Some(id) if !id.is_empty() && id != snap.meta.snapshot_id => Err(Error::new(
            ErrorCode::Rotated,
            format!(
                "snapshot {id} was replaced by {} (generation {})",
                snap.meta.snapshot_id, snap.meta.generation
            ),
        )
        .with_fix("reload: the new snapshot is ready")),
        _ => Ok(()),
    }
}

pub fn entry_out(row: &Row, path: &[u8]) -> EntryOut {
    EntryOut {
        path: lossy(path),
        path_raw: raw_if_lossy(path),
        name: lossy(&row.name),
        kind: row.kind(),
        allocated: row.allocated.max(0) as u64,
        apparent: row.apparent.max(0) as u64,
        mtime: row.mtime,
        newest_mtime: row.newest_mtime,
        nlink: row.nlink.max(0) as u64,
        items: row.items.max(0) as u64,
        errors: row.errors.max(0) as u64,
        scanned_at: row.scanned_at,
        denied: row.flags & flags::DENIED != 0,
        hardlink_alias: row.flags & flags::ALIAS != 0,
        hardlink_owner: None,
        mount: None,
    }
}

/// Mount details for kind = mount rows.
pub fn mount_out(conn: &Connection, id: i64) -> Result<Option<MountOut>> {
    let mut st = conn.prepare_cached(
        "SELECT path, fs_type, fs_used, fs_total FROM mounts WHERE entry_id = ?1",
    )?;
    let m = st
        .query_row(params![id], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })
        .optional()?;
    let Some((path, fs_type, used, total)) = m else {
        return Ok(None);
    };
    let root = roots(conn)?
        .into_iter()
        .find(|r| r.path == path)
        .map(|r| lossy(&r.path));
    Ok(Some(MountOut {
        fs_type,
        used: used.map(|v| v as u64),
        total: total.map(|v| v as u64),
        root,
    }))
}

/// Owner path of a hardlinked entry (the link that carries the size).
pub fn hardlink_owner(conn: &Connection, paths: &mut Paths<'_>, id: i64) -> Result<Option<String>> {
    let mut st = conn.prepare_cached(
        "SELECT h2.entry_id FROM hardlinks h1 JOIN hardlinks h2 ON h2.dev = h1.dev AND h2.ino = h1.ino
         JOIN entries e ON e.id = h2.entry_id
         WHERE h1.entry_id = ?1 AND (e.flags & 2) = 0 LIMIT 1",
    )?;
    let owner: Option<i64> = st.query_row(params![id], |r| r.get(0)).optional()?;
    match owner {
        Some(o) => Ok(Some(lossy(&paths.path(o)?))),
        None => Ok(None),
    }
}

/// Decorate rows for output: mount info and hardlink owners.
pub fn decorate(
    conn: &Connection,
    paths: &mut Paths<'_>,
    row: &Row,
    out: &mut EntryOut,
) -> Result<()> {
    if row.kind == Kind::Mount.code() {
        out.mount = mount_out(conn, row.id)?;
    }
    if row.nlink > 1 && !row.is_dir() {
        out.hardlink_owner = hardlink_owner(conn, paths, row.id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize(b"/").unwrap(), b"/");
        assert_eq!(normalize(b"/home//tass/").unwrap(), b"/home/tass");
        assert_eq!(normalize(b"/home/./tass/../x").unwrap(), b"/home/x");
        assert_eq!(normalize(b"/..").unwrap(), b"/");
        assert!(normalize(b"home").is_err());
    }

    #[test]
    fn shell_path_args_quote_utf8_and_preserve_raw_bytes() {
        assert_eq!(shell_path_args(b"/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_path_args(b"/tmp/a'b"), "'/tmp/a'\\''b'");
        assert_eq!(shell_path_args(b"/tmp/\xff"), "--path-raw 2f746d702fff");
        assert_eq!(shell_path_args(b"/tmp/a\tb"), "--path-raw 2f746d702f610962");
        assert_eq!(shell_path_word(b"/tmp/a b").as_deref(), Some("'/tmp/a b'"));
        assert_eq!(shell_path_word(b"/tmp/\xff"), None);
        assert_eq!(shell_path_word(b"/tmp/a\nb"), None);
    }

    #[test]
    fn under_roots() {
        assert!(under(b"/", b"/boot/efi"));
        assert!(under(b"/boot/efi", b"/boot/efi/EFI"));
        assert!(under(b"/boot/efi", b"/boot/efi"));
        assert!(!under(b"/boot/efi", b"/boot/efix"));
    }
}
