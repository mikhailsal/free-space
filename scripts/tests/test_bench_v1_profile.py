import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bench_v1_profile import analyze_events


def trace(removed=False):
    phases = ['refresh_start', 'refresh_loaded', 'refresh_before_graft']
    if not removed:
        phases += ['refresh_after_names', 'refresh_after_nodes']
    phases += ['refresh_after_graft', 'refresh_after_finalize', 'index']
    events = []
    for i, phase in enumerate(phases):
        event = dict(phase=phase, elapsed_seconds=float(i), rss_kib=10 + i, hwm_kib=20 + i)
        if phase not in ('refresh_start', 'index'):
            appended = not removed and phase in ('refresh_after_nodes', 'refresh_after_graft', 'refresh_after_finalize')
            event.update(node_len=102 if appended else 100, node_capacity=200 if appended else 100,
                         name_len=20, name_capacity=40, node_size_bytes=88,
                         node_buffer_changed=phase == 'refresh_after_nodes', name_buffer_changed=False)
        events.append(event)
    return events


class ProfileTests(unittest.TestCase):
    def check(self, events, removed=False):
        return analyze_events(events, 100, 0 if removed else 3, removed)

    def test_valid_and_capacity_change(self):
        result = self.check(trace())
        self.assertTrue(result['valid_structure'])
        self.assertEqual(result['node_capacity_changes'][0]['after'], 200)

    def test_missing_and_unordered_phases(self):
        events = trace()
        del events[3]
        self.assertFalse(self.check(events)['valid_structure'])
        events = trace()
        events[3], events[4] = events[4], events[3]
        self.assertFalse(self.check(events)['valid_structure'])

    def test_types_and_first_flags(self):
        for key, value in [('node_len', True), ('node_capacity', -1), ('node_size_bytes', 0),
                           ('node_buffer_changed', 0), ('rss_kib', True)]:
            with self.subTest(key=key):
                events = trace()
                events[1][key] = value
                self.assertFalse(self.check(events)['valid_structure'])
        events = trace()
        events[1]['node_buffer_changed'] = True
        self.assertFalse(self.check(events)['valid_structure'])

    def test_hwm_drop_preserved_but_elapsed_drop_invalid(self):
        events = trace()
        events[-1]['hwm_kib'] = 1
        original = copy.deepcopy(events)
        result = self.check(events)
        self.assertTrue(result['valid_structure'])
        self.assertFalse(result['original_hwm_monotonic_contract_pass'])
        self.assertEqual(result['hwm_anomalies'][0]['hwm_kib'], 1)
        self.assertEqual(result['events'], original)
        events[-1]['elapsed_seconds'] = 0
        self.assertFalse(self.check(events)['valid_structure'])

    def test_wrong_length_and_capacity(self):
        events = trace()
        events[4]['node_len'] = 101
        self.assertFalse(self.check(events)['valid_structure'])
        events = trace()
        events[4]['node_capacity'] = 101
        self.assertFalse(self.check(events)['valid_structure'])

    def test_huge_elapsed_is_invalid_and_serializable(self):
        import json
        events = trace()
        events[1]['elapsed_seconds'] = 10 ** 399
        result = self.check(events)
        self.assertFalse(result['valid_structure'])
        self.assertEqual(json.loads(json.dumps(result, allow_nan=False))['events'][1]['elapsed_seconds'], 10 ** 399)

    def test_removed_outcome(self):
        self.assertTrue(self.check(trace(True), True)['valid_structure'])
        self.assertFalse(self.check(trace(), True)['valid_structure'])
        self.assertFalse(self.check(trace(True), False)['valid_structure'])


