"""Admission calculations do not touch disks or start benchmarks."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "bench_v1_admission", Path(__file__).resolve().parents[1] / "bench_v1_admission.py")
ADMISSION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ADMISSION)


class CapacityTests(unittest.TestCase):
    def test_baseline_epoch_resolves_commit_without_shell_interpolation(self):
        commit = 'a' * 40
        with patch.object(ADMISSION.subprocess, 'check_output', side_effect=[commit + '\n', '1700000000\n']) as call:
            self.assertEqual(ADMISSION.baseline_epoch('--odd;$(ignored)'), (commit, 1700000000))
        self.assertEqual(call.call_args_list[0].args[0], ['git', 'rev-parse', '--verify', '--end-of-options', '--odd;$(ignored)^{commit}'])
        self.assertEqual(call.call_args_list[1].args[0], ['git', 'show', '-s', '--format=%ct', commit])
        with patch.object(ADMISSION.subprocess, 'check_output', return_value='not-a-commit\n'), self.assertRaises(ValueError):
            ADMISSION.baseline_epoch('invalid')
        with patch.object(ADMISSION.subprocess, 'check_output', side_effect=[commit, '-1']), self.assertRaises(ValueError):
            ADMISSION.baseline_epoch('invalid-time')

    def test_stage_and_forecast_case_identity(self):
        self.assertEqual(ADMISSION.stage_files('screen'), 1000000)
        case = ADMISSION.forecast_case_id('x86_64-unknown-linux-musl', 'local', ['refresh'], '10000', 'changed', '512')
        self.assertEqual(case, 'x86_64-unknown-linux-musl|local|refresh|10000|changed|512')
        self.assertNotEqual(case, ADMISSION.forecast_case_id('x86_64-unknown-linux-musl', 'local', ['refresh'], '10000', 'changed', '1024'))
        with self.assertRaises(ValueError):
            ADMISSION.forecast_case_id('x86_64-unknown-linux-musl', 'other', ['refresh'], '10000', 'changed', '512')

    def test_qualification_case_whitelist(self):
        for size in ('100', '10000', '100000'):
            for memory in ('512', '1024', '2048', '4096'):
                for scenario in ('unchanged', 'changed'):
                    case = ADMISSION.qualification_case(size, scenario, memory, ['refresh'], 100000)
                    self.assertEqual(case['subtree_entries'], int(size))
                    self.assertEqual(case['memory_mib'], int(memory))
        for args in (('101', 'unchanged', '4096', ['refresh'], 100000),
                     ('100', 'changed', '4096', ['scan', 'refresh'], 100000),
                     ('100', 'unchanged', '256', ['refresh'], 100000),
                     ('100000', 'unchanged', '4096', ['refresh'], 1000)):
            with self.subTest(args=args), self.assertRaises(ValueError):
                ADMISSION.qualification_case(*args)

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
    def test_workflow_forecast_always_writes_strict_rejection_receipt(self):
        root = Path(__file__).resolve().parents[2]
        workflow = (root / '.github/workflows/representative-bench.yml').read_text()
        step = workflow.split('name: Check time admission before local-scale allocation', 1)[1]
        code = textwrap.dedent(step.split("python3 - <<'PY'\n", 1)[1].split('\n          PY', 1)[0])
        case = 'x86_64-unknown-linux-musl|local|refresh|100|unchanged|4096'
        base = dict(FORECAST='100', STEP_CAP='3', TARGET='x86_64-unknown-linux-musl', IO='local',
                    OPERATIONS='refresh', SUBTREE='100', SCENARIO='unchanged', MEMORY='4096', FORECAST_CASE=case)
        for forecast, expected in (('100', True), ('', False), ('abc', False), ('nan', False),
                                   ('inf', False), ('1.7e308', False), ('0', False)):
            with self.subTest(forecast=forecast), tempfile.TemporaryDirectory() as tmp:
                Path(tmp, 'scripts').symlink_to(root / 'scripts', target_is_directory=True)
                env = dict(os.environ, **base)
                env['FORECAST'] = forecast
                run = subprocess.run([sys.executable, '-c', code], cwd=tmp, env=env, capture_output=True, text=True)
                text = Path(tmp, 'representative-results/forecast-admission.json').read_text()
                receipt = json.loads(text, parse_constant=lambda s: self.fail('non-standard JSON: ' + s))
                self.assertEqual(receipt['success'], expected, run.stderr)
                self.assertEqual(run.returncode == 0, expected, run.stderr)
                if not expected:
                    self.assertTrue(receipt.get('error'))
                if forecast in ('', 'abc', 'nan', 'inf'):
                    self.assertIsNone(receipt['estimated_seconds'])
        for override in ({'FORECAST_CASE': 'wrong'}, {'SUBTREE': 'invalid'}, {'STEP_CAP': 'bad'}):
            with self.subTest(override=override), tempfile.TemporaryDirectory() as tmp:
                Path(tmp, 'scripts').symlink_to(root / 'scripts', target_is_directory=True)
                env = dict(os.environ, **base)
                env.update(override)
                run = subprocess.run([sys.executable, '-c', code], cwd=tmp, env=env, capture_output=True, text=True)
                receipt = json.loads(Path(tmp, 'representative-results/forecast-admission.json').read_text())
                self.assertNotEqual(run.returncode, 0)
                self.assertFalse(receipt['success'])
                self.assertTrue(receipt.get('error'))

    def test_benchmark_target(self):
        for target in ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"):
            with self.subTest(target=target):
                self.assertEqual(ADMISSION.benchmark_target(target), target)
        for target in (None, "", "x86_64-unknown-linux-gnu extra", "aarch64-unknown-linux-musl"):
            with self.subTest(target=target), self.assertRaises(ValueError):
                ADMISSION.benchmark_target(target)

    def test_stage_operations(self):
        for stage, expected in (("smoke", ["scan"]), ("screen", ["scan", "refresh"]),
                                ("local-scale", ["scan"])):
            with self.subTest(stage=stage):
                self.assertEqual(ADMISSION.stage_operations(stage), expected)
        self.assertEqual(ADMISSION.stage_operations("local-scale", "refresh"), ["refresh"])
        self.assertEqual(ADMISSION.stage_operations("screen", "scan"), ["scan"])

    def test_stage_operations_rejects_invalid_inputs(self):
        for stage, requested in (("unknown", "auto"), (None, "scan"), ("smoke", None), ("smoke", "unknown"),
                                 ("screen", "scan refresh")):
            with self.subTest(stage=stage, requested=requested), self.assertRaises(ValueError):
                ADMISSION.stage_operations(stage, requested)

    def test_stage_defaults_and_bounded_override(self):
        for stage, cap in (("smoke", 30), ("screen", 65), ("local-scale", 120)):
            for requested in (None, "", cap, str(cap)):
                with self.subTest(stage=stage, requested=requested):
                    self.assertEqual(ADMISSION.stage_timeouts(stage, requested),
                                     dict(step_minutes=cap, job_minutes=cap + 5,
                                          build_minutes=25, maximum_runner_minutes=cap + 30))
        self.assertEqual(ADMISSION.stage_timeouts("local-scale", "47"),
                         dict(step_minutes=47, job_minutes=52, build_minutes=25,
                              maximum_runner_minutes=77))

    def test_stage_timeout_rejects_invalid_inputs(self):
        for stage, requested in (("unknown", None), (None, None), ("smoke", 31),
                                 ("screen", 0), ("local-scale", 121), ("smoke", True),
                                 ("smoke", 1.5), ("smoke", float("nan")),
                                 ("smoke", "1.5"), ("smoke", "NaN"), ("smoke", "-1")):
            with self.subTest(stage=stage, requested=requested), self.assertRaises(ValueError):
                ADMISSION.stage_timeouts(stage, requested)

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
