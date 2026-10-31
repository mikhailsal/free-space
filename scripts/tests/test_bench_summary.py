"""Summary completeness tests using small JSON artifacts; no scanner workloads."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "summarize-v2-container.py"
SPEC = importlib.util.spec_from_file_location("bench_summary", SCRIPT)
summary = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(summary)


class SummaryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.arm = "0001-candidate-v2-100000-keep1-r1"
        self.command = self.arm + "-0-scan"
        self.write("manifest.json", {
            "measurement_protocol": "host-shared-fixture-cold-arm-integrity-v4"})
        self.write("status.json", {"success": True, "completed": True, "failures": []})
        self.write(self.arm + ".spec.json", {"id": self.arm, "operations": ["scan"]})
        self.write(self.arm + ".arm-end.json", {"memory.events": "oom 0\noom_kill 0"})
        self.state = {"Status": "exited", "Running": False, "OOMKilled": False,
                      "ExitCode": 0, "Error": ""}
        self.write(self.arm + ".docker-state.json", self.state)
        self.metrics = {"id": self.command, "arm": "candidate-v2", "files": 100000,
                        "keep_previous": True, "operation": "scan", "elapsed_seconds": 1.0,
                        "validated": True, "cli_validated": True, "observer_validated": True,
                        "exit_code": 0, "timed_out": False, "sampler_errors": []}
        self.write(self.command + "/metrics.json", self.metrics)
        self.observation = {"validated": True, "errors": {}, "logical_content_unchanged": True,
                            "databases": {"snapshot.db": {"dbstat": []}}}
        self.write(self.command + "/dbstat.json", self.observation)

    def write(self, name, value):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value))

    def run_summary(self):
        output = io.StringIO()
        with patch.object(sys, "argv", [str(SCRIPT), str(self.root)]), contextlib.redirect_stdout(output):
            code = summary.main()
        return code, output.getvalue()

    def assert_incomplete(self, message):
        code, text = self.run_summary()
        self.assertEqual(code, 1, text)
        self.assertIn("INVALID / INCOMPLETE RESULTS", text)
        self.assertIn(message, text)
        self.assertIn("| scan profile |", text)

    def test_complete_current_artifact_succeeds(self):
        code, text = self.run_summary()
        self.assertEqual(code, 0, text)
        self.assertIn("All 1 command rows valid", text)

    def test_missing_root_status_rejected(self):
        (self.root / "status.json").unlink()
        self.assert_incomplete("status.json: incomplete/failed experiment")

    def test_unfinished_root_status_rejected(self):
        self.write("status.json", {"success": True, "completed": False, "failures": []})
        self.assert_incomplete("incomplete/failed experiment")

    def test_cli_success_before_observer_completion_rejected(self):
        del self.metrics["observer_validated"]
        self.write(self.command + "/metrics.json", self.metrics)
        self.assert_incomplete("incomplete/failed observer")

    def test_missing_observer_artifact_rejected(self):
        (self.root / self.command / "dbstat.json").unlink()
        self.assert_incomplete("incomplete/failed observer")

    def test_observer_database_error_rejected_even_with_true_flags(self):
        self.observation["databases"]["snapshot.db"]["dbstat_error"] = "interrupted"
        self.write(self.command + "/dbstat.json", self.observation)
        self.assert_incomplete("incomplete/failed observer")

    def test_missing_arm_end_rejected(self):
        (self.root / (self.arm + ".arm-end.json")).unlink()
        self.assert_incomplete("missing/invalid final cgroup state")

    def test_missing_docker_state_rejected(self):
        (self.root / (self.arm + ".docker-state.json")).unlink()
        self.assert_incomplete("missing/failed final Docker state")

    def test_running_docker_state_rejected(self):
        self.state.update(Status="running", Running=True)
        self.write(self.arm + ".docker-state.json", self.state)
        self.assert_incomplete("missing/failed final Docker state")

    def test_truncated_metrics_keeps_diagnostics(self):
        (self.root / self.command / "metrics.json").write_text('{"validated":')
        self.assert_incomplete("incomplete command metrics")

    def test_legacy_observer_uses_integrity_and_database_evidence(self):
        self.write("manifest.json", {"measurement_protocol": "host-setup-cold-arm-integrity-v3"})
        del self.metrics["observer_validated"]
        del self.observation["validated"]
        self.write(self.command + "/metrics.json", self.metrics)
        self.write(self.command + "/dbstat.json", self.observation)
        code, text = self.run_summary()
        self.assertEqual(code, 0, text)


if __name__ == "__main__":
    unittest.main()
