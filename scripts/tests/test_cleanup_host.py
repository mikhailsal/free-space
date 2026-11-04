"""The docker commands the hosted harness builds can only run on GitHub, so their shape is pinned here."""
import importlib.util
import json
import os
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).parents[1]
sys.path.insert(0, str(SCRIPTS))
import cleanup_probe_plan as plan
import cleanup_probe_fixture as fx

spec = importlib.util.spec_from_file_location('cleanup_probe_host', SCRIPTS / 'cleanup_probe_host.py')
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


def receipt(profile, count):
    return dict(scale=count, controls={'group': {'files': fx.GROUP_FILES}}, **fx.receipt_shape(profile, count))


class OracleCommandTest(unittest.TestCase):
    def run_oracle(self, scenario):
        with tempfile.TemporaryDirectory() as tmp:
            out, root = Path(tmp) / 'out', Path(tmp)
            out.mkdir()
            cases = {'v1': root / 'case-v1', 'probe': root / 'case-probe'}
            seed = root / 'seed-files-probe'
            commands = []

            def fake_run(cmd, **kwargs):
                commands.append(cmd)
                if cmd[:2] == ['docker', 'run']:
                    kwargs['stdout'].write(json.dumps({'success': True}))
                    kwargs['stdout'].flush()
                return types.SimpleNamespace(returncode=0)

            ident = f'files-{scenario}-1-oracle'
            expectation = plan.refresh_expectation(scenario, receipt('files', 1_000_000))
            with mock.patch.object(host.subprocess, 'run', fake_run), \
                    mock.patch.object(host, 'capture_resources', lambda: {'success': True}), \
                    mock.patch.object(host, 'require_resources', lambda resources: None), \
                    mock.patch.dict(os.environ, {'FREESPACE_BENCHMARK_RUN_TOKEN': 'token'}):
                host.oracle(types.SimpleNamespace(image='image:tag'), cases, seed, expectation, out, ident, 60)
            return commands, cases, seed, expectation

    def test_the_oracle_container_gets_the_untouched_seed_and_the_planned_scope(self):
        commands, cases, seed, expectation = self.run_oracle('wide')
        run = next(c for c in commands if c[:2] == ['docker', 'run'])
        mounts = [a for a in run if a.startswith('type=bind')]
        self.assertIn(f'type=bind,src={cases["v1"] / "snapshot"},dst=/baseline,readonly', mounts)
        self.assertIn(f'type=bind,src={cases["probe"] / "snapshot"},dst=/coarse,readonly', mounts)
        self.assertIn(f'type=bind,src={seed / "snapshot"},dst=/seed,readonly', mounts)
        self.assertTrue(all(m.endswith(',readonly') for m in mounts), 'the oracle must not be able to change what it judges')
        tail = run[run.index('/harness/cleanup_oracle.py'):]
        self.assertEqual(tail, ['/harness/cleanup_oracle.py', '/baseline/snapshot.db', '/coarse/coarse.db', '--fixture', '/fixture.json',
                                '--seed', '/seed/coarse.db', *expectation])
        self.assertEqual(expectation[-2:], ['945', '956'])
        self.assertIn(['docker', 'rm', '-f', run[run.index('--name') + 1]], commands)

    def test_a_scan_is_expected_to_rewrite_everything(self):
        commands, _, _, expectation = self.run_oracle('full')
        run = next(c for c in commands if c[:2] == ['docker', 'run'])
        self.assertEqual(run[run.index('--seed'):], ['--seed', '/seed/coarse.db', '--expect-scope', 'full', '--expect-boundary', '/scratch/tree'])
        self.assertEqual(expectation, ['--expect-scope', 'full', '--expect-boundary', '/scratch/tree'])


class RunContainerTest(unittest.TestCase):
    """A writer container that dies early must report why, and a case id can never be measured twice."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.out, self.cwd = root / 'out', os.getcwd()
        for name in ('out', 'binaries', 'scripts', 'case', 'tree'):
            (root / name).mkdir()
        (root / 'binaries' / 'freespace').write_text('binary')
        os.chdir(root)
        self.addCleanup(os.chdir, self.cwd)
        env = {'FREESPACE_BENCHMARK_RUN_TOKEN': 'token', 'CLEANUP_OUTER_PATH': str(root)}
        patcher = mock.patch.dict(os.environ, env)
        patcher.start()
        self.addCleanup(patcher.stop)

    def run_writer(self, returncode, log, worker_output=None, ident='files-full-1-v1'):
        def fake_run(cmd, **kwargs):
            if cmd[:2] == ['docker', 'run']:
                kwargs['stdout'].write(log)
                if worker_output is not None:
                    (self.out / ident).mkdir(exist_ok=True)
                    (self.out / ident / 'metrics.json').write_text(json.dumps(worker_output))
                return types.SimpleNamespace(returncode=returncode)
            return types.SimpleNamespace(returncode=0)

        inspect = json.dumps([{'State': {'OOMKilled': False, 'ExitCode': returncode}}])
        args = types.SimpleNamespace(io_device='/dev/null', image='image:tag')
        with mock.patch.object(host.subprocess, 'run', fake_run), mock.patch.object(host.subprocess, 'check_output', lambda *a, **k: inspect):
            return host.run_container(args, 'v1', 'scan', '/scratch/tree', Path('tree'), Path('case'), self.out, ident, 60, {'entries': 1})

    def test_an_early_worker_failure_reports_the_container_log(self):
        with self.assertRaises(RuntimeError) as caught:
            self.run_writer(125, 'Traceback ...\nValueError: unexpected memory/swap limit\n')
        self.assertIn('writer failed files-full-1-v1', str(caught.exception))
        self.assertIn('unexpected memory/swap limit', str(caught.exception))
        self.assertTrue((self.out / 'files-full-1-v1' / 'docker-state.json').is_file())

    def test_a_case_id_is_never_measured_twice(self):
        (self.out / 'files-full-1-v1').mkdir()
        with self.assertRaises(FileExistsError):
            self.run_writer(0, '', {'validated': True})

    def test_a_successful_writer_returns_its_metrics(self):
        record = self.run_writer(0, '', {'validated': True, 'id': 'files-full-1-v1'})
        self.assertEqual(record['id'], 'files-full-1-v1')
        state = json.loads((self.out / 'files-full-1-v1' / 'docker-state.json').read_text())
        self.assertEqual(state['ExitCode'], 0)


if __name__ == '__main__':
    unittest.main()
