//! `find`: every match under a path with a total over all of them (D15).
//! Matches are streamed in id order; a match inside an already matched
//! directory is `nested` (counted, not summed). The page is a bounded heap.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use rusqlite::params;

use super::common::{
    COLS, Paths, Row, check_snapshot_id, decorate, entry_out, get_row, header, lossy, raw_if_lossy,
    resolve,
};
use super::glob;
use super::params::{
    FindParams, KindFilter, PathArg, SortKey, check_limit, parse_size, parse_time_bound,
};
use super::types::{FindEntry, FindResponse, Total};
use crate::error::{Error, Result};
use crate::platform::now_secs;
use crate::snapshot::reader::Snapshot;

/// Heap item: the "largest" item is the one to evict (worst in sort order).
#[derive(Debug, PartialEq, Eq)]
struct Ranked {
    key: i64,
    id: i64,
    nested: bool,
}

impl Ord for Ranked {
    fn cmp(&self, o: &Self) -> Ordering {
        // smaller key = better; ties: smaller id better
        self.key.cmp(&o.key).then(self.id.cmp(&o.id))
    }
}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

pub fn find(snap: &Snapshot, p: &FindParams) -> Result<FindResponse> {
    check_snapshot_id(snap, p.snapshot.as_deref())?;
    let now = now_secs();
    let older = p
        .older_than
        .as_deref()
        .map(|s| parse_time_bound(s, now))
        .transpose()?;
    let newer = p
        .newer_than
        .as_deref()
        .map(|s| parse_time_bound(s, now))
        .transpose()?;
    let kind = match &p.kind {
        Some(k) => KindFilter::parse(k)?,
        // date bounds imply files unless asked otherwise (like top --older-than)
        None if older.is_some() || newer.is_some() => KindFilter::File,
        None => KindFilter::Any,
    };
    let sort = match &p.sort {
        Some(s) => SortKey::parse(s)?,
        None => SortKey::Allocated,
    };
    if sort == SortKey::Apparent {
        return Err(Error::usage("find sorts by allocated or mtime"));
    }
    let limit = check_limit(p.limit)?;
    let min_size = p.min_size.as_deref().map(parse_size).transpose()?;
    let (pattern, icase) = match (&p.name, &p.iname) {
        (Some(_), Some(_)) => return Err(Error::usage("give either --name or --iname, not both")),
        (Some(n), None) => (Some(n.clone()), false),
        (None, Some(n)) => (Some(n.clone()), true),
        (None, None) => (None, false),
    };
    if let Some(pat) = &pattern {
        glob::validate(pat)?;
    }
    let input = PathArg::new(p.path.clone(), p.path_raw.clone()).bytes()?;
    let (scope, path) = resolve(snap, &input)?;
    let conn = &snap.conn;

    let mut sql = format!("SELECT {COLS} FROM entries WHERE id > ?1 AND id <= ?2");
    match kind {
        KindFilter::File => sql.push_str(" AND kind = 0"),
        KindFilter::Dir => sql.push_str(" AND kind = 1"),
        KindFilter::Any => {}
    }
    sql.push_str(" AND allocated >= ?3");
    // directories compare newest_mtime, everything else mtime (D2)
    let date = "(CASE WHEN kind = 1 THEN newest_mtime ELSE mtime END)";
    if older.is_some() {
        sql.push_str(&format!(" AND {date} < ?4"));
    } else {
        sql.push_str(" AND ?4 IS NULL");
    }
    if newer.is_some() {
        sql.push_str(&format!(" AND {date} >= ?5"));
    } else {
        sql.push_str(" AND ?5 IS NULL");
    }
    if pattern.is_some() {
        sql.push_str(" AND fs_glob(name, ?6, ?7)");
    } else {
        sql.push_str(" AND ?6 IS NULL AND ?7 IS NOT NULL");
    }
    sql.push_str(" ORDER BY id");

    let mut st = conn.prepare(&sql)?;
    let mut rows = st.query(params![
        scope.id,
        scope.subtree_end,
        i64::try_from(min_size.unwrap_or(0)).unwrap_or(i64::MAX),
        older,
        newer,
        pattern,
        icase as i64
    ])?;
    let mut total = Total::default();
    let mut heap: BinaryHeap<Ranked> = BinaryHeap::new();
    let mut end = i64::MIN;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let kind_code: i64 = r.get(3)?;
        let allocated: i64 = r.get(5)?;
        let apparent: i64 = r.get(6)?;
        let mtime: i64 = r.get(7)?;
        let sub_end: i64 = r.get(12)?;
        let newest: i64 = r.get(14)?;
        let nested = id <= end;
        total.count += 1;
        if !nested {
            total.counted += 1;
            total.allocated += allocated.max(0) as u64;
            total.apparent += apparent.max(0) as u64;
            if kind_code == 1 {
                end = sub_end;
            }
        }
        let key = match sort {
            SortKey::Mtime => {
                if kind_code == 1 {
                    newest
                } else {
                    mtime
                }
            }
            _ => -allocated,
        };
        heap.push(Ranked { key, id, nested });
        if heap.len() > limit as usize {
            heap.pop();
        }
    }
    drop(rows);
    let mut page = heap.into_sorted_vec();
    page.truncate(limit as usize);

    let mut paths = Paths::new(conn);
    paths.remember(scope.id, path.clone());
    let mut entries = Vec::with_capacity(page.len());
    for it in &page {
        let row: Row = get_row(conn, it.id)?.ok_or_else(|| Error::generic("entry vanished"))?;
        let rp = paths.path(row.id)?;
        let mut e = entry_out(&row, &rp);
        decorate(conn, &mut paths, &row, &mut e)?;
        entries.push(FindEntry {
            entry: e,
            nested: it.nested,
        });
    }
    Ok(FindResponse {
        header: header(snap, Some(&scope))?,
        path: lossy(&path),
        path_raw: raw_if_lossy(&path),
        kind: kind.as_str().into(),
        sort: sort.as_str().into(),
        truncated: total.count > entries.len() as u64,
        entries,
        total_count: total.count,
        total_allocated: total.allocated,
        total,
        limit,
    })
}
