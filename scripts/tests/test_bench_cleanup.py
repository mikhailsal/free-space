"""Exercise EXIT cleanup with fake Docker/mount commands, never a real device."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "bench-representative.sh"


class CleanupTests(unittest.TestCase):
    def run_cleanup(self, mode="ok", initial_status=0):
        source = SCRIPT.read_text()
        cleanup = source[source.index("cleanup() {"):source.index("run_owned() {")]
        mocks = r'''
set -euo pipefail
output=$TEST_DIRECTORY
volume=$output/volume
mount_dir=$volume/mount
mkdir -p "$mount_dir"
touch "$volume/disk.img"
loop_device=/dev/mock-loop
child_pid=''
if [[ $TEST_MODE == producer-stuck || $TEST_MODE == producer-needs-kill ]]; then child_pid=999999; fi
producer_killed=false
benchmark_label=freespace.benchmark=test-owned-run
touch "$output/mounted"
timeout() { shift 2; "$@"; }
sleep() { :; }
kill() {
  if [[ $1 == -0 ]]; then
    [[ $TEST_MODE == producer-stuck || $producer_killed == false ]]
  else
    echo "kill $*" >> "$output/calls"
    if [[ $1 == -KILL ]]; then producer_killed=true; fi
  fi
}
wait() { echo "wait $*" >> "$output/calls"; }
docker() {
  echo "docker $*" >> "$output/calls"
  if [[ $1 == ps ]]; then
    [[ $TEST_MODE != list-fail ]] || return 1
    if [[ $TEST_MODE != empty ]]; then printf 'owned-a\nowned-b\n'; fi
  else
    [[ $TEST_MODE != remove-fail ]] || return 1
  fi
}
mountpoint() { [[ -f $output/mounted ]]; }
sudo() {
  echo "sudo $*" >> "$output/calls"
  if [[ $1 == umount && $TEST_MODE == umount-fail ]]; then return 1; fi
  if [[ $1 == losetup && $TEST_MODE == detach-fail ]]; then return 1; fi
  if [[ $1 == umount ]]; then rm "$output/mounted"; fi
}
'''
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(
                ["bash", "-c", mocks + cleanup + '\ntrap cleanup EXIT\nexit "$TEST_STATUS"\n'],
                env=dict(os.environ, TEST_DIRECTORY=directory, TEST_MODE=mode,
                         TEST_STATUS=str(initial_status)), capture_output=True, text=True,
                timeout=5)
            calls = (Path(directory) / "calls").read_text().splitlines()
            receipt = (Path(directory) / "cleanup.log").read_text()
            backing_exists = (Path(directory) / "volume/disk.img").exists()
            if mode in ("umount-fail", "detach-fail", "list-fail", "remove-fail", "producer-stuck"):
                self.assertTrue(backing_exists)
            else:
                self.assertFalse(backing_exists)
        return result, calls, receipt

    def test_owned_containers_removed_before_unmount(self):
        result, calls, receipt = self.run_cleanup()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[:3], [
            "docker ps -aq --filter label=freespace.benchmark=test-owned-run",
            "docker rm -f owned-a", "docker rm -f owned-b"])
        self.assertEqual(len(calls), 5)
        self.assertTrue(calls[3].startswith("sudo umount "))
        self.assertEqual(calls[4], "sudo losetup -d /dev/mock-loop")
        self.assertIn("cleanup final_status=0", receipt)

    def test_list_failure_preserves_volume(self):
        result, calls, receipt = self.run_cleanup("list-fail")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(len(calls), 1)
        self.assertIn("Failed to list run-owned containers", receipt)
        self.assertIn("preserving mounted volume", receipt)

    def test_remove_failure_attempts_all_owned_containers_and_preserves_volume(self):
        result, calls, receipt = self.run_cleanup("remove-fail")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(calls[1:], ["docker rm -f owned-a", "docker rm -f owned-b"])
        self.assertIn("preserving mounted volume", receipt)

    def test_normal_empty_cleanup_still_unmounts(self):
        result, calls, _ = self.run_cleanup("empty")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(calls), 3)
        self.assertTrue(calls[1].startswith("sudo umount "))

    def test_failed_unmount_or_detach_preserves_backing(self):
        for mode in ("umount-fail", "detach-fail"):
            with self.subTest(mode=mode):
                result, calls, receipt = self.run_cleanup(mode)
                self.assertEqual(result.returncode, 1)
                self.assertIn("preserving backing file", receipt)
                if mode == "umount-fail":
                    self.assertFalse(any("losetup" in call for call in calls))

    def test_interruption_status_retained(self):
        result, _, receipt = self.run_cleanup(initial_status=143)
        self.assertEqual(result.returncode, 143)
        self.assertIn("cleanup final_status=143", receipt)

    def test_stuck_producer_never_waits_or_unmounts(self):
        result, calls, receipt = self.run_cleanup("producer-stuck")
        self.assertEqual(result.returncode, 1)
        self.assertIn("kill -KILL -- -999999", calls)
        self.assertFalse(any(line.startswith(("wait ", "sudo ")) for line in calls))
        self.assertIn("Producer is still alive", receipt)

    def test_killed_producer_is_reaped_before_container_cleanup(self):
        result, calls, receipt = self.run_cleanup("producer-needs-kill")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(calls.index("kill -KILL -- -999999"), calls.index("wait 999999"))
        self.assertLess(calls.index("wait 999999"), calls.index("docker rm -f owned-a"))
        self.assertIn("sending KILL", receipt)


if __name__ == "__main__":
    unittest.main()
