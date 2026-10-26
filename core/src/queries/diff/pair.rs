//! Private phases of the paired-snapshot diff query.

use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};

use super::{LevelRow, same_parent_path};
use crate::error::{Error, ErrorCode, Result};
use crate::queries::common::{Paths, lossy, raw_if_lossy};
use crate::queries::types::{DiffEntry, DiffStatus};
use crate::scan::Kind;

type RowPair = (Option<LevelRow>, Option<LevelRow>);
type RowKey = (i64, u64, Option<i64>);

struct PreviousRows {
    rows: Vec<Option<LevelRow>>,
    next: Vec<Option<usize>>,
    previous: Vec<Option<usize>>,
    by_key: HashMap<RowKey, usize>,
    name_hasher: RandomState,
}

impl PreviousRows {
    fn new(rows: Vec<LevelRow>) -> Self {
        let prev_rows: Vec<Option<LevelRow>> = rows.into_iter().map(Some).collect();
        let mut prev_next = vec![None; prev_rows.len()];
        let mut prev_prev = vec![None; prev_rows.len()];
        let name_hasher = RandomState::new();
        let mut prev_by_key: HashMap<(i64, u64, Option<i64>), usize> =
            HashMap::with_capacity(prev_rows.len());
        for (i, row) in prev_rows.iter().enumerate() {
            let row = row.as_ref().expect("previous row is present");
            let key = (
                row.hash,
                name_hasher.hash_one(&row.row.name),
                row.parent_hash,
            );
            prev_next[i] = prev_by_key.insert(key, i);
            if let Some(next) = prev_next[i] {
                prev_prev[next] = Some(i);
            }
        }
        Self {
            rows: prev_rows,
            next: prev_next,
            previous: prev_prev,
            by_key: prev_by_key,
            name_hasher,
        }
    }

    fn candidate(
        &self,
        c: &LevelRow,
        key: RowKey,
        cpaths: &mut Paths<'_>,
        ppaths: &mut Paths<'_>,
    ) -> Result<Option<usize>> {
        let mut candidate: Option<usize> = None;
        let mut cursor = self.by_key.get(&key).copied();
        while let Some(i) = cursor {
            let row = self.rows[i]
                .as_ref()
                .expect("linked previous row is present");
            if row.row.name == c.row.name
                && same_parent_path(cpaths, c.row.parent_id, ppaths, row.row.parent_id)?
                && candidate.is_none_or(|best| {
                    row.row.id
                        < self.rows[best]
                            .as_ref()
                            .expect("candidate previous row is present")
                            .row
                            .id
                })
            {
                candidate = Some(i);
            }
            cursor = self.next[i];
        }
        Ok(candidate)
    }

    fn take_match(
        &mut self,
        c: &LevelRow,
        cpaths: &mut Paths<'_>,
        ppaths: &mut Paths<'_>,
    ) -> Result<Option<LevelRow>> {
        let key = (
            c.hash,
            self.name_hasher.hash_one(&c.row.name),
            c.parent_hash,
        );
        if let Some(i) = self.candidate(c, key, cpaths, ppaths)? {
            if let Some(previous) = self.previous[i] {
                self.next[previous] = self.next[i];
            } else if let Some(next) = self.next[i] {
                self.by_key.insert(key, next);
            } else {
                self.by_key.remove(&key);
            }
            if let Some(next) = self.next[i] {
                self.previous[next] = self.previous[i];
            }
            Ok(self.rows[i].take())
        } else {
            Ok(None)
        }
    }
}

pub(super) fn match_rows(
    cur_rows: Vec<LevelRow>,
    prev_rows: Vec<LevelRow>,
    denied: &[(i64, i64)],
    cpaths: &mut Paths<'_>,
    ppaths: &mut Paths<'_>,
) -> Result<Vec<RowPair>> {
    let mut previous = PreviousRows::new(prev_rows);
    let mut pairs = Vec::new();
    for c in cur_rows {
        let pm = previous.take_match(&c, cpaths, ppaths)?;
        pairs.push((Some(c), pm));
    }
    for pr in previous.rows.into_iter().flatten() {
        if denied
            .iter()
            .any(|&(a, b)| pr.row.id >= a && pr.row.id <= b)
        {
            continue; // not observed this time, not gone (OV-3)
        }
        pairs.push((None, Some(pr)));
    }
    Ok(pairs)
}

