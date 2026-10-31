import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

PATH = Path(__file__).resolve().parents[1] / 'bench_v1_summary.py'
SPEC = importlib.util.spec_from_file_location('bench_v1_summary', PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def put(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data))


def fixture(root):
    sha = 'a' * 64
    manifest = {'measurement_protocol': MODULE.PROTOCOL, 'expected_arms': ['one', 'two'], 'pairs': 1,
                'files': 10, 'operation': 'refresh', 'fixture_receipt': {'expected_entries': 12},
                'provenance': {arm: {'sha256': sha, 'source_revision': 'revision'} for arm in MODULE.ARMS}}
    put(root / 'manifest.json', manifest)
    put(root / 'status.json', {'success': True, 'completed': True, 'failures': [], 'cleanup_success': True})
    put(root / 'capacity-admission.json', {'success': True})
    cg = {'memory.events': 'low 0\noom 0\noom_kill 0\n', 'memory.max': str(256 * 1024 * 1024),
          'memory.swap.max': '0'}
    state = {'Status': 'exited', 'ExitCode': 0, 'Running': False, 'OOMKilled': False, 'Error': ''}
    for index, ident in enumerate(('one', 'two')):
        spec = {'id': ident, 'arm': MODULE.ARMS[index], 'pair': 1, 'operation': 'refresh', 'backend': 'v1',
                'files': 10, 'memory_mib': 256, 'expected_entries': 12, 'expected_subtree_entries': 5, 'seed_sha256': sha,
                'binary_sha256': sha, 'source_revision': 'revision'}
        metric = {**spec, 'validated': True, 'exit_code': 0, 'timed_out': False, 'sampler_errors': [],
                  'report': {'entries': 12, 'subtree_entries': 5, 'errors': 0, 'incomplete': False}, 'elapsed_seconds': 1 + index,
                  'time': {'peak_rss_kib': 1024 + index}, 'app_disk_peak_bytes': 1024 ** 3 + index,
                  'memory_peak_scope': MODULE.SCOPE, 'cgroup_after': cg}
        observer = {'validated': True, 'logical_content_unchanged': True, 'semantic_sha256': sha,
                    'current_entries': 12, 'previous_entries': 12, 'resources': {'elapsed_seconds': 0.1, 'cgroup': cg},
                    'errors': {}, 'seed_sha256': sha}
        for name, data in [('spec.json', spec), ('metrics.json', metric), ('observer.json', observer),
                           ('cache-flush.json', {'success': True}), ('writer-state.json', state),
                           ('observer-state.json', state)]:
            put(root / ident / name, data)


class SummaryTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        fixture(self.root)

    def change(self, path, key, value):
        file = self.root / path
        obj = json.loads(file.read_text())
        obj[key] = value
        put(file, obj)

    def test_valid_pair_and_cli(self):
        result = MODULE.summarize(self.root)
        self.assertTrue(result['valid'], result['issues'])
        self.assertEqual(result['pairs'][0]['elapsed_seconds']['delta'], 1)
        self.assertEqual(result['summary']['elapsed_seconds']['max_abs_delta'], 1)
        self.assertTrue(result['identical_binaries'])
        self.assertEqual(MODULE.main([str(self.root)]), 0)
        self.assertTrue(json.loads((self.root / 'summary.json').read_text())['valid'])

    def test_failure_evidence(self):
        cases = [('status.json', 'cleanup_success', False), ('capacity-admission.json', 'success', False),
                 ('one/metrics.json', 'cgroup_after', {'memory.events': 'oom 1\noom_kill 0',
                  'memory.max': str(256 * 1024 * 1024), 'memory.swap.max': '0'}),
                 ('one/metrics.json', 'cgroup_after', {'memory.events': 'oom 0\noom_kill 0',
                  'memory.max': '1', 'memory.swap.max': '0'}),
                 ('one/metrics.json', 'report', {'entries': 12, 'errors': 1, 'incomplete': False}),
                 ('one/metrics.json', 'id', 'two'), ('one/metrics.json', 'seed_sha256', 'b' * 64),
                 ('one/metrics.json', 'report', {'entries': 12, 'subtree_entries': 4, 'errors': 0, 'incomplete': False}),
                 ('one/metrics.json', 'timed_out', True), ('one/metrics.json', 'sampler_errors', ['error']),
                 ('one/observer.json', 'semantic_sha256', 'b' * 64), ('one/observer.json', 'validated', False),
                 ('one/observer.json', 'logical_content_unchanged', False),
                 ('one/observer.json', 'resources', {'elapsed_seconds': 1, 'cgroup': {'memory.events': 'oom 1\noom_kill 0'}}),
                 ('one/spec.json', 'seed_sha256', 'b' * 64),
                 ('one/spec.json', 'binary_sha256', 'b' * 64),
                 ('one/writer-state.json', 'OOMKilled', True), ('one/cache-flush.json', 'success', False)]
        for path, key, value in cases:
            with self.subTest(path=path, key=key):
                fixture(self.root)
                self.change(path, key, value)
                self.assertFalse(MODULE.summarize(self.root)['valid'])

    def test_missing_extra_and_unsafe_artifacts(self):
        (self.root / 'one' / 'observer.json').unlink()
        self.assertFalse(MODULE.summarize(self.root)['valid'])
        fixture(self.root)
        (self.root / 'extra').mkdir()
        self.assertFalse(MODULE.summarize(self.root)['valid'])
        self.change('manifest.json', 'expected_arms', ['../outside', 'two'])
        self.assertFalse(MODULE.summarize(self.root)['valid'])

    def test_malformed_cgroup_and_duplicate_pair(self):
        self.change('one/metrics.json', 'cgroup_after', {'memory.events': 'oom 0'})
        self.assertFalse(MODULE.summarize(self.root)['valid'])
        fixture(self.root)
        self.change('two/spec.json', 'arm', 'baseline-v1')
        self.assertFalse(MODULE.summarize(self.root)['valid'])


if __name__ == '__main__':
    unittest.main()
