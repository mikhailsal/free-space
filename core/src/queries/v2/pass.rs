//! Streaming filtered pass with a page bounded by the requested limit.
//! Subtree membership and nesting use component keys, never stable ID ranges.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use rusqlite::params;

use super::common::{name, upper};
use crate::Result;
use crate::queries::glob;
use crate::queries::params::{KindFilter, SortKey};
use crate::queries::pass::{Filter, Order};
use crate::queries::types::Total;
use crate::scan::Kind;
use crate::snapshot::v2::{ENTRY_COLUMNS, Entry, Reader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranked {
    pub key: i64,
    pub traversal_order: Vec<u8>,
    pub id: i64,
    pub nested: bool,
}

impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key
            .cmp(&other.key)
            .then(self.traversal_order.cmp(&other.traversal_order))
            .then(self.id.cmp(&other.id))
            .then(self.nested.cmp(&other.nested))
    }
}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Pass {
    pub total: Total,
    pub page: Vec<Ranked>,
}

pub(super) fn retain<T: Ord>(heap: &mut BinaryHeap<T>, item: T, limit: u32) {
    if heap.len() < limit as usize {
        heap.push(item);
    } else if heap.peek().is_some_and(|worst| item < *worst) {
        *heap.peek_mut().expect("nonempty bounded heap") = item;
    }
}

/// Matches strict descendants of `scope`, with the same totals as the v1 pass.
pub fn run(reader: &Reader, scope: &Entry, filter: &Filter, order: &Order) -> Result<Pass> {
    reader.with_pin(|reader| run_pinned(reader, scope, filter, order))
}

fn run_pinned(reader: &Reader, scope: &Entry, filter: &Filter, order: &Order) -> Result<Pass> {
    let matcher = filter
        .name
        .as_ref()
        .map(|(p, icase)| glob::compile(p, *icase))
        .transpose()?;
    let generation = reader.generation(false)?;
    let mut query = reader.connection().prepare(&format!(
        "SELECT {ENTRY_COLUMNS} FROM versions WHERE scope=?1 AND sort_key>?2 AND sort_key<?3 AND born<=?4 AND (dead IS NULL OR dead>?4) ORDER BY sort_key"
    ))?;
    let mut rows = query.query(params![
        scope.scope,
        scope.sort_key,
        upper(&scope.sort_key),
        generation
    ])?;
    let mut total = Total::default();
    let mut page = BinaryHeap::new();
    let mut enclosing: Option<Vec<u8>> = None;
    while let Some(row) = rows.next()? {
        let entry = Entry::row(row)?;
        if !matches_kind(entry.kind, filter.kind) || entry.allocated < filter.min_size {
            continue;
        }
        let date = if entry.kind == Kind::Dir {
            entry.newest_mtime.unwrap_or(0)
        } else {
            entry.mtime
        };
        if filter.older.is_some_and(|cut| date >= cut)
            || filter.newer.is_some_and(|cut| date < cut)
            || matcher.as_ref().is_some_and(|m| !m.is_match(name(&entry)))
        {
            continue;
        }
        let nested = enclosing
            .as_ref()
            .is_some_and(|key| entry.sort_key.starts_with(key));
        total.count += 1;
        if !nested {
            total.counted += 1;
            total.allocated += entry.allocated.max(0) as u64;
            total.apparent += entry.apparent.max(0) as u64;
            enclosing = (entry.kind == Kind::Dir).then(|| entry.sort_key.clone());
        } else if !order.list_nested {
            continue;
        }
        let key = match order.sort {
            SortKey::Allocated => entry.allocated.saturating_neg(),
            SortKey::Apparent => entry.apparent.saturating_neg(),
            SortKey::Mtime => date,
        };
        retain(
            &mut page,
            Ranked {
                key,
                traversal_order: entry.sort_key,
                id: entry.id,
                nested,
            },
            order.limit,
        );
    }
    Ok(Pass {
        total,
        page: page.into_sorted_vec(),
    })
}

pub(super) fn matches_kind(kind: Kind, filter: KindFilter) -> bool {
    match filter {
        KindFilter::Any => true,
        KindFilter::Dir => kind == Kind::Dir,
        KindFilter::File => kind == Kind::File,
    }
}
