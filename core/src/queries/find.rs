//! `find`: every match under a path with a total over all of them (D15).
//! The pass (`super::pass`) marks a match inside an already matched directory
//! `nested` (counted, not summed) and keeps the page in a bounded heap.

use super::common::{
    Paths, Row, check_snapshot_id, decorate, entry_out, get_row, header, lossy, raw_if_lossy,
    resolve,
};
use super::glob;
use super::params::{
    FindParams, KindFilter, PathArg, SortKey, check_limit, parse_size, parse_time_bound,
};
use super::pass::{self, Filter, Order};
use super::types::{FindEntry, FindResponse};
use crate::error::{Error, Result};
use crate::platform::now_secs;
use crate::snapshot::reader::Snapshot;

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

    let filter = Filter {
        kind,
        min_size: i64::try_from(min_size.unwrap_or(0)).unwrap_or(i64::MAX),
        older,
        newer,
        name: pattern.map(|p| (p, icase)),
    };
    let order = Order {
        sort,
        limit,
        list_nested: true,
    };
    let pass::Pass { total, page } =
        pass::run(snap, (scope.id, scope.subtree_end), &filter, &order)?;

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
