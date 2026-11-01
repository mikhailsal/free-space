import fcntl
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import types
import subprocess
import unittest
from unittest.mock import MagicMock, Mock, patch

MODULE = Path(__file__).resolve().parents[1] / 'bench_v1_cutover.py'
spec = importlib.util.spec_from_file_location('cutover_rehearsal', MODULE)
cutover = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cutover)


class Reader:
    def __init__(self, status):
        self.status = status

    def poll(self):
        return self.status


class CutoverGuards(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / 'v2').mkdir()
        for p in (self.root / '.lock', self.root / 'v2' / '.lock'):
            p.touch()
        (self.root / 'snapshot.db').write_text('old')
        (self.root / 'v2' / 'generations.db').write_text('history')
        (self.root / 'v2' / 'generations.db.readers').write_text('')
        (self.root / 'unrelated.keep').write_text('keep')
        self.locks = [(p.stat().st_dev, p.stat().st_ino) for p in
                      (self.root / '.lock', self.root / 'v2' / '.lock',
                       self.root / 'v2' / 'generations.db.readers')]
        self.capacity = patch.object(cutover, 'capacity')
        self.capacity.start()

    def tearDown(self):
        self.capacity.stop()
        self.temp.cleanup()

    def intact(self):
        self.assertEqual((self.root / 'snapshot.db').read_text(), 'old')
        self.assertEqual((self.root / 'v2' / 'generations.db').read_text(), 'history')

    def test_reader_startup_timeout_and_early_exit_are_bounded(self):
        reader = Mock()
        with patch.object(cutover.select, 'select', return_value=([], [], [])):
            with self.assertRaisesRegex(ValueError, 'timed out'):
                cutover.await_reader(reader)
        with patch.object(cutover.select, 'select', return_value=([reader.stdout], [], [])), \
                patch.object(cutover.os, 'read', return_value=b''):
            with self.assertRaisesRegex(ValueError, 'exited'):
                cutover.await_reader(reader)

    def test_reader_cleanup_reaps_after_broken_pipe_and_timeout(self):
        reader = Mock()
        reader.stdin.close.side_effect = BrokenPipeError()
        reader.wait.side_effect = [cutover.subprocess.TimeoutExpired('reader', 5), 0]
        cutover.close_reader(reader)
        reader.terminate.assert_called_once()
        self.assertEqual(reader.wait.call_count, 2)
        reader.stdout.close.assert_called_once()

    def test_reader_cleanup_kills_only_owned_stuck_child(self):
        reader = Mock()
        reader.wait.side_effect = [cutover.subprocess.TimeoutExpired('reader', 5),
                                   cutover.subprocess.TimeoutExpired('reader', 5), 0]
        cutover.close_reader(reader)
        reader.terminate.assert_called_once()
        reader.kill.assert_called_once()
        self.assertEqual(reader.wait.call_count, 3)

    def test_runner_environment_accepts_private_owner_and_rejects_symlink(self):
        env = {'GITHUB_ACTIONS': 'true', 'RUNNER_ENVIRONMENT': 'github-hosted',
               'RUNNER_TEMP': str(self.root)}
        self.assertEqual(cutover.runner_temp(env, self.root.stat().st_uid), self.root)
        alias = self.root / 'alias'
        alias.symlink_to(self.root / 'v2', target_is_directory=True)
        env['RUNNER_TEMP'] = str(alias)
        with self.assertRaises(ValueError):
            cutover.runner_temp(env, self.root.stat().st_uid)

    def test_call_mismatch_non_json_and_timeout(self):
        env = {'HOME': str(self.root)}
        for completed in (subprocess.CompletedProcess([], 4, '{}', '/private/path failure'),
                          subprocess.CompletedProcess([], 0, 'not-json', '')):
            with self.subTest(code=completed.returncode), patch.object(cutover.subprocess, 'run', return_value=completed):
                with self.assertRaises(cutover.RehearsalError) as caught:
                    cutover.call(Path('/unused'), env, 'scan')
                self.assertEqual(caught.exception.returncode, completed.returncode)
                self.assertNotIn('/private/path', caught.exception.stderr)
        with patch.object(cutover.subprocess, 'run', side_effect=subprocess.TimeoutExpired('cmd', 1)):
            with self.assertRaisesRegex(cutover.RehearsalError, 'timed out'):
                cutover.call(Path('/unused'), env, 'scan')

    def test_main_receipt_success_failure_interrupt_and_existing(self):
        for name, outcome in (('ok', {'valid': True}), ('error', ValueError('fail')),
                              ('interrupt', KeyboardInterrupt('stop'))):
            output = self.root / name
            def fake(_binary, _parent, update):
                self.assertEqual(json.loads(output.read_text())['status'], 'in_progress')
                update({'layout_before_reset': [{'path': 'v2/.lock', 'kind': 'file'}]})
                if isinstance(outcome, BaseException):
                    raise outcome
                return outcome
            args = ['test', '--binary', '/unused', '--output', str(output)]
            with self.subTest(name=name), patch.object(cutover, 'runner_temp', return_value=self.root), \
                    patch.object(cutover, 'rehearsal', side_effect=fake), \
                    patch.object(cutover.signal, 'signal', return_value=None), \
                    patch.dict(cutover.os.environ, {'BENCH_TARGET': 'x86_64-unknown-linux-musl'}), \
                    patch.object(cutover.sys, 'argv', args):
                if isinstance(outcome, BaseException):
                    with self.assertRaises(type(outcome)):
                        cutover.main()
                else:
                    cutover.main()
                saved = json.loads(output.read_text())
                self.assertEqual(saved['status'], {'ok': 'success', 'error': 'error',
                                                   'interrupt': 'interrupted'}[name])
                self.assertEqual(saved['target'], 'x86_64-unknown-linux-musl')
                self.assertEqual(saved['layout_before_reset'][0]['path'], 'v2/.lock')
                with self.assertRaises(ValueError):
                    cutover.main()

    def test_mocked_rehearsal_sequences_isolated_history_and_reset(self):
        binary = self.root / 'fake-binary'
        binary.write_text('not executed')
        child = Mock()
        closed = [False]
        child.poll.side_effect = lambda: 0 if closed[0] else None
        calls = []
        def fake_call(_binary, env, *args, **_kwargs):
            snap = Path(env['FREESPACE_SNAPSHOT_DIR'])
            calls.append(args)
            if args[:2] == ('--backend', 'v1'):
                (snap / '.lock').touch()
                (snap / 'snapshot.db').write_text('old-v1')
                return {'generation': 1}
            (snap / 'v2' / '.lock').touch()
            (snap / 'v2' / 'generations.db').write_text('old-v2')
            (snap / 'v2' / 'generations.db.readers').touch()
            return {'generation': 1 if len(calls) == 2 else 2}
        def close(_reader):
            closed[0] = True
        updates = []
        with patch.object(cutover, 'call', side_effect=fake_call), \
                patch.object(cutover.subprocess, 'Popen', return_value=child), \
                patch.object(cutover, 'await_reader'), patch.object(cutover, 'close_reader', side_effect=close), \
                patch.object(cutover, 'verify_fresh') as fresh:
            result = cutover.rehearsal(binary, self.root, updates.append)
        self.assertTrue(result['valid'])
        self.assertEqual(len(calls), 3)
        self.assertEqual(result['previous'], 'NO_PREV')
        self.assertTrue(any(x['path'] == 'v2/generations.db.readers' for x in updates[0]['layout_before_reset']))
        fresh.assert_called_once()

    def test_busy_real_v2_lease_blocks_reset(self):
        with (self.root / 'v2' / 'generations.db.readers').open('r') as held:
            fcntl.flock(held, fcntl.LOCK_SH | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                cutover.reset_owned(self.root, Reader(0))
        self.intact()
        self.assertEqual((self.root / 'v2' / 'generations.db.readers').stat().st_ino, self.locks[2][1])

    def test_environment_refuses_non_gh_and_root(self):
        with self.assertRaises(ValueError):
            cutover.runner_temp({}, 123)
        with self.assertRaises(ValueError):
            cutover.runner_temp({'GITHUB_ACTIONS': 'true', 'RUNNER_ENVIRONMENT': 'github-hosted'}, 0)

    def test_open_fixture_reader_and_busy_writer_refused_without_deletion(self):
        with self.assertRaises(ValueError):
            cutover.reset_owned(self.root, Reader(None))
        self.intact()
        with (self.root / 'v2' / '.lock').open('r+') as held:
            fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                cutover.reset_owned(self.root, Reader(0))
        self.intact()

    def test_bad_path_and_symlink_refused_before_any_delete(self):
        (self.root / 'foreign').symlink_to(self.root / 'snapshot.db')
        with self.assertRaises(ValueError):
            cutover.reset_owned(self.root, Reader(0))
        self.intact()

    def test_allowlisted_symlink_is_refused_before_delete(self):
        path = self.root / 'snapshot.db'
        path.unlink()
        path.symlink_to(self.root / 'unrelated.keep')
        with self.assertRaises(ValueError):
            cutover.reset_owned(self.root, Reader(0))
        self.assertEqual((self.root / 'v2' / 'generations.db').read_text(), 'history')
        self.assertEqual((self.root / 'unrelated.keep').read_text(), 'keep')

    def test_insufficient_capacity_refused_before_any_delete(self):
        with patch.object(cutover, 'capacity', side_effect=ValueError('insufficient')):
            with self.assertRaises(ValueError):
                cutover.reset_owned(self.root, Reader(0))
        self.intact()

    def test_low_bytes_and_inodes_are_independently_refused(self):
        for values in ((1, 4096, 999), (999999999, 4096, 1)):
            with self.subTest(values=values), patch.object(cutover.os, 'statvfs',
                    return_value=types.SimpleNamespace(f_bavail=values[0], f_frsize=values[1], f_favail=values[2])):
                self.capacity.stop()
                try:
                    with self.assertRaises(ValueError):
                        cutover.capacity(self.root)
                finally:
                    self.capacity.start()

    def test_fresh_snapshot_rejects_stale_name_even_when_count_matches(self):
        config = self.root / 'config.toml'
        config.write_text('backend = "v2"\n')
        original, inode = config.read_bytes(), config.stat().st_ino
        responses = [{'generation': 1, 'entries': 2}, {'generation': 1},
                     {'generation': 1}, {'error': {'code': 'NO_PREV'}}]
        for name, valid in ((b'fresh-only', True), (b'sample', False)):
            with self.subTest(name=name):
                db = Mock()
                db.execute.side_effect = [Mock(fetchone=Mock(return_value=('ok',))),
                                          Mock(fetchone=Mock(return_value=(2,))), iter([(b'',), (name,)])]
                connection = MagicMock()
                connection.__enter__.return_value = db
                connection.__exit__.return_value = False
                with patch.object(cutover, 'call', side_effect=list(responses)), \
                        patch.object(cutover.sqlite3, 'connect', return_value=connection):
                    if valid:
                        cutover.verify_fresh(Path('/unused'), {}, self.root, self.root,
                                             config, original, inode)
                    else:
                        with self.assertRaisesRegex(ValueError, 'stale content'):
                            cutover.verify_fresh(Path('/unused'), {}, self.root, self.root,
                                                 config, original, inode)
                self.assertEqual(config.read_bytes(), original)

    def test_configuration_restored_after_failed_first_scan(self):
        config = self.root / 'config.toml'
        config.write_text('backend = "v2"\n')
        original, inode = config.read_bytes(), config.stat().st_ino
        with patch.object(cutover, 'call', side_effect=RuntimeError('scan failed')):
            with self.assertRaises(RuntimeError):
                cutover.verify_fresh(Path('/unused'), {}, self.root, self.root, config, original, inode)
        self.assertEqual(config.read_bytes(), original)
        self.assertEqual(config.stat().st_ino, inode)

    def test_restore_failure_does_not_mask_original_scan_error_on_python310(self):
        class LegacyScanError(Exception):
            def __getattribute__(self, name):
                if name == 'add_note':
                    raise AttributeError(name)
                return super().__getattribute__(name)

        config = self.root / 'config.toml'
        config.write_text('backend = "v2"\n')
        original, inode = config.read_bytes(), config.stat().st_ino
        failure = LegacyScanError('original scan failure')
        with patch.object(cutover, 'call', side_effect=failure), \
                patch.object(Path, 'write_bytes', side_effect=OSError('restore failed')):
            with self.assertRaisesRegex(LegacyScanError, 'original scan failure') as caught:
                cutover.verify_fresh(Path('/unused'), {}, self.root, self.root, config, original, inode)
        self.assertIs(caught.exception, failure)
        self.assertEqual(failure.__notes__, ['fixture configuration restore failed: OSError'])

    def test_owned_reset_preserves_unrelated_and_lock_inodes(self):
        cutover.reset_owned(self.root, Reader(0))
        self.assertFalse((self.root / 'snapshot.db').exists())
        self.assertFalse((self.root / 'v2' / 'generations.db').exists())
        self.assertEqual((self.root / 'unrelated.keep').read_text(), 'keep')
        self.assertTrue((self.root / 'v2' / 'generations.db.readers').is_file())
        self.assertEqual([(p.stat().st_dev, p.stat().st_ino) for p in
                          (self.root / '.lock', self.root / 'v2' / '.lock',
                       self.root / 'v2' / 'generations.db.readers')], self.locks)


if __name__ == '__main__':
    unittest.main()
