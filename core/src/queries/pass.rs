//! The filtered pass `top` and `find` share: every row under a path that
//! passes the filters, in `id` order, with a total over all of them and a
//! bounded page. A match inside an already matched directory is `nested`
//! (counted, not summed, D15).
//!
//! One thread reads ~7M rows/s warm, so a wide subtree is split into id ranges
//! read in parallel, each on its own connection to the same file. A range
//! starts with the matched directories it lies in (they are ancestors of its
//! first row and live in earlier ranges), so `nested` is exact at the seams.
//! A `--min-size` that few entries pass reads them through `entries_alloc`
//! instead (docs/benchmarks.md: 2.5k rows take 0.04 s warm, 0.5 s cold).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use rusqlite::{Connection, OptionalExtension, params};

use super::params::{KindFilter, SortKey};
use super::types::Total;
use crate::error::Result;
use crate::snapshot::reader::{self, Snapshot};

/// Rows per range below which another thread is not worth a connection.
const RANGE_ROWS: i64 = 65_536;
const MAX_THREADS: usize = 8;
/// Above this many entries over `--min-size`, random reads through the index
/// cost more than a sequential pass on a cold cache.
const INDEX_ROWS: i64 = 8192;

pub struct Filter {
    pub kind: KindFilter,
    pub min_size: i64,
    /// Dates compare `newest_mtime` for directories, `mtime` otherwise (D2).
    pub older: Option<i64>,
    pub newer: Option<i64>,
    /// Glob over the entry name and whether it ignores case.
    pub name: Option<(String, bool)>,
}

pub struct Order {
    pub sort: SortKey,
    pub limit: u32,
    /// false: nested matches are counted but never listed (`top --kind dir`).
    pub list_nested: bool,
}

/// Heap item: the "largest" item is the one to evict (worst in sort order).
#[derive(Debug, PartialEq, Eq)]
pub struct Ranked {
    pub key: i64,
    pub id: i64,
    pub nested: bool,
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

#[derive(Debug, PartialEq, Eq)]
pub struct Pass {
    pub total: Total,
    /// The best `limit` matches in sort order.
    pub page: Vec<Ranked>,
}

fn sql(f: &Filter, index: &str) -> String {
    let mut sql = format!(
        "SELECT id, kind, allocated, apparent, mtime, subtree_end, newest_mtime \
         FROM entries {index} WHERE id > ?1 AND id <= ?2"
    );
    match f.kind {
        KindFilter::File => sql.push_str(" AND kind = 0"),
        KindFilter::Dir => sql.push_str(" AND kind = 1"),
        KindFilter::Any => {}
    }
    sql.push_str(" AND allocated >= ?3");
    let date = "(CASE WHEN kind = 1 THEN newest_mtime ELSE mtime END)";
    if f.older.is_some() {
        sql.push_str(&format!(" AND {date} < ?4"));
    } else {
        sql.push_str(" AND ?4 IS NULL");
    }
    if f.newer.is_some() {
        sql.push_str(&format!(" AND {date} >= ?5"));
    } else {
        sql.push_str(" AND ?5 IS NULL");
    }
    if f.name.is_some() {
        sql.push_str(" AND fs_glob(name, ?6, ?7)");
    } else {
        sql.push_str(" AND ?6 IS NULL AND ?7 IS NOT NULL");
    }
    sql.push_str(" ORDER BY id");
    sql
}

/// Matches in `(lo, hi]` of the subtree `scope`.
fn range(
    conn: &Connection,
    sql: &str,
    f: &Filter,
    o: &Order,
    scope: i64,
    lo: i64,
    hi: i64,
) -> Result<Pass> {
    let mut total = Total::default();
    let mut heap: BinaryHeap<Ranked> = BinaryHeap::new();
    if lo >= hi {
        return Ok(Pass {
            total,
            page: Vec::new(),
        });
    }
    let (pattern, icase) = match &f.name {
        Some((p, i)) => (Some(p.as_str()), *i as i64),
        None => (None, 0),
    };
    let mut st = conn.prepare_cached(sql)?;
    // the end of the outermost matched directory this range starts inside
    let mut end = i64::MIN;
    if f.kind != KindFilter::File && lo > scope {
        let mut parent = conn.prepare_cached("SELECT parent_id FROM entries WHERE id = ?1")?;
        let mut cur: Option<i64> = parent.query_row(params![lo + 1], |r| r.get(0))?;
        while let Some(a) = cur
            && a > scope
        {
            let matched: Option<i64> = st
                .query_row(
                    params![a - 1, a, f.min_size, f.older, f.newer, pattern, icase],
                    |r| r.get(5),
                )
                .optional()?;
            end = end.max(matched.unwrap_or(i64::MIN));
            cur = parent.query_row(params![a], |r| r.get(0))?;
        }
    }
    let mut rows = st.query(params![
        lo, hi, f.min_size, f.older, f.newer, pattern, icase
    ])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let kind: i64 = r.get(1)?;
        let allocated: i64 = r.get(2)?;
        let apparent: i64 = r.get(3)?;
        let nested = id <= end;
        total.count += 1;
        if !nested {
            total.counted += 1;
            total.allocated += allocated.max(0) as u64;
            total.apparent += apparent.max(0) as u64;
            if kind == 1 {
                end = r.get(5)?;
            }
        } else if !o.list_nested {
            continue;
        }
        let key = match o.sort {
            SortKey::Allocated => allocated.saturating_neg(),
            SortKey::Apparent => apparent.saturating_neg(),
            SortKey::Mtime if kind == 1 => r.get(6)?,
            SortKey::Mtime => r.get(4)?,
        };
        let it = Ranked { key, id, nested };
        if heap.len() == o.limit as usize && heap.peek().is_some_and(|worst| it >= *worst) {
            continue;
        }
        heap.push(it);
        if heap.len() > o.limit as usize {
            heap.pop();
        }
    }
    Ok(Pass {
        total,
        page: heap.into_vec(),
    })
}

