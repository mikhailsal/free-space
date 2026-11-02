import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import cause_host
import cause_case

QUIET = {'required_fields_available': True, 'background': {'required_fields_available': True, 'sources': {'memory': {'parsed': {'some': {'avg10': 0.0}}}}}, 'sources': {'meminfo': {'parsed': {'fields': {'MemAvailable': {'value': 6 * 2**20, 'unit': 'kB'}}}}}}


class HostTests(unittest.TestCase):
    def test_schedules(self):
        smoke = cause_host.build_schedule('smoke')
        self.assertEqual([x['id'] for x in smoke], ['smoke-idle', 'smoke-names', 'smoke-stat', 'smoke-scan', 'smoke-reserve', 'disk-pin'])
        screen = cause_host.build_schedule('screen')
        self.assertEqual(len(screen), 23)
        self.assertEqual([[x['mode'] for x in screen[n:n+4]] for n in (0, 4, 8, 12)],
                         [['idle', 'names', 'scan', 'stat'], ['names', 'stat', 'idle', 'scan'],
                          ['stat', 'scan', 'names', 'idle'], ['scan', 'idle', 'stat', 'names']])
        self.assertEqual([x['id'] for x in screen[16:22]], ['col1-full', 'col1-reserve', 'col2-reserve', 'col2-full', 'col3-full', 'col3-reserve'])
        self.assertEqual(sum(x['pin'] for x in screen), 1)
        with self.assertRaises(ValueError):
            cause_host.build_schedule('other')

    def test_guard_before_subprocess(self):
        with mock.patch.dict(os.environ, {}, clear=True), mock.patch('subprocess.Popen') as popen:
            with self.assertRaises(RuntimeError):
                cause_host.execute(None)
            popen.assert_not_called()

    def test_foreign_containers_are_not_ignored(self):
        with mock.patch.object(cause_case.subprocess, 'run', return_value=mock.Mock(stdout='own\ttoken\nforeign\tother\n')):
            capture = cause_case.foreign_containers('freespace.benchmark=token')
        self.assertFalse(capture['exclusive'])
        self.assertEqual(capture['foreign'], [{'id': 'foreign', 'benchmark_label': 'other'}])

    def test_quiet_host_gate(self):
        import copy
        cause_case.require_quiet_host(QUIET)
        busy = copy.deepcopy(QUIET)
        busy['background']['sources']['memory']['parsed']['some']['avg10'] = 1.01
        with self.assertRaises(ValueError):
            cause_case.require_quiet_host(busy)
        low = copy.deepcopy(QUIET)
        low['sources']['meminfo']['parsed']['fields']['MemAvailable']['value'] = 1
        with self.assertRaises(ValueError):
            cause_case.require_quiet_host(low)

    def test_output_must_be_empty(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'out'
            self.assertEqual(cause_host.empty_output(path), path)
            (path / 'foreign').write_text('x')
            with self.assertRaises(ValueError):
                cause_host.empty_output(path)
            self.assertEqual((path / 'foreign').read_text(), 'x')

    def test_binary_mismatch(self):
        with mock.patch.object(cause_host, 'provenance', return_value={'sha256': 'wrong', 'source_revision': cause_host.REVISION, 'build': {}}):
            with self.assertRaises(ValueError):
                cause_host.verify_binary(Path('binary'))

    def test_identity_must_match_nonce(self):
        spec = dict(protocol='resource-cause-v1', id='case', nonce='123')
        cause_case.identity(dict(spec), spec)
        with self.assertRaises(ValueError):
            cause_case.identity(dict(spec, nonce='different'), spec)

    def test_failure_still_acknowledges_and_removes_worker(self):
        import json
        with tempfile.TemporaryDirectory() as root:
            out = Path(root)
            directory = out / 'case'
            directory.mkdir()
            spec = dict(protocol='resource-cause-v1', id='case', nonce='n' * 32,
                        timeout_seconds=1, io_device='/dev/loop1', io_limits=dict(rbps=1, wbps=2, riops=3, wiops=4))
            for name, value in (('worker-ready.json', dict(spec)),
                                ('metrics.json', dict(spec, validated=False)),
                                ('worker-terminal.json', dict(spec, validated=False, acknowledged=True))):
                (directory / name).write_text(json.dumps(value))
            class Helper:
                def dump(self, path, value):
                    Path(path).write_text(json.dumps(value))
            process = mock.Mock()
            process.poll.return_value = 0
            with mock.patch.object(cause_case, 'foreign_containers', return_value={'exclusive': True, 'running_count': 0}), \
                 mock.patch.object(cause_case, 'capture_global', return_value=QUIET), \
                 mock.patch.object(cause_case.subprocess, 'Popen', return_value=process), \
                 mock.patch.object(cause_case.subprocess, 'run') as run, \
                 mock.patch.object(cause_case, 'inspect', return_value={'State': {'Running': False, 'ExitCode': 1, 'OOMKilled': False}}):
                result = cause_case.run_case('image', Path('/binary'), out, out, out, spec, 'label', Helper())
            self.assertFalse(result['validated'])
            self.assertTrue(result['container_cleanup'])
            self.assertEqual(json.loads((directory / 'host-ack.json').read_text()),
                             {key: spec[key] for key in ('protocol', 'id', 'nonce')})
            self.assertEqual(run.call_args.args[0][:3], ['docker', 'rm', '-f'])

    def test_ready_capture_precedes_acknowledgement(self):
        import json
        with tempfile.TemporaryDirectory() as root:
            out = Path(root)
            directory = out / 'case'
            directory.mkdir()
            spec = dict(protocol='resource-cause-v1', id='case', nonce='n' * 32,
                        timeout_seconds=1, io_device='/dev/loop1', io_limits=dict(rbps=1, wbps=2, riops=3, wiops=4))
            for name, value in (('worker-ready.json', dict(spec)),
                                ('metrics.json', dict(spec, validated=True)),
                                ('worker-terminal.json', dict(spec, validated=True, acknowledged=True))):
                (directory / name).write_text(json.dumps(value))
            events = []
            class Helper:
                def dump(self, path, value):
                    Path(path).write_text(json.dumps(value))
            def capture_global(path, **kwargs):
                events.append(Path(path).name)
                return QUIET
            def cgroup(pid):
                events.append('cgroup')
                return {'pid': pid, 'process_identity_stable': True, 'sources': {'io.max': {'available': True}}}
            def publish(helper, path, value):
                events.append('ack')
                helper.dump(path, value)
            process = mock.Mock()
            process.poll.return_value = 0
            live = {'State': {'Pid': 17, 'Running': True}, 'Mounts': []}
            terminal = {'State': {'Running': False, 'ExitCode': 0, 'OOMKilled': False}}
            with mock.patch.object(cause_case, 'foreign_containers', side_effect=[
                    {'exclusive': True, 'running_count': 0}, {'exclusive': True, 'running_count': 1}]), \
                 mock.patch.object(cause_case, 'capture_global', side_effect=capture_global), \
                 mock.patch.object(cause_case, 'capture_cgroup', side_effect=cgroup), \
                 mock.patch.object(cause_case, 'publish', side_effect=publish), \
                 mock.patch.object(cause_case.subprocess, 'Popen', return_value=process), \
                 mock.patch.object(cause_case.subprocess, 'run'), \
                 mock.patch.object(cause_case, 'inspect', side_effect=[live, terminal]):
                result = cause_case.run_case('image', Path('/binary'), out, out, out, spec, 'label', Helper())
            self.assertTrue(result['validated'], result['errors'])
            self.assertEqual(events, ['global-before', 'cgroup', 'global-after', 'ack'])
            self.assertEqual(result['docker_inspect_before_ack'], live)
            self.assertEqual(result['cgroup_after']['pid'], 17)

    def test_probe_integrity_receipt_for_match_and_mismatch(self):
        import json
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'integrity.json'
            class Helper:
                def dump(self, destination, value):
                    Path(destination).write_text(json.dumps(value))
            with mock.patch.object(cause_host, 'seed_hash', return_value='abc'):
                cause_host.verify_seed_integrity(Path(root), 'abc', path, Helper())
                self.assertEqual(json.loads(path.read_text()), {
                    'expected_seed_sha256': 'abc', 'observed_seed_sha256': 'abc', 'unchanged': True})
                with self.assertRaises(ValueError):
                    cause_host.verify_seed_integrity(Path(root), 'different', path, Helper())
                self.assertEqual(json.loads(path.read_text()), {
                    'expected_seed_sha256': 'different', 'observed_seed_sha256': 'abc', 'unchanged': False})

    def test_command_has_isolated_worker(self):
        spec = dict(id='a', io_device='/dev/loop1', io_limits=dict(rbps=1, wbps=2, riops=3, wiops=4))
        command = cause_case.command('image', '/binary', '/case', '/tree', '/out', spec, 'freespace.benchmark=token', 'name')
        self.assertIn('GITHUB_ACTIONS=true', command)
        self.assertIn('type=bind,src=/tree,dst=/scratch/tree,readonly', command)
        self.assertEqual(command[-2:], ['/harness/cause_worker.py', '/results/a.json'])
        self.assertNotIn('--privileged', command)


if __name__ == '__main__':
    unittest.main()
