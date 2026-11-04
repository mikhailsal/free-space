"""The docker commands the hosted harness builds can only run on GitHub, so their shape is pinned here."""
import contextlib
import importlib.util
import io
import json
import os
import subprocess
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


def removal(rm):
    """What `docker rm -f` does in a test: succeed (None), report a failure, or hang past its timeout."""
    if isinstance(rm, BaseException):
        raise rm
    return rm or types.SimpleNamespace(returncode=0, stderr='')


class OracleCommandTest(unittest.TestCase):
    def run_oracle(self, scenario, returncode=0, stdout=None, stderr='', rm=None):
        with tempfile.TemporaryDirectory() as tmp:
            out, root = Path(tmp) / 'out', Path(tmp)
            out.mkdir()
            cases = {'v1': root / 'case-v1', 'probe': root / 'case-probe'}
            seed = root / 'seed-files-probe'
            commands = []

            def fake_run(cmd, **kwargs):
                commands.append(cmd)
                if cmd[:2] == ['docker', 'run']:
                    kwargs['stdout'].write(json.dumps({'success': True}) if stdout is None else stdout)
                    kwargs['stdout'].flush()
                    kwargs['stderr'].write(stderr)
                    kwargs['stderr'].flush()
                    return types.SimpleNamespace(returncode=returncode)
                return removal(rm)

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

    def test_a_rejection_reports_what_the_oracle_found(self):
        verdict = json.dumps({'success': False, 'error': 'the refresh window is not a non-empty bucket of the boundary in the seed'})
        with self.assertRaisesRegex(ValueError, 'oracle rejected files-wide-1-oracle: the refresh window is not a non-empty bucket'):
            self.run_oracle('wide', returncode=1, stdout=verdict)

    def test_a_docker_failure_without_a_verdict_reports_its_stderr(self):
        with self.assertRaisesRegex(ValueError, 'oracle rejected files-wide-1-oracle: docker: invalid mount config'):
            self.run_oracle('wide', returncode=125, stdout='', stderr='docker: invalid mount config\n')
        with self.assertRaisesRegex(ValueError, 'oracle rejected files-wide-1-oracle: no verdict'):
            self.run_oracle('wide', returncode=1, stdout='', stderr='')

    def test_only_a_successful_verdict_is_accepted(self):
        for text in ('{"success": false, "error": "x"}', '{"success": "yes"}', '{}', '[]', '"yes"', 'not json'):
            with self.assertRaises(ValueError, msg=text):
                self.run_oracle('wide', stdout=text)  # even with a clean exit status
        with self.assertRaises(ValueError):
            self.run_oracle('wide', returncode=1, stdout=json.dumps({'success': True}))

    def test_a_failing_removal_never_hides_the_oracle_rejection(self):
        for rm in (types.SimpleNamespace(returncode=1, stderr='Error: busy\n'), subprocess.TimeoutExpired(['docker'], 30)):
            with contextlib.redirect_stderr(io.StringIO()) as warned, self.assertRaisesRegex(ValueError, 'oracle rejected files-wide-1-oracle: boom'):
                self.run_oracle('wide', returncode=1, stdout=json.dumps({'success': False, 'error': 'boom'}), rm=rm)
            self.assertIn('docker rm -f', warned.getvalue())

    def test_a_failing_removal_after_an_accepted_verdict_is_only_a_warning(self):
        with contextlib.redirect_stderr(io.StringIO()) as warned:
            self.run_oracle('wide', rm=types.SimpleNamespace(returncode=1, stderr='Error: busy\n'))
        self.assertIn('busy', warned.getvalue())
        with contextlib.redirect_stderr(io.StringIO()) as warned:
            self.run_oracle('wide', rm=subprocess.TimeoutExpired(['docker'], 30))
        self.assertIn('timed out', warned.getvalue())


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

    def run_writer(self, returncode, log, worker_output=None, ident='files-full-1-v1', container='exited', rm=None):
        """`container` is what `docker inspect` finds: 'exited', 'oom' (killed for memory) or None (docker run died before creating it)."""
        self.commands = []

        def fake_run(cmd, **kwargs):
            self.commands.append(cmd)
            if cmd[:2] == ['docker', 'run']:
                kwargs['stdout'].write(log)
                if worker_output is not None:
                    (self.out / ident).mkdir(exist_ok=True)
                    (self.out / ident / 'metrics.json').write_text(json.dumps(worker_output))
                return types.SimpleNamespace(returncode=returncode)
            if cmd[:2] == ['docker', 'inspect']:
                if container is None:
                    return types.SimpleNamespace(returncode=1, stdout='')
                return types.SimpleNamespace(returncode=0, stdout=json.dumps([{'State': {'OOMKilled': container == 'oom', 'ExitCode': returncode}}]))
            return removal(rm)

        args = types.SimpleNamespace(io_device='/dev/null', image='image:tag')
        with mock.patch.object(host.subprocess, 'run', fake_run):
            return host.run_container(args, 'v1', 'scan', '/scratch/tree', Path('tree'), Path('case'), self.out, ident, 60, {'entries': 1})

    def test_an_early_worker_failure_reports_the_container_log(self):
        with self.assertRaises(RuntimeError) as caught:
            self.run_writer(125, 'Traceback ...\nValueError: unexpected memory/swap limit\n')
        self.assertIn('writer failed files-full-1-v1 (exit status 125)', str(caught.exception))
        self.assertIn('unexpected memory/swap limit', str(caught.exception))
        self.assertTrue((self.out / 'files-full-1-v1' / 'docker-state.json').is_file())

    def test_a_docker_run_that_died_before_creating_the_container_reports_the_log(self):
        with self.assertRaises(RuntimeError) as caught:
            self.run_writer(125, 'docker: Error response from daemon: invalid mount config for type "bind"\n', container=None)
        self.assertIn('writer failed files-full-1-v1 (no such container)', str(caught.exception))
        self.assertIn('invalid mount config', str(caught.exception))
        self.assertFalse((self.out / 'files-full-1-v1' / 'docker-state.json').exists())
        run = next(c for c in self.commands if c[:2] == ['docker', 'run'])
        self.assertIn(['docker', 'rm', '-f', run[run.index('--name') + 1]], self.commands, 'a half-created container must still be removed')

    def test_a_vanished_container_is_a_failure_even_after_a_clean_exit(self):
        with self.assertRaisesRegex(RuntimeError, r'writer failed files-full-1-v1 \(no such container\)'):
            self.run_writer(0, '', {'validated': True}, container=None)

    def test_an_out_of_memory_kill_fails_the_run_whatever_the_exit_status(self):
        with self.assertRaisesRegex(RuntimeError, r'writer failed files-full-1-v1 \(out of memory\)'):
            self.run_writer(0, '', {'validated': True}, container='oom')

    def test_a_failing_removal_never_hides_the_writer_failure(self):
        removals = (types.SimpleNamespace(returncode=1, stderr='Error: busy\n'), subprocess.TimeoutExpired(['docker'], 30))
        for pair, rm in enumerate(removals, 1):  # each case id may be measured once, so each removal failure gets its own
            ident = f'files-full-{pair}-v1'
            with contextlib.redirect_stderr(io.StringIO()) as warned, self.assertRaisesRegex(RuntimeError, rf'writer failed {ident} \(exit status 125\)'):
                self.run_writer(125, 'boom', ident=ident, rm=rm)
            self.assertIn('docker rm -f', warned.getvalue())

    def test_a_failing_removal_after_a_good_run_still_returns_the_metrics(self):
        with contextlib.redirect_stderr(io.StringIO()) as warned:
            record = self.run_writer(0, '', {'validated': True}, rm=types.SimpleNamespace(returncode=1, stderr='Error: busy\n'))
        self.assertTrue(record['validated'])
        self.assertIn('busy', warned.getvalue())

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
