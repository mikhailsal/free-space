//! `ls`: children of one directory, sorted, limited.

use rusqlite::params;

use super::common::{COLS, Paths, Row, check_snapshot_id, decorate, entry_out, header, resolve};
use super::params::{LsParams, PathArg, SortKey, check_limit};
use super::types::LsResponse;
use crate::error::Result;
use crate::scan::tree::push_component;
use crate::snapshot::reader::Snapshot;

pub fn order_by(sort: SortKey) -> &'static str {
    match sort {
        SortKey::Allocated => "allocated DESC, id",
        SortKey::Apparent => "apparent DESC, id",
        SortKey::Mtime => "mtime ASC, id",
    }
}

pub fn ls(snap: &Snapshot, p: &LsParams) -> Result<LsResponse> {
    check_snapshot_id(snap, p.snapshot.as_deref())?;
    let sort = match &p.sort {
        Some(s) => SortKey::parse(s)?,
        None => SortKey::Allocated,
    };
    let limit = check_limit(p.limit)?;
    let input = PathArg::new(p.path.clone(), p.path_raw.clone()).bytes()?;
    let (dir, path) = resolve(snap, &input)?;
    let conn = &snap.conn;
    let total_count: i64 = conn
        .prepare_cached("SELECT count(*) FROM entries WHERE parent_id = ?1")?
        .query_row(params![dir.id], |r| r.get(0))?;
    let mut st = conn.prepare_cached(&format!(
        "SELECT {COLS} FROM entries WHERE parent_id = ?1 ORDER BY {} LIMIT ?2",
        order_by(sort)
    ))?;
    let rows = st
        .query_map(params![dir.id, limit as i64], Row::from_sql)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut paths = Paths::new(conn);
    paths.remember(dir.id, path.clone());
    let mut entries = Vec::with_capacity(rows.len());
    for r in &rows {
        let mut cp = path.clone();
        push_component(&mut cp, &r.name);
        let mut e = entry_out(r, &cp);
        decorate(conn, &mut paths, r, &mut e)?;
        entries.push(e);
    }
    let mut entry = entry_out(&dir, &path);
    decorate(conn, &mut paths, &dir, &mut entry)?;
    Ok(LsResponse {
        header: header(snap, Some(&dir))?,
        entry,
        entries,
        total_count: total_count as u64,
        truncated: (total_count as u64) > limit as u64,
        limit,
        sort: sort.as_str().into(),
    })
}
