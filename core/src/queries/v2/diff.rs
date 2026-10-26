//! Exact-path merge of two generation streams, with bounded result storage.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use rusqlite::{OptionalExtension, params};

use super::common::{get_path, header, input, name, refreshed, under, upper};
use super::pass::{matches_kind, retain};
use crate::queries::common::{lossy, raw_if_lossy};
use crate::queries::params::{DiffParams, KindFilter, MAX_DEPTH, check_limit};
use crate::queries::types::{DiffEntry, DiffResponse, DiffStatus};
use crate::scan::flags;
use crate::snapshot::v2::{ENTRY_COLUMNS, Entry, Reader};
use crate::{Error, ErrorCode, Result};

struct Ranked {
    entry: DiffEntry,
    name: Vec<u8>,
    traversal_order: Vec<u8>,
    gone: bool,
}

impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .entry
            .delta
            .unsigned_abs()
            .cmp(&self.entry.delta.unsigned_abs())
            .then((other.entry.delta > 0).cmp(&(self.entry.delta > 0)))
            .then(self.name.cmp(&other.name))
            .then(self.gone.cmp(&other.gone))
            .then(self.traversal_order.cmp(&other.traversal_order))
    }
}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Ranked {}

pub fn diff(reader: &Reader, p: &DiffParams) -> Result<DiffResponse> {
    let depth = p.depth.unwrap_or(1);
    if depth == 0 || depth > MAX_DEPTH {
        return Err(Error::usage(format!(
            "depth must be between 1 and {MAX_DEPTH}"
        )));
    }
    let kind = p
        .kind
        .as_deref()
        .map(KindFilter::parse)
        .transpose()?
        .unwrap_or(KindFilter::Dir);
    let limit = check_limit(p.limit)?;
    let path = input(&p.path, &p.path_raw)?;
    reader.with_pin(|reader| diff_pinned(reader, &path, depth, kind, limit))
}

fn diff_pinned(
    reader: &Reader,
    path: &[u8],
    depth: u32,
    kind: KindFilter,
    limit: u32,
) -> Result<DiffResponse> {
    let previous_meta = reader.meta(true).map_err(|error| {
        if error.code == ErrorCode::NoPrev {
            crate::queries::diff::no_prev("v2 store has no previous full scan")
        } else {
            error
        }
    })?;
    let current = get_path(reader, path, false)?;
    let previous = get_path(reader, path, true)?;
    if current.is_none() && previous.is_none() {
        return Err(Error::new(
            ErrorCode::PathNotFound,
            format!(
                "{} is in neither the current nor the previous snapshot",
                lossy(path)
            ),
        )
        .with_fix("freespace ls /"));
    }
    let (total_count, entries) = merge_levels(
        reader,
        current.as_ref(),
        previous.as_ref(),
        depth,
        kind,
        limit,
    )?;
    let refreshed = refreshed(reader)?;
    let mut current_roots: Vec<_> = reader.roots(false)?.into_iter().map(|r| r.path).collect();
    let mut previous_roots: Vec<_> = reader.roots(true)?.into_iter().map(|r| r.path).collect();
    current_roots.sort();
    previous_roots.sort();
    Ok(DiffResponse {
        header: header(reader, current.as_ref())?,
        path: lossy(path),
        path_raw: raw_if_lossy(path),
        prev_scanned_at: previous_meta.scanned_at,
        depth,
        kind: kind.as_str().into(),
        partial: !refreshed.is_empty(),
        refreshed,
        roots_changed: current_roots != previous_roots,
        delta: current.as_ref().map_or(0, |e| e.allocated)
            - previous.as_ref().map_or(0, |e| e.allocated),
        truncated: total_count > entries.len() as u64,
        entries,
        total_count,
        limit,
    })
}

fn merge_levels(
    reader: &Reader,
    current: Option<&Entry>,
    previous: Option<&Entry>,
    depth: u32,
    kind: KindFilter,
    limit: u32,
) -> Result<(u64, Vec<DiffEntry>)> {
    let mut current_query = reader.connection().prepare(&level_sql(depth))?;
    let mut previous_query = reader.connection().prepare(&level_sql(depth))?;
    let current_bound = bounds(current);
    let previous_bound = bounds(previous);
    let mut current_rows = current_query.query(params![
        current_bound.0,
        current_bound.1,
        current_bound.2,
        reader.generation(false)?,
        current.map(|e| e.id)
    ])?;
    let mut previous_rows = previous_query.query(params![
        previous_bound.0,
        previous_bound.1,
        previous_bound.2,
        reader.generation(true)?,
        previous.map(|e| e.id)
    ])?;
    let target_depth =
        components(&current.or(previous).expect("one scope is present").path) + depth as usize;
    let mut c = next_level(&mut current_rows, target_depth, kind)?;
    let mut p = next_level(&mut previous_rows, target_depth, kind)?;
    let mut heap = BinaryHeap::new();
    let mut total_count = 0;
    while c.is_some() || p.is_some() {
        let order = match (&c, &p) {
            (Some(c), Some(p)) => c.sort_key.cmp(&p.sort_key),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => unreachable!(),
        };
        let (current_entry, previous_entry) = match order {
            Ordering::Equal => (c.take(), p.take()),
            Ordering::Less => (c.take(), None),
            Ordering::Greater => (None, p.take()),
        };
        let suppressed = if current_entry.is_none() {
            if let (Some(scope), Some(old)) = (current, previous_entry.as_ref()) {
                unknown_previous(reader, scope, old)?
            } else {
                false
            }
        } else {
            false
        };
        if !suppressed {
            total_count += 1;
            retain(
                &mut heap,
                ranked(current_entry.as_ref(), previous_entry.as_ref()),
                limit,
            );
        }
        if order != Ordering::Greater {
            c = next_level(&mut current_rows, target_depth, kind)?;
        }
        if order != Ordering::Less {
            p = next_level(&mut previous_rows, target_depth, kind)?;
        }
    }
    let entries: Vec<_> = heap
        .into_sorted_vec()
        .into_iter()
        .map(|r| r.entry)
        .collect();
    Ok((total_count, entries))
}

