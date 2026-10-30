"""Runner calibration contracts without Docker, fio, or device access."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "bench_disk_calibration", Path(__file__).resolve().parents[1] / "bench-disk-calibration.py"
)
CALIBRATION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CALIBRATION)
LIMITS = "7:0 rbps=33554432 wbps=16777216 riops=1000 wiops=500"


class DiskCalibrationTest(unittest.TestCase):
    def test_only_ephemeral_github_runner_allowed(self):
        for environment in ({}, {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "self-hosted"},
                            {"GITHUB_ACTIONS": "false", "RUNNER_ENVIRONMENT": "github-hosted"}):
            with self.subTest(environment=environment), patch.dict(os.environ, environment, clear=True), \
                    patch.object(CALIBRATION.subprocess, "check_output") as capture, \
                    patch.object(CALIBRATION.subprocess, "run") as cleanup:
                with self.assertRaisesRegex(RuntimeError, "ephemeral GitHub-hosted"):
                    CALIBRATION.run(argparse.Namespace())
                capture.assert_not_called()
                cleanup.assert_not_called()

    def exercise(self, *, profile="limited", installed=LIMITS,
                 charged="7:0 rbytes=4096 wbytes=4096 rios=1 wios=1", bad_mode=None,
                 fio_error=0, multiplier=1, expected_error=None):
        real_stat = os.stat
        commands = []

        def fake_stat(path, *args, **kwargs):
            if path == "/dev/calibration-test":
                return SimpleNamespace(st_rdev=os.makedev(7, 0))
            return real_stat(path, *args, **kwargs)

        def capture(command, **kwargs):
            commands.append(command)
            if command[:2] == ["docker", "run"]:
                return "mock-container-id"
            if command[-1] == "/sys/fs/cgroup/io.max":
                return installed
            if command[-1] == "/sys/fs/cgroup/io.stat":
                return charged
            if command[:2] == ["docker", "exec"] and command[3] == "fio":
                mode = next(value.split("=", 1)[1] for value in command if value.startswith("--rw="))
                job = {"error": fio_error if mode == bad_mode else 0,
                       "read": {"bw_bytes": 33554432, "iops": 1000},
                       "write": {"bw_bytes": 16777216, "iops": 500}}
                if mode == bad_mode:
                    direction = "write" if "write" in mode else "read"
                    metric = "iops" if mode.startswith("rand") else "bw_bytes"
                    job[direction][metric] *= multiplier
                return json.dumps({"jobs": [job]})
            self.fail(f"Unexpected subprocess command: {command}")

        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            scratch_file = folder / "calibration.bin"
            scratch_file.write_bytes(b"mock fixture")
            args = argparse.Namespace(output=temporary, directory=temporary,
                                      device="/dev/calibration-test", image="mock-image", profile=profile)
            with patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted"}), \
                    patch.object(CALIBRATION.os, "stat", side_effect=fake_stat), \
                    patch.object(CALIBRATION.subprocess, "check_output", side_effect=capture), \
                    patch.object(CALIBRATION.subprocess, "run") as cleanup:
                if expected_error:
                    with self.assertRaisesRegex(RuntimeError, expected_error):
                        CALIBRATION.run(args)
                else:
                    CALIBRATION.run(args)
                cleanup.assert_called_once()
                self.assertEqual(cleanup.call_args.args[0][:3], ["docker", "rm", "-f"])
            report = json.loads((folder / "disk-calibration.json").read_text())
            self.assertFalse(scratch_file.exists())
            self.assertEqual(report["success"], expected_error is None)
            if expected_error:
                self.assertIn("error", report)
            else:
                self.assertEqual(len(report["jobs"]), 4)
                self.assertEqual(len(list(folder.glob("fio-*.json"))), 4)
            return report, commands

    def test_limited_profile_succeeds_and_installs_all_limits(self):
        report, commands = self.exercise()
        self.assertEqual(report["device"], "7:0")
        for flag in ("read-bps", "write-bps", "read-iops", "write-iops"):
            self.assertIn("--device-" + flag, commands[0])

    def test_native_profile_has_no_limits_or_device_accounting_requirement(self):
        report, commands = self.exercise(profile="native", installed="", charged="",
                                         bad_mode="write", multiplier=10)
        self.assertEqual(report["requested_limits"], {})
        self.assertFalse(any(part.startswith("--device-") for part in commands[0]))

    def test_installed_limit_mismatch_rejected_before_fio(self):
        for installed in ("", LIMITS.replace("500", "501"), LIMITS.replace("7:0", "8:0")):
            with self.subTest(installed=installed):
                report, _ = self.exercise(installed=installed, expected_error="did not install")
                self.assertEqual(report["jobs"], [])

    def test_each_fio_limit_exceeded_rejected(self):
        for mode in ("read", "write", "randread", "randwrite"):
            with self.subTest(mode=mode):
                self.exercise(bad_mode=mode, multiplier=2, expected_error="disk envelope probe failed")

    def test_fio_error_rejected_even_under_limit(self):
        self.exercise(bad_mode="write", fio_error=5, expected_error="disk envelope probe failed")

    def test_missing_or_uncharged_device_rejected(self):
        for charged in ("", "8:0 rbytes=4096", "7:0 rbytes=0 wbytes=0 rios=0 wios=0"):
            with self.subTest(charged=charged):
                self.exercise(charged=charged, expected_error="no observed I/O")


if __name__ == "__main__":
    unittest.main()
