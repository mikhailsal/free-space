"""Admission calculations do not touch disks or start benchmarks."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "bench_v1_admission", Path(__file__).resolve().parents[1] / "bench_v1_admission.py")
ADMISSION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ADMISSION)


class CapacityTests(unittest.TestCase):
    def test_components_cover_concurrent_snapshots_and_seed(self):
        plan = ADMISSION.estimate_capacity(7_000_000, 7_007_020, seed_bytes=12345)
        parts = plan["components"]
        self.assertEqual(parts["fixture_payload_bytes"], 70_000 * 4096)
        for name in ("current", "previous", "stage", "output"):
            self.assertEqual(parts[name + "_snapshot_bytes"], 7_007_020 * 512)
        self.assertEqual(parts["retained_seed_bytes"], 12345)
        self.assertEqual(plan["required_bytes"], sum(parts.values()))
        self.assertEqual(plan["required_inodes"], 7_017_020)
        self.assertLess(plan["required_bytes"], 28 * ADMISSION.GIB)

    def test_invalid_counts(self):
        for args in ((-1, 0), (2, 1), (0, -1), (0, 0, -1), (True, 1)):
            with self.subTest(args=args), self.assertRaises(ValueError):
                ADMISSION.estimate_capacity(*args)

    def test_bytes_and_inodes_are_both_required(self):
        plan = ADMISSION.estimate_capacity(10, 12, reserve_bytes=0)
        def check(byte_delta, inode_delta):
            stat = SimpleNamespace(f_frsize=1, f_bsize=4096,
                                   f_bavail=plan["required_bytes"] + byte_delta,
                                   f_favail=plan["required_inodes"] + inode_delta)
            with patch.object(ADMISSION.os, "statvfs", return_value=stat):
                return ADMISSION.check_capacity("/fixture", 10, 12, reserve_bytes=0)
        self.assertTrue(check(0, 0)["success"])
        self.assertFalse(check(-1, 0)["success"])
        self.assertFalse(check(0, -1)["success"])

    def test_unavailable_stat_fails_closed(self):
        with patch.object(ADMISSION.os, "statvfs", side_effect=OSError("unavailable")):
            receipt = ADMISSION.check_capacity("/missing", 1, 1)
        self.assertFalse(receipt["success"])
        self.assertIn("unavailable", receipt["error"])


class TimeTests(unittest.TestCase):
    def test_time_with_margin(self):
        self.assertTrue(ADMISSION.admit_time(130, 100)["success"])
        self.assertFalse(ADMISSION.admit_time(129, 100)["success"])
        self.assertEqual(ADMISSION.admit_time(200, 100)["required_seconds"], 130)

    def test_missing_invalid_or_nonfinite_time_fails_closed(self):
        for remaining, estimated, margin in ((None, 10, .3), (10, None, .3), (0, 10, .3),
                                             (10, 0, .3), (10, 10, -1), (float("nan"), 10, .3)):
            with self.subTest(remaining=remaining, estimated=estimated, margin=margin):
                self.assertFalse(ADMISSION.admit_time(remaining, estimated, margin)["success"])


if __name__ == "__main__":
    unittest.main()
