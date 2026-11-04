from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import cleanup_buckets as cb


def stats(entries, allocated=0, newest=None):
    s = cb.Stats()
    s.files = entries
    s.allocated = allocated
    s.newest = newest
    return s


class SlotHashTests(unittest.TestCase):
    def test_vectors_match_the_rust_probe(self):
        # The same vectors are asserted by buckets.rs; a mismatch breaks refresh lookup.
        for name, slot in ((b'', 3979), (b'small', 59), (b'g000500', 3502), (b'raw-\xff', 1228),
                           (b'large', 1142), (b'bulk', 3954)):
            self.assertEqual(cb.slot_of(name), slot, name)

    def test_slots_stay_in_range_and_spread_sequential_names(self):
        counts = [0] * cb.SLOTS
        for i in range(40960):
            counts[cb.slot_of(f'n{i:09}'.encode())] += 1
        self.assertLess(max(counts), 40)
        self.assertTrue(all(counts))


class TilingTests(unittest.TestCase):
    def test_empty_directory_has_one_bucket_covering_everything(self):
        tiles = cb.tile({})
        self.assertEqual([(lo, hi, s.entries) for lo, hi, s in tiles], [(0, cb.SLOTS, 0)])

    def test_cap_closes_buckets_but_one_slot_may_exceed_it(self):
        slots = {3: stats(4, 400), 4: stats(4, 400), 9: stats(4, 400), 10: stats(50, 5000), 11: stats(1, 100), 4000: stats(3, 300)}
        tiles = cb.tile(slots, cap=10)
        self.assertEqual([s.entries for _, _, s in tiles], [8, 4, 50, 4])
        self.assertEqual(sum(s.allocated for _, _, s in tiles), 6600)
        cb.validate([(lo, hi) for lo, hi, _ in tiles])

    def test_range_tiling_ignores_other_slots(self):
        slots = {5: stats(6), 50: stats(6), 51: stats(6), 500: stats(6)}
        tiles = cb.tile(slots, cap=12, lo=40, hi=100)
        self.assertEqual([(lo, hi, s.entries) for lo, hi, s in tiles], [(40, 100, 12)])
        with self.assertRaises(ValueError):
            cb.tile(slots, lo=100, hi=100)

    def test_newest_mtime_is_the_maximum(self):
        merged = stats(1, newest=5)
        merged.add(stats(1, newest=9))
        merged.add(stats(1))
        self.assertEqual(merged.newest, 9)

    def test_validate_rejects_gaps_overlaps_and_short_cover(self):
        cb.validate([(0, 10), (10, cb.SLOTS)])
        for broken in ([], [(0, 10)], [(0, 10), (11, cb.SLOTS)], [(0, 10), (9, cb.SLOTS)], [(1, cb.SLOTS)]):
            with self.assertRaises(ValueError, msg=broken):
                cb.validate(broken)

    def test_fixture_chunk_directory_range(self):
        # Million-entry fixture: 1000 chunk directories with 1001 entries each.
        names = [f'g{i:06}'.encode() for i in range(1000)]
        lo, hi, entries = cb.range_of(names, 1001, b'g000050')
        self.assertLessEqual(lo, cb.slot_of(b'g000050'))
        self.assertLess(cb.slot_of(b'g000050'), hi)
        self.assertEqual(entries % 1001, 0)
        self.assertLessEqual(entries, cb.BUCKET_ENTRIES)
        # The 100k smoke profile has only the first hundred chunk directories.
        smoke = cb.range_of(names[:100], 1001, b'g000050')
        self.assertEqual(smoke[2] % 1001, 0)
        self.assertLessEqual(smoke[2], cb.BUCKET_ENTRIES)


if __name__ == '__main__':
    unittest.main()
