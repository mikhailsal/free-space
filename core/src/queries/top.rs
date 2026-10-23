//! `top`: largest files, or topmost qualifying directories, under a path.

use rusqlite::params;

use super::common::{COLS, Paths, Row, decorate, entry_out, header, lossy, raw_if_lossy, resolve};
use super::params::{
    KindFilter, PathArg, SortKey, TopParams, check_limit, parse_size, parse_time_bound,
};
use super::types::{TopResponse, Total};
use crate::error::{Error, Result};
use crate::platform::now_secs;
use crate::snapshot::reader::Snapshot;

fn sort_rows(rows: &mut [Row], sort: SortKey) {
    match sort {
        SortKey::Allocated => {
            rows.sort_by(|a, b| b.allocated.cmp(&a.allocated).then(a.id.cmp(&b.id)))
        }
        SortKey::Apparent => rows.sort_by(|a, b| b.apparent.cmp(&a.apparent).then(a.id.cmp(&b.id))),
        SortKey::Mtime => {
            rows.sort_by(|a, b| a.filter_date().cmp(&b.filter_date()).then(a.id.cmp(&b.id)))
        }
    }
}

pub fn top(snap: &Snapshot, p: &TopParams) -> Result<TopResponse> {
    let kind = match &p.kind {
        Some(k) => KindFilter::parse(k)?,
        None => KindFilter::File,
    };
    if kind == KindFilter::Any {
        return Err(Error::usage("top takes --kind file or --kind dir"));
    }
    let sort = match &p.sort {
        Some(s) => SortKey::parse(s)?,
        None => SortKey::Allocated,
    };
    let limit = check_limit(p.limit)?;
    let min_size = p.min_size.as_deref().map(parse_size).transpose()?;
    let cut = p
        .older_than
        .as_deref()
        .map(|s| parse_time_bound(s, now_secs()))
        .transpose()?;
    let input = PathArg::new(p.path.clone(), p.path_raw.clone()).bytes()?;
    let (scope, path) = resolve(snap, &input)?;
    let conn = &snap.conn;
    let min = i64::try_from(min_size.unwrap_or(0)).unwrap_or(i64::MAX);

    // gstack-shortcut(dec-ac91ac89): unknown/unreadable dates pass --older-than, upgrade when a
    // running-service dir is listed as untouched or mtime=0 appears in a real snapshot.
    // (mtime = 0 and directories with DENIED children are not excluded; `incomplete` is the signal.)
    let (rows, total) = if kind == KindFilter::File {
        let date_filter = if cut.is_some() {
            "AND mtime < ?4"
        } else {
            "AND ?4 IS NULL"
        };
        let order = match sort {
            SortKey::Allocated => "allocated DESC, id",
            SortKey::Apparent => "apparent DESC, id",
            SortKey::Mtime => "mtime ASC, id",
        };
        let filter = format!("id > ?1 AND id <= ?2 AND kind = 0 AND allocated >= ?3 {date_filter}");
        let mut st = conn.prepare_cached(&format!(
            "SELECT {COLS} FROM entries WHERE {filter} ORDER BY {order} LIMIT ?5"
        ))?;
        let rows = st
            .query_map(
                params![scope.id, scope.subtree_end, min, cut, limit as i64],
                Row::from_sql,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let (count, alloc, app): (i64, i64, i64) = conn
            .prepare_cached(&format!(
                "SELECT count(*), coalesce(sum(allocated), 0), coalesce(sum(apparent), 0) FROM entries WHERE {filter}"
            ))?
            .query_row(params![scope.id, scope.subtree_end, min, cut], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        let total = Total {
            count: count as u64,
            counted: count as u64,
            allocated: alloc as u64,
            apparent: app as u64,
        };
        (rows, total)
    } else {
        // Directories: qualifying set in id order, topmost only (a qualifying dir
        // implies its qualifying descendants), totals over the topmost set, then sort/limit.
        let date_filter = if cut.is_some() {
            "AND newest_mtime < ?4"
        } else {
            "AND ?4 IS NULL"
        };
        let mut st = conn.prepare_cached(&format!(
            "SELECT {COLS} FROM entries WHERE id > ?1 AND id <= ?2 AND kind = 1 AND allocated >= ?3 {date_filter} ORDER BY id"
        ))?;
        let mut topmost = Vec::new();
        let mut end = i64::MIN;
        let mut it = st.query_map(
            params![scope.id, scope.subtree_end, min, cut],
            Row::from_sql,
        )?;
        for r in it.by_ref() {
            let r = r?;
            if r.id <= end {
                continue;
            }
            end = r.subtree_end;
            topmost.push(r);
        }
        let total = Total {
            count: topmost.len() as u64,
            counted: topmost.len() as u64,
            allocated: topmost.iter().map(|r| r.allocated.max(0) as u64).sum(),
            apparent: topmost.iter().map(|r| r.apparent.max(0) as u64).sum(),
        };
        sort_rows(&mut topmost, sort);
        topmost.truncate(limit as usize);
        (topmost, total)
    };

    let mut paths = Paths::new(conn);
    paths.remember(scope.id, path.clone());
    let mut entries = Vec::with_capacity(rows.len());
    for r in &rows {
        let rp = paths.path(r.id)?;
        let mut e = entry_out(r, &rp);
        decorate(conn, &mut paths, r, &mut e)?;
        entries.push(e);
    }
    Ok(TopResponse {
        header: header(snap, Some(&scope))?,
        path: lossy(&path),
        path_raw: raw_if_lossy(&path),
        kind: kind.as_str().into(),
        min_size,
        older_than: cut,
        sort: sort.as_str().into(),
        truncated: total.count > entries.len() as u64,
        entries,
        total_count: total.count,
        total_allocated: total.allocated,
        total,
        limit,
    })
}
