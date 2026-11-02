"""Tiny fixtures only: no app binaries, Docker, real proc/sys probes or workloads."""
import importlib.util
import io
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).parents[1]))
import cause_global

SPEC = importlib.util.spec_from_file_location("cause_observe", Path(__file__).parents[1] / "cause_observe.py")
observe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(observe)


class CauseObserveTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="cause-observe-fixture-")
        self.root = Path(self.tmp.name)
        self.access = observe.FixtureAccess(self.root)
        self.snapshot = self.root / "snapshot"
        self.snapshot.mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def database(self, name, rows=3):
        path = self.snapshot / name
        with sqlite3.connect(path) as conn:
            conn.execute("CREATE TABLE entries(id INTEGER PRIMARY KEY,name TEXT)")
            conn.executemany("INSERT INTO entries VALUES(?,?)", [(i, "file" + str(i)) for i in range(rows)])
            conn.execute("CREATE INDEX names ON entries(name)")
        return path

    def test_real_operations_fail_without_hosted_guard_before_read(self):
        with patch.dict(os.environ, {}, clear=True):
            with patch.object(observe, "_read", side_effect=AssertionError("must not read")):
                for run in (lambda: observe.capture_global(self.root / "out"),
                            lambda: observe.capture_cgroup(123),
                            lambda: observe.PinSet(self.snapshot),
                            lambda: observe.db_breakdown(self.snapshot / "x.db")):
                    with self.assertRaises(RuntimeError):
                        run()
        with self.assertRaises(ValueError):
            observe.FixtureAccess(Path(tempfile.gettempdir()))
        with self.assertRaises(ValueError):
            observe.capture_global(self.root / "out", fixture=self.access)

    def test_global_raw_parsed_and_missing_fields_are_explicit(self):
        proc = self.root / "proc"
        proc.mkdir()
        (proc / "slabinfo").write_text("slabinfo - version: 2.1\n# name <active_objs>\ndentry 3 7 192 21 1 : tunables 0 0 0 : slabdata 1 2 0\n")
        (proc / "meminfo").write_text("MemTotal: 1024 kB\nSlab: 12 kB\n")
        row = observe.capture_global(self.root / "out", proc_root=proc, fixture=self.access)
        self.assertIn("GLOBAL", row["scope"])
        slab = row["sources"]["slabinfo"]["parsed"]["objects"][0]
        self.assertEqual((slab["object_size_bytes"], slab["total_objects"], slab["total_slab_pages"]), (192, 7, 2))
        self.assertNotIn("SReclaimable", row["sources"]["meminfo"]["parsed"]["fields"])
        self.assertEqual((self.root / "out/slabinfo.raw").read_text(), row["sources"]["slabinfo"]["raw"])
        self.assertEqual(json.loads((self.root / "out/global.json").read_text())["schema"], 1)
        (proc / "slabinfo").unlink()
        missing = observe.capture_global(self.root / "missing", proc_root=proc, fixture=self.access)
        self.assertFalse(missing["sources"]["slabinfo"]["available"])
        self.assertNotIn("parsed", missing["sources"]["slabinfo"])
        self.assertFalse(observe.parse_slabinfo("broken line")["required_fields_available"])

    def fake_sudo(self, output=b"dentry 3 7 192 21 1 : tunables 0 0 0 : slabdata 1 2 0\n"):
        streams = {9001: io.BytesIO(output), 9002: io.BytesIO(b"")}
        proc = SimpleNamespace(stdout=SimpleNamespace(fileno=lambda: 9001, close=lambda: None),
                               stderr=SimpleNamespace(fileno=lambda: 9002, close=lambda: None),
                               wait=lambda **unused: 0, poll=lambda: 0, kill=lambda: None)
        class FakeSelector:
            def __init__(self): self.sources = {}
            def __enter__(self): return self
            def __exit__(self, *unused): pass
            def register(self, source, event, data): self.sources[source.fileno()] = (source, data)
            def get_map(self): return self.sources
            def select(self, unused):
                return [(SimpleNamespace(fileobj=f, data=d), 1) for f, d in self.sources.values()]
            def unregister(self, source): self.sources.pop(source.fileno())
        return proc, FakeSelector, lambda fd, count: streams[fd].read(count)

    def test_fixed_privileged_global_fallback_records_reader_and_raw(self):
        proc, selector, read = self.fake_sudo()
        denied = dict(available=False, error="permission denied", error_type="PermissionError", errno=13)
        def direct(name, **unused):
            return dict(denied) if name == "slabinfo" else dict(available=True, raw="MemTotal: 1024 kB\n")
        with patch.dict(os.environ, GITHUB_ACTIONS="true", RUNNER_ENVIRONMENT="github-hosted"), \
             patch.object(cause_global, "_open_dir", return_value=9999), \
             patch.object(cause_global, "_read", side_effect=direct), \
             patch.object(cause_global.os, "fstat", return_value=self.root.stat()), \
             patch.object(cause_global.os, "close"), \
             patch.object(cause_global.subprocess, "Popen", return_value=proc) as popen, \
             patch.object(cause_global.selectors, "DefaultSelector", selector), \
             patch.object(cause_global.os, "read", side_effect=read):
            row = observe.capture_global(self.root / "privileged")
        source = row["sources"]["slabinfo"]
        self.assertTrue(row["required_fields_available"])
        self.assertEqual(source["reader"], "fixed-path sudo cat")
        self.assertFalse(source["direct_attempt"]["available"])
        self.assertEqual(popen.call_args.args[0], ["sudo", "-n", "cat", "/proc/slabinfo"])
        self.assertEqual(source["timeout_seconds"], 5)
        self.assertLessEqual(source["before"]["monotonic_ns"], source["after"]["monotonic_ns"])

    def test_privileged_read_never_runs_for_fixture_or_non_hosted_capture(self):
        fixture_proc = self.root / "proc"
        fixture_proc.mkdir()
        denied = dict(available=False, error="permission denied", error_type="PermissionError", errno=13)
        with patch.object(cause_global.subprocess, "Popen") as popen:
            with patch.object(cause_global, "_read", return_value=denied):
                row = observe.capture_global(self.root / "fixture-output", proc_root=fixture_proc,
                                             fixture=self.access)
            self.assertFalse(row["required_fields_available"])
            with patch.dict(os.environ, {}, clear=True):
                with self.assertRaises(RuntimeError):
                    observe.capture_global(self.root / "no-host-output")
                with self.assertRaises(RuntimeError):
                    cause_global._sudo_slabinfo()
            popen.assert_not_called()

    def test_privileged_output_cap_and_missing_sudo_fail_explicitly(self):
        proc, selector, read = self.fake_sudo(b"x" * (2 * 1024 * 1024 + 1))
        with patch.dict(os.environ, GITHUB_ACTIONS="true", RUNNER_ENVIRONMENT="github-hosted"), \
             patch.object(cause_global.subprocess, "Popen", return_value=proc), \
             patch.object(cause_global.selectors, "DefaultSelector", selector), \
             patch.object(cause_global.os, "read", side_effect=read):
            row = cause_global._sudo_slabinfo()
        self.assertFalse(row["available"])
        self.assertIn("two-MiB", row["error"])
        self.assertNotIn("raw", row)
        with patch.dict(os.environ, GITHUB_ACTIONS="true", RUNNER_ENVIRONMENT="github-hosted"), \
             patch.object(cause_global.subprocess, "Popen", side_effect=FileNotFoundError("no sudo")):
            missing = cause_global._sudo_slabinfo()
        self.assertFalse(missing["available"])
        self.assertEqual(missing["error_type"], "FileNotFoundError")
        proc, selector, read = self.fake_sudo()
        with patch.dict(os.environ, GITHUB_ACTIONS="true", RUNNER_ENVIRONMENT="github-hosted"), \
             patch.object(cause_global.subprocess, "Popen", return_value=proc), \
             patch.object(cause_global.selectors, "DefaultSelector", selector), \
             patch.object(cause_global.time, "monotonic", side_effect=[10, 16]):
            timeout = cause_global._sudo_slabinfo()
        self.assertFalse(timeout["available"])
        self.assertIn("five-second deadline", timeout["error"])
        self.assertNotIn("raw", timeout)

    def mock_cgroup(self, membership="0::/worker\n"):
        proc = self.root / "proc"
        pid = proc / "123"
        pid.mkdir(parents=True)
        (pid / "cgroup").write_text(membership)
        # State is field 3; starttime is field 22 (index 19 after comm).
        (pid / "stat").write_text("123 (worker with spaces) S " + " ".join(["0"] * 18 + ["777"] + ["0"] * 3))
        cg = self.root / "cgroup"
        worker = cg / "worker"
        worker.mkdir(parents=True)
        (worker / "memory.current").write_text("4096\n")
        (worker / "memory.stat").write_text("anon 2048\nfile 2048\n")
        (worker / "cgroup.procs").write_text("123\n")
        return proc, cg

    def test_cgroup_resolves_host_membership_without_namespace_fallback(self):
        proc, cg = self.mock_cgroup()
        row = observe.capture_cgroup(123, proc_root=proc, cgroup_root=cg, fixture=self.access)
        self.assertEqual(row["cgroup_path"], str(cg / "worker"))
        self.assertTrue(row["process_identity_stable"])
        self.assertEqual(row["sources"]["memory.current"]["raw"], "4096\n")
        self.assertFalse(row["sources"]["memory.events.local"]["available"])
        self.assertNotIn("raw", row["sources"]["memory.events.local"])
        self.assertLessEqual(row["before"]["monotonic_ns"], row["after"]["monotonic_ns"])
        self.assertEqual(row["group_identity"]["inode"], (cg / "worker").stat().st_ino)

    def test_unsafe_cgroup_paths_and_physical_symlinks_rejected(self):
        proc, cg = self.mock_cgroup("0::/../outside\n")
        with self.assertRaises(ValueError):
            observe.capture_cgroup(123, proc_root=proc, cgroup_root=cg, fixture=self.access)
        (proc / "123/cgroup").write_text("0::/alias\n")
        (cg / "alias").symlink_to(cg / "worker", target_is_directory=True)
        with self.assertRaises(OSError):
            observe.capture_cgroup(123, proc_root=proc, cgroup_root=cg, fixture=self.access)

    def test_pins_deduplicate_hardlinks_then_observe_natural_unlinks(self):
        previous = self.database("snapshot.prev.db")
        with observe.PinSet(self.snapshot, fixture=self.access) as pins:
            opened = pins.pin_old_previous()
            os.link(previous, self.snapshot / "alias.db")
            stage = self.database("snapshot.stage.tmp")
            pins.pin_stage()
            named = observe.record_inventory(self.snapshot, pins)
            self.assertEqual(len(named["entries"]), 2)
            self.assertEqual(named["allocated_bytes"], previous.stat().st_blocks * 512 + stage.stat().st_blocks * 512)
            self.assertEqual(opened["opened"]["inode"], previous.stat().st_ino)
            self.assertFalse(os.get_inheritable(pins.fd("stage")))
            stage.unlink()
            previous.unlink()
            (self.snapshot / "alias.db").unlink()
            self.database("snapshot.prev.db")
            unlinked = observe.record_inventory(self.snapshot, pins)
            self.assertTrue(all(p["naturally_unlinked"] for p in unlinked["pins"].values()))
            self.assertEqual(unlinked["pins"]["previous"]["path_state"], "replaced")
            self.assertEqual(unlinked["pins"]["stage"]["path_state"], "absent")
            old_fd = pins.fd("stage")
            stage_db = observe.db_breakdown(old_fd, fixture=self.access)
            # Linux SQLite may canonicalize proc-fd to the deleted pathname;
            # record that limitation instead of inventing absent page totals.
            if not stage_db["available"]:
                self.assertTrue(stage_db["errors"])
                self.assertNotIn("dbstat", stage_db)
            self.assertTrue(stage_db["query_connection_closed"])
            self.assertEqual(stage_db["source_identity"]["inode"], unlinked["pins"]["stage"]["observation"]["inode"])
            pins.close_stage()
            with self.assertRaises(OSError):
                os.fstat(old_fd)
            released = observe.release_measurement(pins, "previous", seconds=0)
            self.assertTrue(released["requires_unlinked"])
            self.assertEqual(len(released["comparisons"]), 1)
            self.assertEqual(observe.settling_samples(pins, seconds=0)["quantization_bytes"], os.statvfs(self.snapshot).f_frsize)

    def test_pin_symlink_fifo_and_existing_stage_are_rejected(self):
        stage = self.snapshot / "snapshot.stage.tmp"
        stage.touch()
        with self.assertRaises(ValueError):
            observe.PinSet(self.snapshot, fixture=self.access)
        stage.unlink()
        with observe.PinSet(self.snapshot, fixture=self.access) as pins:
            (self.snapshot / "snapshot.prev.db").symlink_to(self.root / "missing")
            with self.assertRaises(OSError):
                pins.pin_old_previous()
            (self.snapshot / "snapshot.prev.db").unlink()
            os.mkfifo(self.snapshot / "snapshot.prev.db")
            with self.assertRaises(ValueError):
                pins.pin_old_previous()
            with self.assertRaises(FileNotFoundError):
                pins.pin_stage()

    def test_inspection_aliases_retain_payload_once_and_are_removed_before_release(self):
        previous = self.database("snapshot.prev.db")
        with observe.PinSet(self.snapshot, fixture=self.access, inspection_aliases=True) as pins:
            pins.pin_old_previous()
            stage = self.database("snapshot.stage.tmp")
            pins.pin_stage()
            paths = pins.inspection_paths()
            alias_dir = Path(paths["stage"]).parent
            self.assertEqual(alias_dir.parent, self.snapshot.parent)
            for name in ("stage", "previous"):
                self.assertEqual(Path(paths[name]).stat().st_ino, os.fstat(pins.fd(name)).st_ino)
            original = observe.record_inventory(self.snapshot, pins)
            self.assertEqual(len(original["entries"]), 2)
            stage.unlink()
            previous.unlink()
            retained = observe.record_inventory(self.snapshot, pins)
            self.assertEqual(retained["allocated_bytes"], original["allocated_bytes"])
            for name, row in retained["pins"].items():
                self.assertTrue(row["app_path_retired"])
                self.assertFalse(row["naturally_unlinked"])
                self.assertEqual(row["observation"]["nlink"], 1)
                db = observe.db_breakdown(paths[name], fixture=self.access)
                self.assertTrue(db["available"], db)
                self.assertTrue(db["query_connection_closed"])
                self.assertEqual(db["source_identity"]["inode"], row["observation"]["inode"])
            with self.assertRaises(ValueError):
                pins.close_stage()
            self.assertTrue(pins.drop_inspection_aliases()["available"])
            self.assertFalse(alias_dir.exists())
            unlinked = observe.record_inventory(self.snapshot, pins)
            self.assertTrue(all(row["naturally_unlinked"] for row in unlinked["pins"].values()))
            for name in ("stage", "previous"):
                released = observe.release_measurement(pins, name, seconds=0)
                self.assertEqual(released["expected_inode_bytes"], unlinked["pins"][name]["observation"]["allocated_bytes"])

    def test_pin_context_cleans_only_owned_aliases_on_error(self):
        self.database("snapshot.prev.db")
        pins = observe.PinSet(self.snapshot, fixture=self.access, inspection_aliases=True)
        pins.pin_old_previous()
        path = Path(pins.inspection_paths()["previous"])
        held_fd = pins.fd("previous")
        path.unlink()
        path.write_bytes(b"foreign replacement")
        pins.close()
        self.assertEqual(path.read_bytes(), b"foreign replacement")
        with self.assertRaises(OSError):
            os.fstat(held_fd)

    def test_inventory_bound_and_symlink_exclusions_are_explicit(self):
        with observe.PinSet(self.snapshot, fixture=self.access) as pins:
            (self.snapshot / "a").touch()
            (self.snapshot / "b").touch()
            row = observe.record_inventory(self.snapshot, pins, max_entries=1)
            self.assertFalse(row["complete"])
            self.assertIn("bound exceeded", row["errors"][0]["error"])
            (self.snapshot / "b").unlink()
            (self.snapshot / "symbolic").symlink_to(self.root / "outside")
            row = observe.record_inventory(self.snapshot, pins)
            self.assertFalse(row["complete"])
            self.assertEqual(row["excluded"][0]["path"], "symbolic")
            with self.assertRaises(ValueError):
                observe.record_inventory(self.root, pins)

    def test_vanishing_entry_stat_or_subdirectory_open_is_a_race(self):
        vanished = self.snapshot / "vanished"
        vanished.mkdir()
        directory_stat = vanished.stat()
        class Scan:
            def __init__(self, entry): self.entry = entry
            def __enter__(self): return iter([self.entry])
            def __exit__(self, *unused): pass
        with observe.PinSet(self.snapshot, fixture=self.access) as pins:
            for phase in ("stat", "subdirectory_open"):
                with self.subTest(phase=phase):
                    def entry_stat(**unused):
                        if phase == "stat":
                            raise FileNotFoundError("entry disappeared before stat")
                        return directory_stat
                    original_open = os.open
                    def open_directory(path, *args, **kwargs):
                        if path == "vanished":
                            raise FileNotFoundError("directory disappeared before open")
                        return original_open(path, *args, **kwargs)
                    entry = SimpleNamespace(name="vanished", stat=entry_stat)
                    with patch.object(observe.os, "scandir", return_value=Scan(entry)), \
                         patch.object(observe.os, "open", side_effect=open_directory):
                        row = observe.record_inventory(self.snapshot, pins)
                    self.assertFalse(row["complete"])
                    self.assertEqual(len(row["errors"]), 1)
                    self.assertTrue(row["errors"][0]["race"])
                    self.assertEqual(row["errors"][0]["path"], "vanished")

    def test_dbstat_categories_and_nonempty_wal_rejection(self):
        path = self.database("snapshot.db")
        row = observe.db_breakdown(path, fixture=self.access)
        self.assertTrue(row["available"], row)
        self.assertEqual(row["sqlite_version"], sqlite3.sqlite_version)
        self.assertEqual(row["pragmas"]["auto_vacuum"], 0)
        self.assertEqual((row["header"]["write_version"], row["header"]["read_version"]), (1, 1))
        self.assertEqual(row["header"]["bytes_read"], 100)
        self.assertTrue(row["source_identity_stable"])
        self.assertTrue(row["query_connection_closed"])
        self.assertTrue(row["read_only"])
        names = {r[0] for r in row["dbstat"]}
        self.assertTrue({"entries", "names"}.issubset(names))
        self.assertGreater(row["pragmas"]["page_count"], 0)
        self.assertEqual(sum(r[3] for r in row["dbstat"]), row["pragmas"]["page_count"] * row["pragmas"]["page_size"])
        (self.snapshot / "snapshot.db-wal").write_bytes(b"not-safe")
        with self.assertRaises(ValueError):
            observe.db_breakdown(path, fixture=self.access)

    def test_db_errors_and_fixture_row_limit_are_not_zero_filled(self):
        broken = self.snapshot / "broken.db"
        broken.write_bytes(b"not a sqlite database")
        row = observe.db_breakdown(broken, fixture=self.access)
        self.assertFalse(row["available"])
        self.assertTrue(row["errors"])
        self.assertNotIn("pragmas", row)
        maximum = self.database("maximum.db", rows=64)
        self.assertTrue(observe.db_breakdown(maximum, fixture=self.access)["available"])

    def test_db_source_metadata_change_is_reported_after_connection_close(self):
        path = self.database("snapshot.db")
        original = sqlite3.connect
        class ClosingConnection:
            def __init__(self, conn): self.conn = conn
            def __getattr__(self, name): return getattr(self.conn, name)
            def close(self):
                self.conn.close()
                old = path.stat()
                os.utime(path, ns=(old.st_atime_ns, old.st_mtime_ns + 1000000))
        def connect(*args, **kwargs):
            return ClosingConnection(original(*args, **kwargs))
        with patch.object(observe.sqlite3, "connect", side_effect=connect):
            row = observe.db_breakdown(path, fixture=self.access)
        self.assertFalse(row["available"])
        self.assertFalse(row["source_identity_stable"])
        self.assertTrue(row["query_connection_closed"])
        self.assertTrue(any("changed during inspection" in message for message in row["errors"]))


if __name__ == "__main__":
    unittest.main()