pub(super) fn sort_pairs(pairs: &mut [RowPair]) {
    let delta_of = |c: &Option<LevelRow>, p: &Option<LevelRow>| {
        c.as_ref().map(|r| r.row.allocated).unwrap_or(0)
            - p.as_ref().map(|r| r.row.allocated).unwrap_or(0)
    };
    pairs.sort_by(|a, b| {
        let (da, db) = (delta_of(&a.0, &a.1), delta_of(&b.0, &b.1));
        db.unsigned_abs()
            .cmp(&da.unsigned_abs())
            .then((db > 0).cmp(&(da > 0)))
            .then_with(|| {
                let na = a.0.as_ref().or(a.1.as_ref()).map(|r| &r.row.name);
                let nb = b.0.as_ref().or(b.1.as_ref()).map(|r| &r.row.name);
                na.cmp(&nb)
            })
            .then_with(|| {
                // deterministic: current rows by id, then gone rows by their prev id
                let key = |x: &(Option<LevelRow>, Option<LevelRow>)| match (&x.0, &x.1) {
                    (Some(c), _) => (0, c.row.id),
                    (None, Some(p)) => (1, p.row.id),
                    (None, None) => (2, 0),
                };
                key(a).cmp(&key(b))
            })
    });
}

pub(super) fn entries<'conn>(
    pairs: &[RowPair],
    cpaths: &mut Paths<'conn>,
    ppaths: &mut Paths<'conn>,
) -> Result<Vec<DiffEntry>> {
    let mut entries = Vec::with_capacity(pairs.len());
    for (c, pv) in pairs {
        validate_pair(c, pv, cpaths, ppaths)?;
        entries.push(entry(c, pv, cpaths, ppaths)?);
    }
    Ok(entries)
}

fn validate_pair(
    c: &Option<LevelRow>,
    pv: &Option<LevelRow>,
    cpaths: &mut Paths<'_>,
    ppaths: &mut Paths<'_>,
) -> Result<()> {
    if let (Some(c), Some(pv)) = (c, pv) {
        let same_parent = same_parent_path(cpaths, c.row.parent_id, ppaths, pv.row.parent_id)?;
        if c.row.name != pv.row.name || !same_parent {
            return Err(Error::new(
                    ErrorCode::HashCollision,
                    format!(
                        "path hash collision between {} (current) and {} (prev)",
                        lossy(&cpaths.path(c.row.id)?),
                        lossy(&ppaths.path(pv.row.id)?)
                    ),
                )
                .with_fix("report this at https://github.com/mikhailsal/free-space/issues; ls and top are unaffected"));
        }
    }
    Ok(())
}

fn entry<'conn>(
    c: &Option<LevelRow>,
    pv: &Option<LevelRow>,
    cpaths: &mut Paths<'conn>,
    ppaths: &mut Paths<'conn>,
) -> Result<DiffEntry> {
    let (src, paths) = match c {
        Some(c) => (&c.row, cpaths),
        None => (&pv.as_ref().expect("one side present").row, ppaths),
    };
    let full = paths.path(src.id)?;
    let allocated = c
        .as_ref()
        .map(|r| r.row.allocated.max(0) as u64)
        .unwrap_or(0);
    let allocated_prev = pv
        .as_ref()
        .map(|r| r.row.allocated.max(0) as u64)
        .unwrap_or(0);
    let items = c.as_ref().map(|r| r.row.items.max(0) as u64).unwrap_or(0);
    let items_prev = pv.as_ref().map(|r| r.row.items.max(0) as u64).unwrap_or(0);
    let delta = allocated as i64 - allocated_prev as i64;
    let status = match (c, pv) {
        (Some(_), None) => DiffStatus::New,
        (None, Some(_)) => DiffStatus::Gone,
        _ if delta != 0 || items != items_prev => DiffStatus::Changed,
        _ => DiffStatus::Same,
    };
    Ok(DiffEntry {
        path: lossy(&full),
        path_raw: raw_if_lossy(&full),
        name: lossy(&src.name),
        kind: Kind::from_code(src.kind),
        status,
        allocated,
        allocated_prev,
        delta,
        items,
        items_prev,
        mtime: src.mtime,
        newest_mtime: src.newest_mtime,
    })
}
