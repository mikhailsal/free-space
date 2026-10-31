import importlib.util
import json
import sys
from pathlib import Path
import tempfile
import unittest

PATH = Path(__file__).resolve().parents[1] / 'bench_v1_summary.py'
sys.path.insert(0, str(PATH.parent))
SPEC = importlib.util.spec_from_file_location('bench_v1_summary', PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def put(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data))


def fixture(root):
    sha = 'a' * 64
    manifest = {'measurement_protocol': MODULE.LEGACY_PROTOCOL, 'expected_arms': ['one', 'two'], 'pairs': 1,
                'files': 10, 'operation': 'refresh', 'fixture_receipt': {'expected_entries': 12},
                'provenance': {arm: {'sha256': sha, 'source_revision': 'revision'} for arm in MODULE.ARMS}}
    put(root / 'manifest.json', manifest)
    put(root / 'status.json', {'success': True, 'completed': True, 'failures': [], 'cleanup_success': True})
    put(root / 'capacity-admission.json', {'success': True})
    put(root / 'runner-cleanup.json', {'success': True})
    cg = {'memory.events': 'low 0\noom 0\noom_kill 0\n', 'memory.max': str(256 * 1024 * 1024),
          'memory.swap.max': '0'}
    state = {'Status': 'exited', 'ExitCode': 0, 'Running': False, 'OOMKilled': False, 'Error': ''}
    for index, ident in enumerate(('one', 'two')):
        spec = {'id': ident, 'arm': MODULE.ARMS[index], 'pair': 1, 'operation': 'refresh', 'backend': 'v1',
                'files': 10, 'memory_mib': 256, 'expected_entries': 12, 'expected_subtree_entries': 5, 'seed_sha256': sha,
                'binary_sha256': sha, 'source_revision': 'revision'}
        metric = {**spec, 'validated': True, 'exit_code': 0, 'timed_out': False, 'sampler_errors': [],
                  'report': {'entries': 12, 'subtree_entries': 5, 'incomplete': False, 'unknown_subtrees': []}, 'elapsed_seconds': 1 + index,
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

    def test_changed_full_manifest_and_observer_reject_mismatches(self):
        from unittest.mock import patch
        sys.path.insert(0, str(PATH.parent))
        from bench_v1_fixture import prepare, mutate_full
        with tempfile.TemporaryDirectory() as temp, patch('bench_v1_fixture.os.sync'):
            tree = Path(temp) / 'tree'
            actual = prepare(tree, 100, 100, 'changed-full')
            mutation = mutate_full(tree)
        plan = actual['mutation_plan']
        actual['expected_entries'] = 12  # The synthetic observer fixture has 12 entries.
        def link(prefix, flags, allocated, size, nlink):
            return [flags, allocated, size, 1700000000, nlink, plan[prefix + '_dev'], plan[prefix + '_inode'], plan[prefix + '_allocated'], plan[prefix + '_size']]
        old = link('old_owner', 0, plan['old_owner_allocated'], plan['old_owner_size'], 2)
        alias = link('old_owner', 2, 0, 0, 2)
        owner = [0, plan['outside_owner_allocated'], plan['outside_owner_size'], 1700000000, 1, None, None, None, None]
        seed = {plan['old_owner_path']: old, plan['old_outside_path']: alias, plan['new_owner_path']: owner}
        current = {plan['old_outside_path']: old, plan['new_owner_path']: owner,
                   plan['new_link_path']: link('outside_owner', 2, 0, 0, 2)}
        manifest = json.loads((self.root / 'manifest.json').read_text())
        manifest.update(measurement_protocol=MODULE.PROTOCOL, case_policy='qualification-v1', scenario='changed-full',
                        subtree_entries=100, memory_mib=512, setup_memory_mib=4096, observer_memory_mib=4096,
                        fixture_receipt=actual, mutation_receipt=mutation,
                        seed_link_facts={'snapshot.db': seed, 'snapshot.prev.db': seed})
        put(self.root / 'manifest.json', manifest)
        for arm in ('one', 'two'):
            spec = json.loads((self.root / arm / 'spec.json').read_text())
            spec.update(expected_subtree_entries=100, memory_mib=512, observer_memory_mib=4096, scenario='changed-full', mutation_plan=plan)
            put(self.root / arm / 'spec.json', spec)
            metric = json.loads((self.root / arm / 'metrics.json').read_text())
            metric['memory_mib'] = 512; metric['report']['subtree_entries'] = 100
            metric['cgroup_after']['memory.max'] = str(512 * 1024 * 1024)
            put(self.root / arm / 'metrics.json', metric)
            observer = json.loads((self.root / arm / 'observer.json').read_text())
            observer['resources']['cgroup']['memory.max'] = str(4096 * 1024 * 1024)
            observer['full_link_facts'] = {'previous': seed, 'current': current}
            put(self.root / arm / 'observer.json', observer)
        self.assertTrue(MODULE.summarize(self.root)['valid'])
        for path, key, value in [('manifest.json', 'mutation_receipt', {'recipe': 'rename-size-mtime'}),
                                 ('manifest.json', 'fixture_receipt', {'expected_entries': 12, 'expected_subtree_entries': 100})]:
            original = json.loads((self.root / path).read_text())
            self.change(path, key, value)
            self.assertFalse(MODULE.summarize(self.root)['valid'])
            put(self.root / path, original)
        def missing_seed(obj):
            del obj['seed_link_facts']
        def missing_previous(obj):
            del obj['seed_link_facts']['snapshot.prev.db']
        def extra_seed_snapshot(obj):
            obj['seed_link_facts']['snapshot.extra.db'] = obj['seed_link_facts']['snapshot.db']
        def wrong_seed_alias(obj):
            obj['seed_link_facts']['snapshot.db'][plan['old_outside_path']][0] = 0
        def wrong_previous(obj):
            obj['full_link_facts']['previous'][plan['old_outside_path']][0] = 0
        def wrong_plan(obj):
            obj['mutation_plan']['old_owner_inode'] += 1
        def wrong_current(index, value):
            def change(obj):
                obj['full_link_facts']['current'][plan['new_link_path']][index] = value
            return change
        for path, mutate in [('manifest.json', missing_seed), ('manifest.json', missing_previous),
                             ('manifest.json', extra_seed_snapshot),
                             ('manifest.json', wrong_seed_alias), ('one/observer.json', wrong_previous),
                             ('one/spec.json', wrong_plan),
                             ('one/observer.json', wrong_current(1, 1)),
                             ('one/observer.json', wrong_current(2, 1)),
                             ('one/observer.json', wrong_current(4, 1)),
                             ('one/observer.json', wrong_current(5, plan['old_owner_dev'] + 1)),
                             ('one/observer.json', wrong_current(6, plan['old_owner_inode'])),
                             ('one/observer.json', wrong_current(8, plan['old_owner_size']))]:
            with self.subTest(path=path, mutation=mutate):
                file = self.root / path
                original = json.loads(file.read_text())
                changed = json.loads(file.read_text())
                mutate(changed)
                put(file, changed)
                self.assertFalse(MODULE.summarize(self.root)['valid'])
                put(file, original)
        with self.subTest(mutation='both arms and current evidence agree on a different plan'):
            originals = {}
            for arm in ('one', 'two'):
                spec_path = self.root / arm / 'spec.json'
                observer_path = self.root / arm / 'observer.json'
                for file in (spec_path, observer_path):
                    originals[file] = json.loads(file.read_text())
                spec = json.loads(spec_path.read_text())
                observer = json.loads(observer_path.read_text())
                spec['mutation_plan']['old_owner_inode'] += 2 ** 40
                observer['full_link_facts']['current'][plan['old_outside_path']][6] = spec['mutation_plan']['old_owner_inode']
                self.assertTrue(MODULE.full_link_evidence_ok(observer['full_link_facts']['current'], spec['mutation_plan'], 'refresh'))
                put(spec_path, spec)
                put(observer_path, observer)
            self.assertFalse(MODULE.summarize(self.root)['valid'])
            for file, original in originals.items():
                put(file, original)
        self.change('one/observer.json', 'full_link_facts', {'previous': seed, 'current': {}})
        self.assertFalse(MODULE.summarize(self.root)['valid'])

    def test_runner_preparation_policy_requires_valid_context(self):
        self.assertTrue(MODULE.summarize(self.root)['valid'])  # Historical receipt has no policy.
        self.change('manifest.json', 'runner_preparation_policy', 'sdk-space-v1')
        self.assertFalse(MODULE.summarize(self.root)['valid'])
        valid = {'success': True, 'acted': True, 'freed_bytes': -4096, 'duration_seconds': 1.5,
                 'floor_bytes': 42 * 1024**3, 'target_bytes': 44 * 1024**3}
        self.change('manifest.json', 'runner_preparation', valid)
        self.assertTrue(MODULE.summarize(self.root)['valid'])
        for key, value in (('success', False), ('acted', 1), ('freed_bytes', True), ('freed_bytes', '0'),
                           ('duration_seconds', float('nan')), ('floor_bytes', 0), ('target_bytes', 42 * 1024**3)):
            with self.subTest(key=key, value=value):
                self.change('manifest.json', 'runner_preparation', dict(valid, **{key: value}))
                self.assertFalse(MODULE.summarize(self.root)['valid'])
        manifest = json.loads((self.root / 'manifest.json').read_text())
        del manifest['runner_preparation_policy']
        manifest['runner_preparation'] = {'bad': True}
        put(self.root / 'manifest.json', manifest)
        self.assertFalse(MODULE.summarize(self.root)['valid'])

    def test_qualification_case_rejects_missing_and_mismatched_bindings(self):
        self.change('manifest.json', 'measurement_protocol', MODULE.PROTOCOL)
        self.change('manifest.json', 'case_policy', 'qualification-v1')
        self.change('manifest.json', 'scenario', 'unchanged')
        self.change('manifest.json', 'setup_memory_mib', 4096)
        self.change('manifest.json', 'observer_memory_mib', 4096)
        self.change('manifest.json', 'mutation_receipt', None)
        self.change('manifest.json', 'fixture_receipt', {'expected_entries': 12, 'expected_subtree_entries': 100})
        for subtree, memory in ((None, 512), (100, None), (True, 512), (100, 256)):
            self.change('manifest.json', 'subtree_entries', subtree)
            self.change('manifest.json', 'memory_mib', memory)
            self.assertFalse(MODULE.summarize(self.root)['valid'])
        self.change('manifest.json', 'subtree_entries', 100)
        self.change('manifest.json', 'memory_mib', 512)
        self.assertFalse(MODULE.summarize(self.root)['valid'])  # Both specs disagree.
        for arm in ('one', 'two'):
            self.change(f'{arm}/spec.json', 'expected_subtree_entries', 100)
            self.change(f'{arm}/spec.json', 'memory_mib', 512)
            self.change(f'{arm}/spec.json', 'observer_memory_mib', 4096)
            self.change(f'{arm}/spec.json', 'scenario', 'unchanged')
            self.change(f'{arm}/metrics.json', 'memory_mib', 512)
            observer = json.loads((self.root / arm / 'observer.json').read_text())
            observer['resources']['cgroup']['memory.max'] = str(4096 * 1024 * 1024)
            put(self.root / arm / 'observer.json', observer)
            metric = json.loads((self.root / arm / 'metrics.json').read_text())
            metric['report']['subtree_entries'] = 100
            metric['cgroup_after']['memory.max'] = str(512 * 1024 * 1024)
            put(self.root / arm / 'metrics.json', metric)
        self.assertTrue(MODULE.summarize(self.root)['valid'])
        self.assertEqual(MODULE.summarize(self.root)['measurement_protocol'], MODULE.PROTOCOL)
        observer = json.loads((self.root / 'one' / 'observer.json').read_text())
        observer['resources']['cgroup']['memory.max'] = str(512 * 1024 * 1024)
        put(self.root / 'one' / 'observer.json', observer)
        self.assertFalse(MODULE.summarize(self.root)['valid'])
        observer['resources']['cgroup']['memory.max'] = str(4096 * 1024 * 1024)
        put(self.root / 'one' / 'observer.json', observer)
        manifest = json.loads((self.root / 'manifest.json').read_text())
        del manifest['case_policy']
        put(self.root / 'manifest.json', manifest)
        self.assertFalse(MODULE.summarize(self.root)['valid'])
        self.change('manifest.json', 'case_policy', 'qualification-v1')
        self.change('one/spec.json', 'expected_subtree_entries', 99)
        self.assertFalse(MODULE.summarize(self.root)['valid'])
        self.change('manifest.json', 'case_policy', 'unknown')
        self.assertFalse(MODULE.summarize(self.root)['valid'])

    def test_valid_pair_and_cli(self):
        result = MODULE.summarize(self.root)
        self.assertTrue(result['valid'], result['issues'])
        self.assertEqual(result['measurement_protocol'], MODULE.LEGACY_PROTOCOL)
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

    def test_copy_allocation_and_runner_cleanup(self):
        self.change('one/spec.json', 'initial_snapshot_allocated_bytes', 4096)
        self.change('two/spec.json', 'initial_snapshot_allocated_bytes', 8192)
        self.assertTrue(MODULE.summarize(self.root)['valid'])
        (self.root / 'runner-cleanup.json').unlink()
        self.assertFalse(MODULE.summarize(self.root)['valid'])
        self.assertTrue(MODULE.summarize(self.root, require_runner_cleanup=False)['valid'])
        put(self.root / 'runner-cleanup.json', {'success': False})
        self.assertFalse(MODULE.summarize(self.root)['valid'])

    def test_malformed_cgroup_and_duplicate_pair(self):
        self.change('one/metrics.json', 'cgroup_after', {'memory.events': 'oom 0'})
        self.assertFalse(MODULE.summarize(self.root)['valid'])
        fixture(self.root)
        self.change('two/spec.json', 'arm', 'baseline-v1')
        self.assertFalse(MODULE.summarize(self.root)['valid'])


if __name__ == '__main__':
    unittest.main()
