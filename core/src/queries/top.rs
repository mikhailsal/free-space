//! `top`: largest files, or topmost qualifying directories, under a path.

use super::common::{Paths, decorate, entry_out, get_row, header, lossy, raw_if_lossy, resolve};
use super::params::{
    KindFilter, PathArg, SortKey, TopParams, check_limit, parse_size, parse_time_bound,
};
use super::pass::{self, Filter, Order};
use super::types::{TopResponse, Total};
use crate::error::{Error, Result};
use crate::platform::now_secs;
use crate::snapshot::reader::Snapshot;

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
    // Directories compare newest_mtime and only the topmost qualifying ones count
    // and are listed (a qualifying dir implies its qualifying descendants); files
    // never nest.
    let filter = Filter {
        kind,
        min_size: min,
        older: cut,
        newer: None,
        name: None,
    };
    let order = Order {
        sort,
        limit,
        list_nested: false,
    };
    let found = pass::run(snap, (scope.id, scope.subtree_end), &filter, &order)?;
    let total = Total {
        count: found.total.counted,
        ..found.total
    };
    let mut rows = Vec::with_capacity(found.page.len());
    for it in &found.page {
        rows.push(get_row(conn, it.id)?.ok_or_else(|| Error::generic("entry vanished"))?);
    }

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
