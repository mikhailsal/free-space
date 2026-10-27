use super::{Entry, insert, live_id, retire};
use crate::{Error, Result};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
#[derive(Clone, Copy, Default)]
pub(super) struct Delta {
    allocated: i128,
    apparent: i128,
    items: i128,
    errors: i128,
}
impl Delta {
    pub(super) fn between(old: &Entry, new: Option<&Entry>) -> Self {
        Self {
            allocated: new.map_or(0, |e| e.allocated) as i128 - old.allocated as i128,
            apparent: new.map_or(0, |e| e.apparent) as i128 - old.apparent as i128,
            items: new.map_or(0, |e| e.items as i128 + 1) - (old.items as i128 + 1),
            errors: new.map_or(0, |e| e.errors) as i128 - old.errors as i128,
        }
    }
    fn merge(&mut self, d: Self) {
        self.allocated += d.allocated;
        self.apparent += d.apparent;
        self.items += d.items;
        self.errors += d.errors;
    }
}
fn apply(value: i64, delta: i128) -> Result<i64> {
    i64::try_from(value as i128 + delta)
        .ok()
        .filter(|v| *v >= 0)
        .ok_or_else(|| Error::generic("aggregate delta exceeds v2 integer range"))
}
/// Union ancestor paths before changing any rows, then update deepest first.
/// One row version per affected ancestor even when many inode groups converge.
pub(super) fn update(
    c: &Connection,
    changes: Vec<(Option<i64>, Delta)>,
    generation: i64,
) -> Result<usize> {
    let mut ancestors: HashMap<i64, Entry> = HashMap::new();
    let mut deltas: HashMap<i64, Delta> = HashMap::new();
    for (parent, delta) in changes {
        if let Some(p) = parent {
            deltas.entry(p).or_default().merge(delta);
        }
        let mut parent = parent;
        while let Some(id) = parent {
            if ancestors.contains_key(&id) {
                break;
            }
            let e = live_id(c, id)?;
            parent = e.parent;
            ancestors.insert(id, e);
        }
    }
    let count = ancestors.len();
    let mut entries: Vec<_> = ancestors.into_values().collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.sort_key.len()));
    for mut e in entries {
        let d = deltas.remove(&e.id).unwrap_or_default();
        e.allocated = apply(e.allocated, d.allocated)?;
        e.apparent = apply(e.apparent, d.apparent)?;
        e.items = apply(e.items, d.items)?;
        e.errors = apply(e.errors, d.errors)?;
        let min: Option<i64> = c
            .query_row(
                "SELECT min_scanned_at FROM live_rows WHERE parent=?1 ORDER BY min_scanned_at LIMIT 1",
                [e.id],
                |r| r.get(0),
            )
            .optional()?;
        let newest:Option<i64>=c.query_row("SELECT newest_mtime FROM live_rows WHERE parent=?1 AND newest_mtime IS NOT NULL ORDER BY newest_mtime DESC LIMIT 1",[e.id],|r|r.get(0)).optional()?;
        e.min_scanned_at = e.scanned_at.min(min.unwrap_or(e.scanned_at));
        e.newest_mtime = newest;
        if let Some(p) = e.parent {
            deltas.entry(p).or_default().merge(d);
        }
        retire(c, e.id, generation)?;
        insert(c, &e, generation)?;
    }
    Ok(count)
}
