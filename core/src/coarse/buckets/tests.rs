use super::*;
use crate::coarse::model::BUCKET_ENTRIES;

fn stats(entries: u64, allocated: u64) -> Stats {
    Stats {
        allocated,
        entries,
        min_scanned_at: Some(7),
        ..Stats::default()
    }
}

#[test]
fn slot_hash_is_stable() {
    // The slot of a name is part of the stored format. These vectors come from an
    // independent implementation of the published algorithm; a mismatch would make
    // refresh look into the wrong bucket of every existing snapshot.
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
fn fnv1a_matches_the_published_vectors() {
    let vectors: [(&[u8], u64); 3] = [
        (b"", 0xcbf2_9ce4_8422_2325),
        (b"a", 0xaf63_dc4c_8601_ec8c),
        (b"foobar", 0x8594_4171_f739_67e8),
    ];
    for (name, hash) in vectors {
        assert_eq!(fnv1a(name), hash, "{name:?}");
    }
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
    assert_eq!(tiles[0].stats.entries, 0);
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
    let entries: Vec<u64> = tiles.iter().map(|t| t.stats.entries).collect();
    assert_eq!(entries, vec![8, 4, 50, 4]);
    assert_eq!(tiles.iter().map(|t| t.stats.allocated).sum::<u64>(), 6600);
    assert!(
        tiles
            .iter()
            .all(|t| t.stats.entries <= 10 || slots.range(t.lo..t.hi).count() == 1)
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
    assert_eq!(tiles[0].stats.entries, 12);
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

#[test]
fn wide_windows_are_stable() {
    // Equal 1001-entry chunk directories g000000..: the target's bucket is shared with
    // siblings. The windows are the ones the research reference produced for the same
    // names, so the tiling stays reproducible across implementations.
    for (chunks, lo, hi, entries) in [(100u32, 804, 971, 4004), (1000, 945, 956, 4004)] {
        let mut slots = Slots::new();
        for i in 0..chunks {
            let slot = slot_of(format!("g{i:06}").as_bytes());
            slots.entry(slot).or_default().add(&stats(1001, 0));
        }
        let tiles = tile(&slots, BUCKET_ENTRIES, 0, SLOTS).unwrap();
        validate(&tiles).unwrap();
        let bucket = &tiles[find(&tiles, slot_of(b"g000050")).unwrap()];
        assert_eq!(
            (bucket.lo, bucket.hi, bucket.stats.entries),
            (lo, hi, entries),
            "{chunks} chunks"
        );
    }
}