fn level_sql(depth: u32) -> String {
    if depth == 1 {
        format!(
            "SELECT {ENTRY_COLUMNS} FROM versions WHERE parent=?5 AND born<=?4 AND (dead IS NULL OR dead>?4) AND scope=?1 AND sort_key>?2 AND sort_key<?3 ORDER BY sort_key"
        )
    } else {
        // Index-ordered streaming avoids a recursive CTE's potentially unbounded
        // breadth queue or a sort/materialization of all requested-depth rows.
        format!(
            "SELECT {ENTRY_COLUMNS} FROM versions WHERE scope=?1 AND sort_key>?2 AND sort_key<?3 AND born<=?4 AND (dead IS NULL OR dead>?4) AND ?5 IS NOT NULL ORDER BY sort_key"
        )
    }
}

fn bounds(entry: Option<&Entry>) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    entry
        .map(|e| (e.scope.clone(), e.sort_key.clone(), upper(&e.sort_key)))
        .unwrap_or_default()
}

fn components(path: &[u8]) -> usize {
    path.split(|b| *b == b'/').filter(|p| !p.is_empty()).count()
}

fn next_level(
    rows: &mut rusqlite::Rows<'_>,
    depth: usize,
    kind: KindFilter,
) -> Result<Option<Entry>> {
    while let Some(row) = rows.next()? {
        let entry = Entry::row(row)?;
        if components(&entry.path) == depth && matches_kind(entry.kind, kind) {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

/// Gone rows below a currently denied directory are unknown, not deleted.
/// Check ancestors with exact scoped lookups, keeping memory bounded by depth.
fn unknown_previous(reader: &Reader, scope: &Entry, old: &Entry) -> Result<bool> {
    let mut path = old.path.clone();
    let mut query = reader.connection().prepare_cached(
        "SELECT flags FROM versions WHERE scope=?1 AND path=?2 AND born<=?3 AND (dead IS NULL OR dead>?3)")?;
    while under(&scope.path, &path) {
        let flags: Option<u8> = query
            .query_row(params![scope.scope, path, reader.generation(false)?], |r| {
                r.get(0)
            })
            .optional()?;
        if flags.is_some_and(|f| f & flags::DENIED != 0)
            && reader
                .get(&path, true)?
                .is_some_and(|previous| previous.scope == old.scope)
        {
            return Ok(true);
        }
        if path == scope.path || path == b"/" {
            break;
        }
        let pos = path
            .iter()
            .rposition(|b| *b == b'/')
            .expect("absolute path");
        path.truncate(pos.max(1));
    }
    Ok(false)
}

fn ranked(current: Option<&Entry>, previous: Option<&Entry>) -> Ranked {
    let source = current.or(previous).expect("one side is present");
    let allocated = current.map_or(0, |e| e.allocated.max(0) as u64);
    let allocated_prev = previous.map_or(0, |e| e.allocated.max(0) as u64);
    let items = current.map_or(0, |e| e.items.max(0) as u64);
    let items_prev = previous.map_or(0, |e| e.items.max(0) as u64);
    let delta = allocated as i64 - allocated_prev as i64;
    let status = match (current, previous) {
        (Some(_), None) => DiffStatus::New,
        (None, Some(_)) => DiffStatus::Gone,
        _ if delta != 0 || items != items_prev => DiffStatus::Changed,
        _ => DiffStatus::Same,
    };
    Ranked {
        name: name(source).to_vec(),
        traversal_order: source.sort_key.clone(),
        gone: current.is_none(),
        entry: DiffEntry {
            path: lossy(&source.path),
            path_raw: raw_if_lossy(&source.path),
            name: lossy(name(source)),
            kind: source.kind,
            status,
            allocated,
            allocated_prev,
            delta,
            items,
            items_prev,
            mtime: source.mtime,
            newest_mtime: source.newest_mtime.unwrap_or(0),
        },
    }
}