fn threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(MAX_THREADS)
}

/// Whether at most [`INDEX_ROWS`] entries of the whole snapshot have
/// `allocated >= min` (an index-only count that stops there).
fn few_above(conn: &Connection, min: i64) -> Result<bool> {
    let n: i64 = conn
        .prepare_cached(
            "SELECT count(*) FROM (SELECT 1 FROM entries INDEXED BY entries_alloc \
             WHERE allocated >= ?1 LIMIT ?2)",
        )?
        .query_row(params![min, INDEX_ROWS + 1], |r| r.get(0))?;
    Ok(n <= INDEX_ROWS)
}

/// Matches under the entry `scope` (its id and `subtree_end`).
pub fn run(snap: &Snapshot, scope: (i64, i64), f: &Filter, o: &Order) -> Result<Pass> {
    if f.min_size > 0 && few_above(&snap.conn, f.min_size)? {
        let sql = sql(f, "INDEXED BY entries_alloc");
        let part = range(&snap.conn, &sql, f, o, scope.0, scope.0, scope.1);
        return merge(vec![part], o.limit);
    }
    let ranges = ((scope.1 - scope.0) / RANGE_ROWS).clamp(1, threads() as i64);
    run_in(snap, scope, f, o, ranges as usize)
}

/// [`run`] over at most `ranges` id ranges.
pub fn run_in(
    snap: &Snapshot,
    scope: (i64, i64),
    f: &Filter,
    o: &Order,
    ranges: usize,
) -> Result<Pass> {
    let mut conns = Vec::new();
    for _ in 1..ranges {
        // a newer generation published under the same path: read on one connection
        match reader::open_file(&snap.path) {
            Ok(c)
                if reader::read_meta(&c).is_ok_and(|m| m.snapshot_id == snap.meta.snapshot_id) =>
            {
                c.execute_batch("PRAGMA cache_size=-2048")?;
                conns.push(c);
            }
            _ => break,
        }
    }
    let n = conns.len() as i64 + 1;
    let (id, end) = scope;
    let bound = |k: i64| id + (end - id) * k / n;
    let sql = sql(f, "");
    let parts: Vec<Result<Pass>> = std::thread::scope(|s| {
        let rest: Vec<_> = conns
            .into_iter()
            .zip(1..)
            .map(|(c, k)| {
                let sql = &sql;
                s.spawn(move || range(&c, sql, f, o, id, bound(k), bound(k + 1)))
            })
            .collect();
        let first = range(&snap.conn, &sql, f, o, id, bound(0), bound(1));
        std::iter::once(first)
            .chain(
                rest.into_iter()
                    .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e))),
            )
            .collect()
    });
    merge(parts, o.limit)
}

fn merge(parts: Vec<Result<Pass>>, limit: u32) -> Result<Pass> {
    let mut total = Total::default();
    let mut heap: BinaryHeap<Ranked> = BinaryHeap::new();
    for p in parts {
        let p = p?;
        total.count += p.total.count;
        total.counted += p.total.counted;
        total.allocated += p.total.allocated;
        total.apparent += p.total.apparent;
        for r in p.page {
            heap.push(r);
            if heap.len() > limit as usize {
                heap.pop();
            }
        }
    }
    Ok(Pass {
        total,
        page: heap.into_sorted_vec(),
    })
}
