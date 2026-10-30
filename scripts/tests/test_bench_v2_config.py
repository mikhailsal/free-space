"""Fast benchmark interface checks; no containers, scans, or real cgroups."""

import contextlib
import importlib.util
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch


SCRIPTS = Path(__file__).resolve().parents[1]
with patch.object(sys, "path", [str(SCRIPTS), *sys.path]):
    import bench_v2_config

    SPEC = importlib.util.spec_from_file_location(
        "bench_v2_container", SCRIPTS / "bench-v2-container.py"
    )
    HARNESS = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(HARNESS)


class ArgumentsTest(unittest.TestCase):
    def parse(self, *options):
        argv = ["benchmark", "--candidate", "/unused/binary", "--output", "/unused/results",
                "--current-backends", *options]
        with patch.object(sys, "argv", argv), contextlib.redirect_stderr(io.StringIO()):
            return bench_v2_config.parse_arguments("test")

    def assert_invalid(self, *options):
        with self.assertRaises(SystemExit) as error:
            self.parse(*options)
        self.assertEqual(error.exception.code, 2)

    def test_arbitrary_increasing_steps_and_bounds(self):
        for steps in ("1", "17,12345,7200000", "1,8000000"):
            with self.subTest(steps=steps):
                self.assertEqual(self.parse("--steps", steps).steps, steps)

    def test_invalid_steps(self):
        for steps in ("", "1,", "abc", "1.5", "0", "-1", "8000001", "2,1", "1,1"):
            with self.subTest(steps=steps):
                self.assert_invalid("--steps", steps)

    def test_each_io_limit_requires_device_and_positive_integer(self):
        for option in ("--read-bps", "--write-bps", "--read-iops", "--write-iops"):
            with self.subTest(option=option):
                self.assert_invalid(option, "1")
                for value in ("0", "-1", "1.5"):
                    self.assert_invalid("--io-device", "/dev/test", option, value)
                args = self.parse("--io-device", "/dev/test", option, "1")
                self.assertEqual(getattr(args, option[2:].replace("-", "_")), 1)

    def test_invalid_run_bounds(self):
        for option, value in (("--workers", "0"), ("--cpus", "0"), ("--repeats", "0"),
                              ("--timeout-seconds", "0"), ("--scans", "1"),
                              ("--refreshes", "0")):
            with self.subTest(option=option):
                self.assert_invalid(option, value)


class IoVerificationTest(unittest.TestCase):
    def setUp(self):
        self.spec = {"io_device_number": "7:0", "io_limits": {
            "rbps": 1048576, "wbps": 2097152, "riops": 100, "wiops": 200}}
        self.metrics = {"io.max": "7:0 rbps=1048576 wbps=2097152 riops=100 wiops=200",
                        "io.stat": ""}

    def test_exact_requested_limits_accepted(self):
        HARNESS.verify_io(self.metrics, self.spec)

    def test_other_devices_and_unspecified_unlimited_fields_accepted(self):
        HARNESS.verify_io({"io.max": "8:0 rbps=1\n7:0 rbps=1048576 wbps=max riops=max wiops=max",
                           "io.stat": ""},
                          {"io_device_number": "7:0", "io_limits": {"rbps": 1048576}})

    def test_each_mismatched_or_missing_limit_rejected(self):
        for key, value in self.spec["io_limits"].items():
            for replacement in (f"{key}={value + 1}", f"{key}=max", ""):
                with self.subTest(key=key, replacement=replacement):
                    metrics = dict(self.metrics)
                    metrics["io.max"] = metrics["io.max"].replace(f"{key}={value}", replacement)
                    with self.assertRaises(RuntimeError):
                        HARNESS.verify_io(metrics, self.spec)

    def test_limits_on_wrong_device_rejected(self):
        metrics = dict(self.metrics, **{"io.max": self.metrics["io.max"].replace("7:0", "8:0")})
        with self.assertRaises(RuntimeError):
            HARNESS.verify_io(metrics, self.spec)

    def test_missing_cgroup_observation_rejected(self):
        for missing in ("io.max", "io.stat"):
            with self.subTest(missing=missing):
                metrics = {key: value for key, value in self.metrics.items() if key != missing}
                with self.assertRaises(RuntimeError):
                    HARNESS.verify_io(metrics, self.spec)

    def test_no_limits_needs_no_io_controller(self):
        HARNESS.verify_io({}, {})


if __name__ == "__main__":
    unittest.main()
