"""W5 per-case admission preserves both independent space checks."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bench_v1_pair import stage_space_receipt


class StageSpaceTests(unittest.TestCase):
    def test_both_limits_are_required(self):
        minimum = stage_space_receipt(1_200_000_000, 100, 100, 2_400_000_000, 0)
        self.assertGreater(minimum['stage_required_bytes'], minimum['common_required_bytes'])
        self.assertFalse(stage_space_receipt(1_200_000_000, 100, 100, 2_400_000_000,
                                             minimum['stage_required_bytes'] - 1)['success'])
        self.assertTrue(stage_space_receipt(1_200_000_000, 100, 100, 2_400_000_000,
                                            minimum['stage_required_bytes'])['success'])
        minimum = stage_space_receipt(1, 1, 1, 10_000_000_000, 0)
        self.assertGreater(minimum['common_required_bytes'], minimum['stage_required_bytes'])
        self.assertFalse(stage_space_receipt(1, 1, 1, 10_000_000_000,
                                             minimum['common_required_bytes'] - 1)['success'])

    def test_count_estimate_and_overflow(self):
        receipt = stage_space_receipt(100, 1000, 100, 0, 10**12)
        self.assertEqual(receipt['estimated_entries'], 1099)
        self.assertEqual(receipt['stage_required_bytes'], 6 * 150 * 1099 + 512 * 1024 * 1024)
        for args in ((0, 1, 0, 0, 0), (True, 1, 1, 0, 0), (2**64, 1, 1, 0, 0)):
            with self.subTest(args=args), self.assertRaises(ValueError):
                stage_space_receipt(*args)
