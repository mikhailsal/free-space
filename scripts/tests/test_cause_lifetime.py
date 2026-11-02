"""Tiny files and mock proc/sys only; no real process inventory or workload."""
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1]))
from cause_lifetime import LifetimeWatch
from cause_observe import FixtureAccess, PinSet
import cause_lifetime


class LifetimeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cause-observe-fixture-")
        self.root = Path(self.temp.name)
        self.access = FixtureAccess(self.root)
        self.snapshot = self.root / "snapshot"
        self.snapshot.mkdir()
        self.previous = self.snapshot / "snapshot.prev.db"
        self.previous.write_bytes(b"old" * 100)
        (self.snapshot / "snapshot.db").write_bytes(b"current")
        self.proc, self.cg = self.root / "proc", self.root / "cgroup"
        self.group = self.cg / "worker"
        self.group.mkdir(parents=True)
        (self.group / "cgroup.procs").write_text("123\n")
        self.pid(123, "/worker")
        self.pins = PinSet(self.snapshot, fixture=self.access, inspection_aliases=True)
        self.pins.pin_old_previous()
        self.watch = LifetimeWatch(self.snapshot, self.pins, 123, proc_root=self.proc,
                                    cgroup_root=self.cg, fixture=self.access)

    def tearDown(self):
        self.pins.close()
        self.temp.cleanup()

    def pid(self, pid, group):
        path = self.proc / str(pid)
        (path / "fd").mkdir(parents=True)
        (path / "stat").write_text(f"{pid} (worker) S " + " ".join(["0"] * 18 + ["777"] + ["0"] * 3))
        (path / "cgroup").write_text("0::" + group + "\n")
        return path

    def link_fd(self, name, path, pid=123):
        (self.proc / str(pid) / "fd" / str(name)).symlink_to(path)

    def test_identity_transitions_and_deduplicated_maxima(self):
        self.link_fd(3, self.previous)
        first = self.watch.poll()
        self.assertTrue(first["complete"])
        stage = self.snapshot / "snapshot.stage.tmp"
        stage.write_bytes(b"stage" * 100)
        self.pins.pin_stage()
        self.link_fd(4, stage)
        self.watch.poll()
        expected = sum((self.snapshot / name).stat().st_blocks * 512
                       for name in ("snapshot.db", "snapshot.prev.db", "snapshot.stage.tmp"))
        before = len(self.watch.transitions)
        # Size change is not a path-identity transition.
        with stage.open("ab") as stream:
            stream.write(b"x")
        self.watch.poll()
        self.assertEqual(len(self.watch.transitions), before)
        stage.unlink()
        (self.proc / "123/fd/4").unlink()
        self.watch.poll()
        result = self.watch.finish()
        self.assertTrue(result["coverage_complete"], result["errors"])
        peak = result["maxima"]["combined"]
        self.assertEqual(peak["allocated_bytes"], expected)
        self.assertEqual(len(peak["entries"]), 3)
        self.assertEqual(result["maxima"]["named_snapshot"]["allocated_bytes"], expected)
        stage_changes = [r["current"] for r in result["transitions"] if r["path"] == "named:snapshot.stage.tmp"]
        self.assertEqual(stage_changes[0], {"state": "absent"})
        self.assertIn("inode", stage_changes[1])
        self.assertEqual(stage_changes[-1], {"state": "absent"})
        self.assertIn("must not be summed", result["limitation"])
        with self.assertRaises(ValueError):
            self.watch.poll()

    def test_open_deleted_pin_and_fd_are_deduplicated(self):
        self.link_fd(3, self.previous)
        self.previous.unlink()
        self.pins.drop_inspection_aliases()
        original_stat = os.stat
        fd_path_inode = os.fstat(self.pins.fd("previous"))
        # Mock proc symlink stat for an unlinked descriptor, which ordinary
        # filesystem symlinks cannot model; no real /proc traversal is used.
        def read_stat(path, *args, **kwargs):
            if path == "3" and kwargs.get("dir_fd") is not None:
                return fd_path_inode
            return original_stat(path, *args, **kwargs)
        with patch.object(cause_lifetime.os, "stat", side_effect=read_stat):
            row = self.watch.poll()
        self.assertTrue(row["complete"], self.watch.errors)
        deleted = self.watch.finish()["maxima"]["open_deleted"]
        self.assertEqual(len(deleted["entries"]), 1)
        self.assertEqual(deleted["allocated_bytes"], fd_path_inode.st_blocks * 512)
        aliases = next(iter(deleted["entries"].values()))["aliases"]
        self.assertEqual({a["kind"] for a in aliases}, {"pin", "container_fd"})

    def test_only_exact_group_pids_are_inspected(self):
        foreign = self.pid(456, "/other")
        (foreign / "fd/4").symlink_to(self.previous)
        self.watch.poll()
        result = self.watch.finish()
        self.assertEqual(result["counts"]["pids_examined"], 1)
        self.assertEqual(result["counts"]["descriptors_examined"], 0)
        self.assertTrue(result["coverage_complete"])

    def test_unavailable_fd_is_explicit_race_not_zero_absence(self):
        self.link_fd(3, self.previous)
        self.watch.poll()
        self.previous.unlink()
        # A dangling ordinary symlink mocks a disappearing descriptor.
        self.watch.poll()
        result = self.watch.finish()
        self.assertFalse(result["coverage_complete"])
        self.assertGreater(result["counts"]["races"], 0)
        changes = [r for r in result["transitions"] if r["path"] == "fd:123:3"]
        self.assertEqual(changes[-1]["current"]["state"], "unavailable")

    def test_descriptor_and_trace_limits_are_reported(self):
        for fd in range(129):
            self.link_fd(fd, self.previous)
        self.watch.poll()
        self.assertEqual(self.watch.counts["descriptors_examined"], 128)
        self.assertGreater(self.watch.counts["limits"], 0)
        self.assertFalse(self.watch.finish()["coverage_complete"])

    def test_pid_migration_and_missing_cgroup_are_not_valid_coverage(self):
        (self.proc / "123/cgroup").write_text("0::/moved\n")
        self.watch.poll()
        result = self.watch.finish()
        self.assertGreater(result["counts"]["races"], 0)
        self.assertFalse(result["coverage_complete"])

    def test_failed_inventory_preserves_unknown_maximum(self):
        with patch.object(cause_lifetime, "record_inventory", side_effect=OSError("no inventory")):
            self.watch.poll()
        result = self.watch.finish()
        self.assertIsNone(result["maxima"]["combined"])
        self.assertFalse(result["coverage_complete"])
        self.assertTrue(result["errors"])

    def test_real_constructor_is_guarded_before_capture(self):
        with patch.dict(os.environ, {}, clear=True), \
             patch.object(cause_lifetime, "capture_cgroup", side_effect=AssertionError("no real reads")):
            with self.assertRaises(RuntimeError):
                LifetimeWatch(self.snapshot, self.pins, 123)

    def test_late_poll_records_gap_estimate_without_claiming_continuity(self):
        self.watch.poll()
        self.watch.last_start -= 500000000
        self.watch.poll()
        result = self.watch.finish()
        self.assertGreaterEqual(result["counts"]["missed_poll_intervals_estimate"], 9)
        self.assertIn("missed", result["limitation"])


if __name__ == "__main__":
    unittest.main()
