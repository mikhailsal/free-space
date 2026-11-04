//! Bounded residual buckets: the folded-away children of a retained directory are
//! spread over hash slots, and contiguous slot ranges are tiled into buckets that hold
//! about `BUCKET_ENTRIES` entries. Refreshing a folded path rescans one bucket, never
//! the whole retained directory.
use crate::model::{Bucket, R, SLOT_BITS, SLOTS, Stats};
use std::collections::BTreeMap;

pub type Slots = BTreeMap<u32, Stats>;

/// Stable name hash (FNV-1a, then a Fibonacci mix to use its well-mixed top bits).
/// The oracle and the tests carry the same constants; changing them is a format change.
pub fn slot_of(name: &[u8]) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (h.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - SLOT_BITS)) as u32
}

/// Tile `[lo, hi)` greedily. A bucket is closed before the slot that would push it over
/// `cap` entries; a single slot may exceed `cap` because it cannot be split. The result
/// always has at least one bucket, so an empty directory still covers every slot.
pub fn tile(slots: &Slots, cap: u64, lo: u32, hi: u32) -> R<Vec<Bucket>> {
    if lo >= hi || hi > SLOTS {
        return Err("invalid slot range".into());
    }
    let mut out = Vec::new();
    let mut start = lo;
    let mut acc = Stats::default();
    for (&slot, stats) in slots.range(lo..hi) {
        let held = acc.entries();
        if held > 0 && held.saturating_add(stats.entries()) > cap {
            out.push(Bucket {
                lo: start,
                hi: slot,
                stats: std::mem::take(&mut acc),
            });
            start = slot;
        }
        acc.add(stats)?;
    }
    out.push(Bucket {
        lo: start,
        hi,
        stats: acc,
    });
    Ok(out)
}

pub fn find(buckets: &[Bucket], slot: u32) -> Option<usize> {
    buckets.iter().position(|b| b.lo <= slot && slot < b.hi)
}

/// The buckets of a directory must be contiguous and cover every slot exactly once.
pub fn validate(buckets: &[Bucket]) -> R<()> {
    let mut next = 0;
    for bucket in buckets {
        if bucket.lo != next || bucket.hi <= bucket.lo || bucket.hi > SLOTS {
            return Err("bucket tiling is not contiguous".into());
        }
        next = bucket.hi;
    }
    if next != SLOTS {
        return Err("bucket tiling does not cover all slots".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(entries: u64, allocated: u64) -> Stats {
        Stats {
            allocated,
            files: entries,
            min_scanned_at: Some(7),
            ..Stats::default()
        }
    }

    #[test]
    fn slot_hash_is_stable_across_languages() {
        // scripts/tests/test_cleanup_oracle.py asserts the same vectors from an independent
        // Python implementation; a mismatch would break the oracle's refresh lookup.
        let vectors: [(&[u8], u32); 6] = [
            (b"", 3979),
            (b"small", 59),
            (b"g000500", 3502),
            (b"raw-\xff", 1228),
            (b"large", 1142),
            (b"bulk", 3954),
        ];
        for (name, slot) in vectors {
            assert_eq!(slot_of(name), slot, "{name:?}");
        }
        assert!((0..2000u32).all(|i| slot_of(format!("n{i}").as_bytes()) < SLOTS));
    }

    #[test]
    fn slot_hash_spreads_sequential_names() {
        let mut counts = vec![0u32; SLOTS as usize];
        for i in 0..40_960u32 {
            counts[slot_of(format!("n{i:09}").as_bytes()) as usize] += 1;
        }
        let worst = counts.iter().copied().max().unwrap();
        assert!(worst < 40, "worst slot holds {worst} of 40960 names");
        assert!(counts.iter().all(|c| *c > 0));
    }

    #[test]
    fn empty_directory_still_covers_every_slot() {
        let tiles = tile(&Slots::new(), 10, 0, SLOTS).unwrap();
        assert_eq!(tiles.len(), 1);
        assert_eq!((tiles[0].lo, tiles[0].hi), (0, SLOTS));
        assert_eq!(tiles[0].stats.entries(), 0);
        validate(&tiles).unwrap();
    }

    #[test]
    fn tiles_respect_cap_unless_one_slot_is_larger() {
        let mut slots = Slots::new();
        for (slot, entries) in [(3, 4), (4, 4), (9, 4), (10, 50), (11, 1), (4000, 3)] {
            slots.insert(slot, stats(entries, entries * 100));
        }
        let tiles = tile(&slots, 10, 0, SLOTS).unwrap();
        validate(&tiles).unwrap();
        let entries: Vec<u64> = tiles.iter().map(|t| t.stats.entries()).collect();
        assert_eq!(entries, vec![8, 4, 50, 4]);
        assert_eq!(tiles.iter().map(|t| t.stats.allocated).sum::<u64>(), 6600);
        assert!(
            tiles
                .iter()
                .all(|t| t.stats.entries() <= 10 || slots.range(t.lo..t.hi).count() == 1)
        );
    }

    #[test]
    fn range_tiles_keep_outer_bounds_and_ignore_other_slots() {
        let mut slots = Slots::new();
        for slot in [5, 50, 51, 500] {
            slots.insert(slot, stats(6, 1));
        }
        let tiles = tile(&slots, 12, 40, 100).unwrap();
        assert_eq!((tiles[0].lo, tiles.last().unwrap().hi), (40, 100));
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].stats.entries(), 12);
        assert!(tile(&slots, 12, 100, 100).is_err());
        assert!(tile(&slots, 12, 0, SLOTS + 1).is_err());
    }

    #[test]
    fn find_and_validate_detect_gaps() {
        let mut slots = Slots::new();
        slots.insert(7, stats(20, 1));
        slots.insert(900, stats(20, 1));
        let tiles = tile(&slots, 25, 0, SLOTS).unwrap();
        assert_eq!(find(&tiles, 7), Some(0));
        assert_eq!(find(&tiles, 900), Some(1));
        assert_eq!(find(&tiles, SLOTS), None);
        let mut broken = tiles.clone();
        broken[1].lo += 1;
        assert!(validate(&broken).is_err());
        broken = tiles.clone();
        broken.pop();
        assert!(validate(&broken).is_err());
        assert!(validate(&[]).is_err());
    }
}
