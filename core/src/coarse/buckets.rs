//! Bounded residual buckets: the folded children of a retained directory are spread
//! over hash slots, and contiguous slot ranges are tiled into buckets that hold about
//! `BUCKET_ENTRIES` entries. Refreshing a folded path rescans the bucket that holds it
//! instead of the whole retained directory.

use std::collections::BTreeMap;

use super::model::{Bucket, SLOT_BITS, SLOTS, Stats, invariant};
use crate::error::Result;

pub type Slots = BTreeMap<u32, Stats>;

/// FNV-1a, 64 bit, over the raw bytes of a name (the published algorithm).
fn fnv1a(name: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Stable name hash: FNV-1a, then a Fibonacci mix to use its well-mixed top bits. The
/// slot of a name is part of the stored format: changing this is a format change.
pub fn slot_of(name: &[u8]) -> u32 {
    (fnv1a(name).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - SLOT_BITS)) as u32
}

/// Tile `[lo, hi)` greedily. A bucket is closed before the slot that would push it over
/// `cap` entries; a single slot may exceed `cap` because it cannot be split. The result
/// always has at least one bucket, so an empty directory still covers every slot.
pub fn tile(slots: &Slots, cap: u64, lo: u32, hi: u32) -> Result<Vec<Bucket>> {
    if lo >= hi || hi > SLOTS {
        return Err(invariant("invalid slot range"));
    }
    let mut out = Vec::new();
    let mut start = lo;
    let mut acc = Stats::default();
    for (&slot, stats) in slots.range(lo..hi) {
        if acc.entries > 0 && acc.entries.saturating_add(stats.entries) > cap {
            out.push(Bucket {
                lo: start,
                hi: slot,
                stats: std::mem::take(&mut acc),
            });
            start = slot;
        }
        acc.add(stats);
    }
    out.push(Bucket {
        lo: start,
        hi,
        stats: acc,
    });
    Ok(out)
}

pub fn find(buckets: &[Bucket], slot: u32) -> Option<usize> {
    buckets.iter().position(|b| (b.lo..b.hi).contains(&slot))
}

/// The buckets of a directory must be contiguous and cover every slot exactly once.
pub fn validate(buckets: &[Bucket]) -> Result<()> {
    let mut next = 0;
    for bucket in buckets {
        if bucket.lo != next || bucket.hi <= bucket.lo || bucket.hi > SLOTS {
            return Err(invariant("bucket tiling is not contiguous"));
        }
        next = bucket.hi;
    }
    if next != SLOTS {
        return Err(invariant("bucket tiling does not cover all slots"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