class FolderTests(unittest.TestCase):
    def setUp(self):
        import tempfile
        from unittest.mock import patch
        import json
        import bench_v1_profile as profile
        self.profile, self.json = profile, json
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.arm = self.root / 'p1-baseline-v1'
        self.arm.mkdir()
        self.write('manifest.json', dict(operation='refresh', expected_arms=[self.arm.name],
                                         diagnostic_policy=dict(allocation_diagnostics=True, profile_policy='profile-v2')))
        self.write('p1-baseline-v1/spec.json', dict(allocation_diagnostics=True, expected_entries=100,
                                                   expected_subtree_entries=3))
        self.write('p1-baseline-v1/metrics.json', dict(report=dict(subtree_entries=3, removed=False),
                                                       time=dict(peak_rss_kib=100)))
        self.trace = self.arm / 'allocation-profile.jsonl'
        self.trace.write_text(''.join(json.dumps(e) + '\n' for e in trace()))
        mock = patch.object(profile, 'summarize', return_value={'valid': True})
        self.summary = mock.start()
        self.addCleanup(mock.stop)

    def write(self, name, data):
        (self.root / name).write_text(self.json.dumps(data))

    def manifest(self):
        return self.json.loads((self.root / 'manifest.json').read_text())

    def test_gate_and_prospective_policy(self):
        self.assertTrue(self.profile.analyze(self.root)['valid_structure'])
        self.summary.return_value = {'valid': False}
        self.assertFalse(self.profile.analyze(self.root)['valid_structure'])
        self.summary.return_value = {'valid': True}
        m = self.manifest()
        del m['diagnostic_policy']['profile_policy']
        self.write('manifest.json', m)
        self.assertFalse(self.profile.analyze(self.root)['valid_structure'])

    def test_operation_diagnostic_and_ids(self):
        for key, value in [('operation', 'scan'), ('expected_arms', ['../outside']),
                           ('expected_arms', ['same', 'same'])]:
            m = self.manifest()
            old = m[key]
            m[key] = value
            self.write('manifest.json', m)
            self.assertFalse(self.profile.analyze(self.root)['valid_structure'])
            m[key] = old
            self.write('manifest.json', m)
        self.write('p1-baseline-v1/spec.json', dict(allocation_diagnostics=False, expected_entries=100,
                                                   expected_subtree_entries=3))
        self.assertFalse(self.profile.analyze(self.root)['valid_structure'])

    def test_trace_safety_and_json(self):
        import os
        for content in ('{"x":1e999}', '{"x":1,"x":2}', 'x' * (128 * 1024 + 1), '[' * 2000):
            with self.subTest(content=content[:20]):
                self.trace.write_text(content)
                self.assertFalse(self.profile.analyze(self.root)['valid_structure'])
        self.trace.unlink()
        self.assertFalse(self.profile.analyze(self.root)['valid_structure'])
        os.mkfifo(self.trace)
        self.assertFalse(self.profile.analyze(self.root)['valid_structure'])
        self.trace.unlink()
        self.trace.symlink_to(self.root / 'manifest.json')
        self.assertFalse(self.profile.analyze(self.root)['valid_structure'])

    def test_hwm_and_output_protection(self):
        events = trace()
        events[-1]['hwm_kib'] = 1
        self.trace.write_text(''.join(self.json.dumps(e) + '\n' for e in events))
        result = self.profile.analyze(self.root)
        self.assertTrue(result['valid_structure'])
        self.assertFalse(result['original_hwm_monotonic_contract_pass'])
        output = self.root / 'old.json'
        output.write_text('old report')
        with self.assertRaises(FileExistsError):
            self.profile.main([str(self.root), '--json-out', str(output)])
        self.assertEqual(output.read_text(), 'old report')

    def test_report_subtree_count_and_directory_link(self):
        self.write('p1-baseline-v1/metrics.json', dict(report=dict(subtree_entries=2, removed=False), time={}))
        self.assertFalse(self.profile.analyze(self.root)['valid_structure'])
        alias = self.root / 'alias'
        alias.symlink_to(self.arm, target_is_directory=True)
        m = self.manifest()
        m['expected_arms'] = ['alias']
        self.write('manifest.json', m)
        self.assertFalse(self.profile.analyze(self.root)['valid_structure'])

    def test_nonfinite_peak_writes_invalid_report(self):
        self.summary.return_value = {'valid': False}
        for value in (float('nan'), float('inf'), float('-inf')):
            with self.subTest(value=value):
                self.write('p1-baseline-v1/metrics.json',
                           dict(report=dict(subtree_entries=3, removed=False), time=dict(peak_rss_kib=value)))
                output = self.root / ('invalid-' + str(value) + '.json')
                self.assertEqual(self.profile.main([str(self.root), '--json-out', str(output)]), 1)
                result = self.json.loads(output.read_text())
                self.assertFalse(result['valid_structure'])
                self.assertIsNone(result['arms'][self.arm.name]['time_peak_rss_kib'])

    def test_huge_elapsed_writes_invalid_report(self):
        self.summary.return_value = {'valid': False}
        events = trace()
        events[1]['elapsed_seconds'] = 10 ** 399
        self.trace.write_text(''.join(self.json.dumps(e) + '\n' for e in events))
        output = self.root / 'invalid-huge.json'
        self.assertEqual(self.profile.main([str(self.root), '--json-out', str(output)]), 1)
        result = self.json.loads(output.read_text())
        self.assertFalse(result['valid_structure'])
        self.assertEqual(result['arms'][self.arm.name]['events'][1]['elapsed_seconds'], 10 ** 399)


if __name__ == '__main__':
    unittest.main()
