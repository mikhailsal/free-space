use super::*;
use std::collections::VecDeque;
use std::mem::size_of;

const BATCH: usize = 32;
const ROW_LIMIT: usize = 256;
const BYTE_LIMIT: usize = 64 * 1024;
// Bookkeeping allowance per allocation; this is not an allocator-resident bound.
const ALLOC_OVERHEAD: usize = 64;

#[derive(Default)]
pub(super) struct Pending {
    rows: VecDeque<Raw>,
    bytes: usize,
    slots: usize,
}

pub(super) struct Budget {
    slots: usize,
    bytes: usize,
    max_slots: usize,
    max_bytes: usize,
    #[cfg(test)]
    selects: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            slots: 0,
            bytes: 0,
            max_slots: ROW_LIMIT,
            max_bytes: BYTE_LIMIT,
            #[cfg(test)]
            selects: 0,
        }
    }
}

impl Budget {
    pub(super) fn can_buffer(&self) -> bool {
        self.slots.saturating_add(BATCH) <= self.max_slots
            && self
                .bytes
                .saturating_add(BATCH * size_of::<Raw>() + ALLOC_OVERHEAD)
                <= self.max_bytes
    }
}

impl Pending {
    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(super) fn release(&mut self, budget: &mut Budget) {
        debug_assert_eq!(self.rows.capacity(), self.slots);
        budget.slots -= self.slots;
        budget.bytes -= self.bytes;
        self.rows = VecDeque::new();
        self.bytes = 0;
        self.slots = 0;
    }

    fn pop(&mut self, budget: &mut Budget) -> Option<Raw> {
        let row = self.rows.pop_front()?;
        let bytes = name_bytes(&row);
        self.bytes -= bytes;
        budget.bytes -= bytes;
        if self.rows.is_empty() {
            self.release(budget);
        }
        Some(row)
    }
}

fn name_bytes(row: &Raw) -> usize {
    if row.name.capacity() == 0 {
        0
    } else {
        row.name.capacity().saturating_add(ALLOC_OVERHEAD)
    }
}

// The budget covers extra pending buffers across all active DFS frames, not
// baseline cursors, paths, SQLite memory, an in-flight row, or process RSS.
pub(super) fn next(
    conn: &Connection,
    raw: u64,
    cursor: &Option<(Vec<u8>, u64)>,
    pending: &mut Pending,
    budget: &mut Budget,
) -> Result<Option<Raw>> {
    if let Some(row) = pending.pop(budget) {
        return Ok(Some(row));
    }
    let minimum = BATCH * size_of::<Raw>() + ALLOC_OVERHEAD;
    if !budget.can_buffer() {
        return single(conn, raw, cursor, budget);
    }
    #[cfg(test)]
    {
        budget.selects += 1;
    }
    let sql = if cursor.is_some() {
        format!(
            "SELECT {RAW_COLS} FROM raw WHERE parent=?1 AND (name,id)>(?2,?3) ORDER BY name,id LIMIT {BATCH}"
        )
    } else {
        format!("SELECT {RAW_COLS} FROM raw WHERE parent=?1 ORDER BY name,id LIMIT {BATCH}")
    };
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = if let Some((name, id)) = cursor {
        stmt.query(params![raw as i64, name, *id as i64])?
    } else {
        stmt.query([raw as i64])?
    };
    let Some(first) = rows.next()? else {
        return Ok(None);
    };
    let first = Raw::from_sql(first)?;
    let first_bytes = name_bytes(&first);
    if budget
        .bytes
        .saturating_add(minimum)
        .saturating_add(first_bytes)
        > budget.max_bytes
    {
        return Ok(Some(first));
    }
    let buffer = VecDeque::with_capacity(BATCH);
    let slots = buffer.capacity();
    let bytes = slots
        .saturating_mul(size_of::<Raw>())
        .saturating_add(ALLOC_OVERHEAD);
    if budget.slots.saturating_add(slots) > budget.max_slots
        || budget
            .bytes
            .saturating_add(bytes)
            .saturating_add(first_bytes)
            > budget.max_bytes
    {
        return Ok(Some(first));
    }
    pending.rows = buffer;
    pending.slots = slots;
    pending.bytes = bytes + first_bytes;
    budget.slots += slots;
    budget.bytes += pending.bytes;
    pending.rows.push_back(first);
    while let Some(row) = rows.next()? {
        let row = Raw::from_sql(row)?;
        let bytes = name_bytes(&row);
        if budget.bytes.saturating_add(bytes) > budget.max_bytes {
            break; // The consumed cursor still points before this discarded row.
        }
        budget.bytes += bytes;
        pending.bytes += bytes;
        pending.rows.push_back(row);
    }
    Ok(pending.pop(budget))
}

fn single(
    conn: &Connection,
    raw: u64,
    cursor: &Option<(Vec<u8>, u64)>,
    budget: &mut Budget,
) -> Result<Option<Raw>> {
    #[cfg(test)]
    {
        budget.selects += 1;
    }
    #[cfg(not(test))]
    let _ = budget;
    let row = if let Some((name, id)) = cursor {
        conn.prepare_cached(&format!("SELECT {RAW_COLS} FROM raw WHERE parent=?1 AND (name,id)>(?2,?3) ORDER BY name,id LIMIT 1"))?
            .query_row(params![raw as i64, name, *id as i64], Raw::from_sql).optional()?
    } else {
        conn.prepare_cached(&format!(
            "SELECT {RAW_COLS} FROM raw WHERE parent=?1 ORDER BY name,id LIMIT 1"
        ))?
        .query_row([raw as i64], Raw::from_sql)
        .optional()?
    };
    Ok(row)
}

#[cfg(test)]
mod tests;
