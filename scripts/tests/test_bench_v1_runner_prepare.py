"""Mocked preparation tests: never inspect or delete host SDKs."""
import importlib.util
import io
import itertools
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import textwrap
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
SPEC = importlib.util.spec_from_file_location('runner_prepare', Path(__file__).resolve().parents[1] / 'bench_v1_runner_prepare.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
ENV = {'GITHUB_ACTIONS': 'true', 'RUNNER_ENVIRONMENT': 'github-hosted', 'RUNNER_TEMP': '/test/scratch'}


def space(n):
    return dict(total_bytes=100 * MODULE.FLOOR, used_bytes=0, available_bytes=n)


def invoke(**kwargs):
    defaults = dict(get_target=lambda _: ('/test/scratch', 7), read_disk=lambda _: space(1),
                    read_mounts=lambda: [], safe=lambda *_: True, run=Mock())
    defaults.update(kwargs)
    return MODULE.prepare(ENV, **defaults)


class RunnerPreparationTests(unittest.TestCase):
    def test_workflow_apt_sources_guard_rewrite_and_symlink(self):
        workflow = (Path(__file__).resolve().parents[2] / '.github/workflows/representative-bench.yml').read_text()
        step = workflow.split('      - name: Install musl tools\n', 1)[1].split('      - uses:', 1)[0]
        code = textwrap.dedent(step.split("python3 - <<'PY'\n", 1)[1].split('          PY\n', 1)[0])
        self.assertEqual(step.count('timeout --kill-after=5s 180s apt-get'), 2)
        self.assertIn('sudo --preserve-env=GITHUB_ACTIONS,RUNNER_ENVIRONMENT python3', step)
        self.assertEqual(step.count('-o Acquire::Retries=1 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20'), 2)
        self.assertIn('--error-on=any update -q', step)
        with patch.dict(os.environ, {}, clear=True), patch('os.open', side_effect=AssertionError('filesystem touched')):
            with self.assertRaisesRegex(RuntimeError, 'GitHub-hosted'):
                exec(code, {})
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'sources.list'
            deb822 = root / 'ubuntu.sources'
            mapped = code.replace('/etc/apt/sources.list.d/ubuntu.sources', str(deb822)).replace('/etc/apt/sources.list', str(source))
            source.write_text('deb http://azure.archive.ubuntu.com/ubuntu noble main\n'
                              'deb http://security.ubuntu.com/ubuntu noble-security main\n'
                              'deb http://thirdparty.example/ubuntu noble main\n'
                              'deb http://archive.ubuntu.com/custom noble main\n'
                              'deb http://archive.ubuntu.com/ubuntu-ports noble main\n'
                              'deb http://archive.ubuntu.com.thirdparty.example/ubuntu noble main\n')
            deb822.write_text('Types: deb\nURIs: mirror+file:/etc/apt/apt-mirrors.txt http://security.ubuntu.com/ubuntu/\nSuites: noble-updates\nSigned-By: /key\n')
            with patch.dict(os.environ, ENV, clear=True):
                exec(mapped, {})
            self.assertEqual(source.read_text(), 'deb https://archive.ubuntu.com/ubuntu noble main\n'
                             'deb https://security.ubuntu.com/ubuntu noble-security main\n'
                             'deb http://thirdparty.example/ubuntu noble main\n'
                             'deb http://archive.ubuntu.com/custom noble main\n'
                             'deb http://archive.ubuntu.com/ubuntu-ports noble main\n'
                             'deb http://archive.ubuntu.com.thirdparty.example/ubuntu noble main\n')
            self.assertEqual(deb822.read_text(), 'Types: deb\nURIs: https://archive.ubuntu.com/ubuntu/ https://security.ubuntu.com/ubuntu/\nSuites: noble-updates\nSigned-By: /key\n')
            before_times = (source.stat().st_mtime_ns, deb822.stat().st_mtime_ns)
            with patch.dict(os.environ, ENV, clear=True):
                exec(mapped, {})
            self.assertEqual(before_times, (source.stat().st_mtime_ns, deb822.stat().st_mtime_ns))
            original_source, original_deb822 = source.read_bytes(), deb822.read_bytes()
            for path, extra in ((source, 'deb mirror+file:/etc/apt/unknown.txt noble main\n'),
                                (deb822, 'URIs: mirror+https://thirdparty.example/list\n')):
                path.write_text(path.read_text() + extra)
                before_failure = (source.read_bytes(), deb822.read_bytes())
                with patch.dict(os.environ, ENV, clear=True), self.assertRaisesRegex(ValueError, 'unsupported active Ubuntu mirror'):
                    exec(mapped, {})
                self.assertEqual(before_failure, (source.read_bytes(), deb822.read_bytes()))
                source.write_bytes(original_source)
                deb822.write_bytes(original_deb822)
            deb822.unlink()
            deb822.symlink_to(source)
            before = source.read_bytes()
            with patch.dict(os.environ, ENV, clear=True), self.assertRaises(OSError):
                exec(mapped, {})
            self.assertEqual(source.read_bytes(), before)
            deb822.unlink()
            os.link(source, deb822)
            with patch.dict(os.environ, ENV, clear=True), self.assertRaisesRegex(ValueError, 'uniquely linked'):
                exec(mapped, {})
            deb822.unlink()
            source.write_text('deb http://thirdparty.example/ubuntu noble main\n')
            with patch.dict(os.environ, ENV, clear=True), self.assertRaisesRegex(ValueError, 'no supported Ubuntu'):
                exec(mapped, {})
            self.assertEqual(source.read_text(), 'deb http://thirdparty.example/ubuntu noble main\n')


    def test_sufficient_space_never_checks_mounts_or_deletes(self):
        mounts, safe, run = Mock(), Mock(), Mock()
        result = invoke(read_disk=lambda _: space(MODULE.TARGET), read_mounts=mounts, safe=safe, run=run)
        self.assertTrue(result['success']); self.assertEqual(result['actions'], [])
        mounts.assert_not_called(); safe.assert_not_called(); run.assert_not_called()

    def test_guard_precedes_all_filesystem_or_delete(self):
        for env in ({}, {'GITHUB_ACTIONS': 'true'}, {'GITHUB_ACTIONS': 'true', 'RUNNER_ENVIRONMENT': 'self-hosted'}):
            target, read, mounts, safe, run = Mock(), Mock(), Mock(), Mock(), Mock()
            result = MODULE.prepare(env, get_target=target, read_disk=read, read_mounts=mounts, safe=safe, run=run)
            self.assertFalse(result['success'])
            for mock in (target, read, mounts, safe, run): mock.assert_not_called()

    def test_fixed_allowlist_checkpoint_recheck_and_stop(self):
        values = iter([space(1), space(2), space(MODULE.TARGET), space(MODULE.TARGET)])
        snapshots = []
        def update(receipt): snapshots.append(json.loads(json.dumps(receipt)))
        def run(*args, **kwargs):
            if args[0][0] == 'timeout': return
            self.assertEqual(snapshots[-1]['actions'][-1]['status'], 'started')
            self.assertEqual(args[0][0:4], ['sudo', '-n', 'timeout', '--kill-after=2s'])
            self.assertIn(args[0][-1], MODULE.ALLOWLIST)
            self.assertLessEqual(kwargs['timeout'], MODULE.COMMAND_SECONDS + 5)
        result = invoke(read_disk=lambda _: next(values), run=run, update=update)
        self.assertTrue(result['success'])
        self.assertEqual([a['path'] for a in result['actions']], list(MODULE.ALLOWLIST[:2]))
        self.assertEqual(result['actions'][0]['freed_bytes'], 1)
        self.assertEqual((result['target_path'], result['target_dev']), ('/test/scratch', 7))

    def test_reaching_target_after_synced_action_does_not_sync_twice(self):
        spaces = iter([space(1), space(MODULE.TARGET)])
        run = Mock()
        result = invoke(read_disk=lambda _: next(spaces), run=run)
        self.assertTrue(result['success'])
        self.assertEqual([c.args[0][0] for c in run.call_args_list], ['sudo', 'timeout'])

    def test_insufficient_space_after_removals_reports_capacity_cause(self):
        run = Mock()
        result = invoke(read_disk=lambda _: space(MODULE.FLOOR), run=run)
        self.assertFalse(result['success'])
        self.assertIn('insufficient free space after permitted preparation', result['errors'])
        self.assertEqual(len(run.call_args_list), 2 * len(MODULE.ALLOWLIST))

    def test_skips_unsafe_and_fails_when_allowlist_insufficient(self):
        run = Mock()
        result = invoke(safe=lambda *_: False, run=run)
        self.assertFalse(result['success']); self.assertEqual(len(result['actions']), len(MODULE.ALLOWLIST))
        self.assertTrue(all(a['status'] == 'skipped-unsafe-or-absent' for a in result['actions']))
        run.assert_not_called()

    def test_errors_fail_closed_and_never_use_real_deletion(self):
        for error in (subprocess.TimeoutExpired('rm', 1), subprocess.CalledProcessError(124, 'rm'), OSError('failed')):
            with self.subTest(error=error):
                result = invoke(run=Mock(side_effect=error))
                self.assertFalse(result['success']); self.assertEqual(result['actions'][0]['status'], 'error')
                self.assertTrue(result['errors'])
        result = invoke(read_disk=Mock(side_effect=OSError('statvfs failed')), safe=Mock(), run=Mock())
        self.assertFalse(result['success'])
        self.assertFalse(invoke(read_mounts=Mock(side_effect=OSError('mountinfo failed')), safe=Mock(), run=Mock())['success'])

    def test_permission_error_and_stderr_are_recorded(self):
        errors = (PermissionError('cannot signal sudo'), subprocess.CalledProcessError(1, 'rm', stderr=b'x' * 5000))
        for error in errors:
            def run(command, **_kwargs):
                if command[0] == 'sudo': raise error
            result = invoke(run=run)
            action = result['actions'][0]
            self.assertFalse(result['success'])
            if isinstance(error, PermissionError):
                self.assertTrue(action['python_timeout_advisory'])
            else:
                self.assertEqual(len(action['stderr']), 4096)

    def test_hard_floor_is_below_preparation_target(self):
        self.assertEqual(MODULE.TARGET - MODULE.FLOOR, 2 * 1024 ** 3)
        safe = Mock(return_value=False)
        result = invoke(read_disk=lambda _: space(MODULE.FLOOR), safe=safe, run=Mock())
        self.assertFalse(result['success'])
        self.assertTrue(safe.called)

    def test_remaining_time_clamped_and_deadline_after_safe(self):
        clock = itertools.chain([0, 210, 210, 210], itertools.repeat(211))
        calls = []
        spaces = iter([space(1), space(MODULE.TARGET), space(MODULE.TARGET)])
        result = invoke(now=lambda: next(clock), read_disk=lambda _: next(spaces), run=lambda *a, **k: calls.append((a, k)))
        self.assertTrue(result['success'])
        self.assertEqual(calls[0][0][0][4], '10s')
        self.assertEqual(calls[0][1]['timeout'], 15)
        clock = iter([0, 1, 241, 241])
        run = Mock()
        self.assertFalse(invoke(now=lambda: next(clock), run=run)['success'])
        run.assert_not_called()

    def test_target_validation_mocked(self):
        with patch.object(MODULE.os.path, 'realpath', side_effect=lambda p: p), patch.object(MODULE.os, 'stat', return_value=SimpleNamespace(st_mode=stat.S_IFDIR, st_dev=7)):
            self.assertEqual(MODULE.target_info(ENV), ('/test/scratch', 7))
            for value in ('', 'relative', '/test/../scratch'):
                with self.assertRaises(ValueError): MODULE.target_info(dict(ENV, RUNNER_TEMP=value))
        with patch.object(MODULE.os.path, 'realpath', return_value='/test/scratch'), patch.object(MODULE.os, 'stat', side_effect=FileNotFoundError):
            with self.assertRaises(FileNotFoundError): MODULE.target_info(ENV)

    def test_safe_path_same_device_mounts_symlinks_and_absent(self):
        with patch.object(MODULE.os.path, 'realpath', side_effect=lambda p: p), patch.object(MODULE.os, 'lstat', return_value=SimpleNamespace(st_mode=stat.S_IFDIR, st_dev=7)):
            self.assertTrue(MODULE.safe_directory('/usr/local/go', 7, ['/other']))
            self.assertFalse(MODULE.safe_directory('/usr/local/go', 8, []))
            self.assertFalse(MODULE.safe_directory('/usr/local/go', 7, ['/usr/local/go']))
            self.assertFalse(MODULE.safe_directory('/usr/local/go', 7, ['/usr/local/go/bind']))
        with patch.object(MODULE.os.path, 'realpath', side_effect=lambda p: p), patch.object(MODULE.os, 'lstat', side_effect=FileNotFoundError):
            self.assertFalse(MODULE.safe_directory('/usr/local/go', 7, []))
        with patch.object(MODULE.os.path, 'realpath', side_effect=lambda p: p), patch.object(MODULE.os, 'lstat', return_value=SimpleNamespace(st_mode=stat.S_IFLNK, st_dev=7)):
            self.assertFalse(MODULE.safe_directory('/usr/local/go', 7, []))
        with patch.object(MODULE.os.path, 'realpath', return_value='/elsewhere'), patch.object(MODULE.os, 'lstat') as lst:
            self.assertFalse(MODULE.safe_directory('/usr/local/go', 7, [])); lst.assert_not_called()

    def test_mountinfo_escapes_and_malformed_input(self):
        with patch('builtins.open', return_value=io.StringIO('1 2 0:1 / /opt/a\\040b rw - tmpfs tmpfs rw\n')):
            self.assertEqual(MODULE.mountpoints(), ['/opt/a b'])
        with patch('builtins.open', return_value=io.StringIO('invalid\n')), self.assertRaises(ValueError):
            MODULE.mountpoints()

    def test_main_atomically_persists_checkpoints(self):
        original_cwd = os.getcwd()
        try:
            with tempfile.TemporaryDirectory() as tmp:
                os.chdir(tmp)
                def fake_prepare(_env, update):
                    update({'success': False, 'actions': [{'status': 'started'}]})
                    path = Path('representative-results/runner-preparation/receipt.json')
                    self.assertEqual(json.loads(path.read_text())['actions'][0]['status'], 'started')
                    result = {'success': True, 'actions': [{'status': 'removed'}]}
                    update(result)
                    return result
                with patch.object(MODULE, 'prepare', side_effect=fake_prepare):
                    self.assertEqual(MODULE.main(), 0)
                self.assertEqual(json.loads((Path(tmp) / 'representative-results/runner-preparation/receipt.json').read_text())['actions'][0]['status'], 'removed')
        finally:
            os.chdir(original_cwd)

    def test_workflow_fallback_records_real_step_exit_with_mocked_stat(self):
        root = Path(__file__).resolve().parents[2]
        workflow = (root / '.github/workflows/representative-bench.yml').read_text()
        block = workflow.split('name: Prepare ephemeral runner disk when needed', 1)[1]
        code = textwrap.dedent(block.split("python3 - <<'PY' || true\n", 1)[1].split('\n          PY', 1)[0])
        original_cwd = os.getcwd()
        try:
            for status, elapsed in ((1, 1), (124, 270), (137, 270), (137, 5)):
                with self.subTest(status=status, elapsed=elapsed), tempfile.TemporaryDirectory() as tmp:
                    os.chdir(tmp); dest = Path('representative-results/runner-preparation'); dest.mkdir(parents=True)
                    (dest / 'receipt.json').write_text(json.dumps({'before': space(1), 'actions': [{'status': 'started'}], 'errors': []}))
                    vfs = SimpleNamespace(f_blocks=100, f_bfree=50, f_bavail=40, f_frsize=4096)
                    with patch.dict(os.environ, {'RUNNER_TEMP': '/test/scratch', 'PREP_EXIT_CODE': str(status), 'PREP_ELAPSED': str(elapsed)}), patch.object(os.path, 'realpath', side_effect=lambda p: p), patch.object(os, 'stat', return_value=SimpleNamespace(st_dev=7)), patch.object(os, 'statvfs', return_value=vfs):
                        exec(code, {})
                    r = json.loads((dest / 'receipt.json').read_text())
                    self.assertFalse(r['success']); self.assertEqual(r['exit_code'], status)
                    self.assertEqual(r['timed_out'], status == 124 or (status == 137 and elapsed >= 270)); self.assertEqual(r['possibly_killed'], status == 137)
                    self.assertEqual(r['duration_seconds'], elapsed)
                    self.assertEqual(r['actions'][0]['status'], 'interrupted'); self.assertEqual(r['target_dev'], 7)
        finally:
            os.chdir(original_cwd)

    def test_step_summary_calls_partial_failure_possibly_modified(self):
        root = Path(__file__).resolve().parents[2]
        workflow = (root / '.github/workflows/representative-bench.yml').read_text()
        block = workflow.split('name: Summarize runner disk preparation', 1)[1]
        code = textwrap.dedent(block.split("python3 - <<'PY'\n", 1)[1].split('\n          PY', 1)[0])
        original_cwd = os.getcwd()
        try:
            for status, expected in (('started', 'True'), ('error', 'True'), ('interrupted', 'True'), ('removed', 'True'), ('skipped-unsafe-or-absent', 'False')):
                with self.subTest(status=status), tempfile.TemporaryDirectory() as tmp:
                    os.chdir(tmp)
                    dest = Path('representative-results/runner-preparation'); dest.mkdir(parents=True)
                    (dest / 'receipt.json').write_text(json.dumps({'success': False, 'actions': [{'status': status}],
                                                                   'before': space(1), 'after': space(2), 'duration_seconds': 1}))
                    summary = Path(tmp) / 'summary.md'
                    with patch.dict(os.environ, {'GITHUB_STEP_SUMMARY': str(summary)}): exec(code, {})
                    self.assertIn('possibly_modified=' + expected, summary.read_text())
        finally:
            os.chdir(original_cwd)


if __name__ == '__main__':
    unittest.main()
