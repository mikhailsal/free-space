"""Small semantic-oracle fixtures; no scanner, Docker, cache flush or disk load."""
from pathlib import Path
import json
import os
import sqlite3
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bench_v1_runtime import observe_database, seed_hash, normalize
from bench_v1_pair import arguments, host, seed_timestamp


class OracleTests(unittest.TestCase):
    def test_shell_argv_dispatch_and_memory_modes(self):
        import runpy
        from types import ModuleType
        from unittest.mock import patch, Mock
        from bench_v1_runtime import container_memory
        spec = {'memory_mib': 512, 'observer_memory_mib': 4096}
        self.assertEqual(container_memory(spec, 'worker'), 512)
        self.assertEqual(container_memory(spec, 'observer'), 4096)
        fake = ModuleType('bench_v1_pair')
        fake.main = Mock(return_value=0)
        argv = ['bench-v2-container.py', '--v1-pairs', '--subtree-entries', '10000', '--memory', '512']
        with patch.dict(sys.modules, {'bench_v1_pair': fake}), patch.object(sys, 'argv', argv):
            with self.assertRaises(SystemExit) as exit_status:
                runpy.run_path(str(Path(__file__).resolve().parents[1] / 'bench-v2-container.py'), run_name='__main__')
        self.assertEqual(exit_status.exception.code, 0)
        fake.main.assert_called_once_with(argv[2:])
    def test_qualification_fixture_counts_and_shared_mutation(self):
        from unittest.mock import patch
        from bench_v1_fixture import prepare, mutate, STAMP
        for files, subtree in ((100, 100), (1000, 7)):
            with self.subTest(files=files, subtree=subtree), tempfile.TemporaryDirectory() as tmp:
                tree = Path(tmp) / 'tree'
                with patch('bench_v1_fixture.os.sync'):
                    receipt = prepare(tree, files, subtree)
                    count = lambda: sum(1 + len(names) for _, _, names in os.walk(tree))
                    self.assertEqual(count(), receipt['expected_entries'])
                    self.assertEqual(receipt['distribution']['small_files'] + receipt['distribution']['outer_files'], files)
                    self.assertEqual(receipt['shape'] == 'mixed-parameterized-subtree-v1', subtree != 100)
                    self.assertEqual(len(list((tree / 'small').iterdir())) + 1, subtree)
                    original = (tree / 'small' / 'f00000').stat().st_ino
                    unchanged_ids = {p.name: p.stat().st_ino for p in (tree / 'small').iterdir()}
                    mutation = mutate(tree)
                    self.assertEqual(mutation['recipe'], 'rename-size-mtime')
                    self.assertEqual(mutation['renamed_inode'], original)
                    self.assertEqual(count(), receipt['expected_entries'])
                    self.assertEqual((tree / 'small' / 'renamed').stat().st_ino, original)
                    self.assertEqual((tree / 'small' / 'f00001').stat().st_mtime, STAMP + 60)
                    changed_ids = {p.name: p.stat().st_ino for p in (tree / 'small').iterdir()}
                    self.assertEqual(changed_ids, {('renamed' if k == 'f00000' else k): v
                                                   for k, v in unchanged_ids.items()})

    def test_qualification_argument_choices_and_defaults(self):
        import contextlib
        import io
        base = ['--baseline', '/missing', '--candidate', '/missing', '--output', '/unused']
        defaults = arguments(base)
        self.assertEqual((defaults.subtree_entries, defaults.scenario, defaults.memory), (100, 'unchanged', 4096))
        for value in ('100', '10000', '100000'):
            self.assertEqual(arguments(base + ['--files', '100000', '--subtree-entries', value]).subtree_entries, int(value))
        for extra in (['--subtree-entries', '101'], ['--subtree-entries', '10000', '--files', '100'], ['--memory', '128'], ['--memory', '256']):
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                arguments(base + extra)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def database(self, name, old_time=100, new_time=300, generation=3):
        path = self.root / name
        with sqlite3.connect(path) as db:
            db.executescript('''
                CREATE TABLE meta(generation INTEGER, snapshot_id TEXT, scan_seconds REAL);
                CREATE TABLE entries(id INTEGER PRIMARY KEY, parent_id INTEGER, kind INTEGER, items INTEGER,
                    subtree_end INTEGER, scanned_at INTEGER, min_scanned_at INTEGER, mtime INTEGER, newest_mtime INTEGER);
                CREATE TABLE roots(entry_id INTEGER, path BLOB, fs_used INTEGER);
            ''')
            db.execute('INSERT INTO meta VALUES(?,?,?)', (generation, name, 1.2))
            db.executemany('INSERT INTO entries VALUES(?,?,?,?,?,?,?,?,?)', [
                (0, None, 1, 2, 2, 50, min(50, old_time, new_time), 0, 10),
                (1, 0, 0, 0, 1, old_time, old_time, 10, 10),
                (2, 0, 0, 0, 2, new_time, new_time, 10, 10)])
            db.execute('INSERT INTO roots VALUES(1,?,?)', (b'/scratch/tree', 123 if name == 'a.db' else 987))
        return path

    def digest(self, path):
        return observe_database(path, cutoff=200, deadline=time.monotonic() + 5)[1]

    def test_new_times_and_generated_metadata_normalize(self):
        a = self.database('a.db', new_time=300)
        b = self.database('b.db', new_time=400)
        self.assertEqual(self.digest(a), self.digest(b))

    def test_untouched_timestamp_change_is_detected(self):
        self.assertNotEqual(self.digest(self.database('a.db')), self.digest(self.database('b.db', old_time=101)))

    def test_generation_change_is_detected(self):
        self.assertNotEqual(self.digest(self.database('a.db')), self.digest(self.database('b.db', generation=4)))

    def test_invalid_freshness_rejected(self):
        path = self.database('a.db')
        with sqlite3.connect(path) as db:
            db.execute('UPDATE entries SET min_scanned_at=scanned_at+1 WHERE id=1')
        with self.assertRaisesRegex(ValueError, 'freshness'):
            self.digest(path)

    def test_normalized_new_freshness_still_checks_ancestor_minimum(self):
        path = self.database('a.db', old_time=300, new_time=400)
        with sqlite3.connect(path) as db:
            db.execute('UPDATE entries SET scanned_at=500, min_scanned_at=350 WHERE id=0')
        with self.assertRaisesRegex(ValueError, 'freshness minimum'):
            self.digest(path)

    def test_empty_directory_does_not_mask_negative_file_mtime(self):
        path = self.database('a.db')
        with sqlite3.connect(path) as db:
            db.execute('UPDATE entries SET kind=1, mtime=0, newest_mtime=0 WHERE id=1')
            db.execute('UPDATE entries SET mtime=-10, newest_mtime=-10 WHERE id=2')
            db.execute('UPDATE entries SET newest_mtime=-10 WHERE id=0')
        self.digest(path)
        with sqlite3.connect(path) as db:
            db.execute('UPDATE entries SET newest_mtime=0 WHERE id=0')
        with self.assertRaisesRegex(ValueError, 'newest file mtime'):
            self.digest(path)

    def test_invalid_preorder_structure_rejected(self):
        for mutation in ('UPDATE entries SET id=3 WHERE id=2',
                         'UPDATE entries SET parent_id=NULL WHERE id=2',
                         'UPDATE entries SET subtree_end=2 WHERE id=1',
                         'UPDATE entries SET items=1 WHERE id=0'):
            with self.subTest(mutation=mutation):
                path = self.database('structure.db')
                with sqlite3.connect(path) as db:
                    db.execute(mutation)
                with self.assertRaises(ValueError):
                    self.digest(path)
                path.unlink()

    def test_nonempty_wal_rejected(self):
        path = self.database('a.db')
        path.with_name('a.db-wal').write_bytes(b'not empty')
        with self.assertRaisesRegex(ValueError, 'WAL'):
            self.digest(path)

    def test_seed_hash_includes_state_and_rejects_links(self):
        self.database('snapshot.db')
        self.database('snapshot.prev.db')
        state = self.root / 'state.json'
        state.write_text('{"generation":2}')
        before = seed_hash(self.root)
        state.write_text('{"generation":3}')
        self.assertNotEqual(before, seed_hash(self.root))
        (self.root / 'unsafe').symlink_to('/etc/passwd')
        with self.assertRaisesRegex(ValueError, 'symlink'):
            seed_hash(self.root)

    def test_seed_cutoff_preserves_times_after_generation_start(self):
        for name in ('snapshot.db', 'snapshot.prev.db'):
            self.database(name, old_time=100, new_time=300)
            with sqlite3.connect(self.root / name) as db:
                db.execute('ALTER TABLE meta ADD COLUMN scanned_at INTEGER')
                db.execute('UPDATE meta SET scanned_at=90')
        self.assertEqual(seed_timestamp(self.root), 300)
        self.assertEqual(normalize('entries', ['scanned_at'], [300], seed_timestamp(self.root)), [300])
        self.assertEqual(normalize('entries', ['scanned_at'], [301], seed_timestamp(self.root)), ['new observation'])

    def test_host_refuses_local_execution_before_allocation(self):
        from unittest.mock import patch
        args = arguments(['--baseline', '/missing', '--candidate', '/missing', '--output', str(self.root / 'out')])
        with patch.dict('os.environ', {}, clear=True), self.assertRaisesRegex(RuntimeError, 'GitHub-hosted'):
            host(args)
        self.assertFalse((self.root / 'out').exists())

    def test_changed_scan_not_silently_accepted(self):
        import contextlib
        import io
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            arguments(['--baseline', '/missing', '--candidate', '/missing', '--output', '/unused', '--scenario', 'changed'])

    def test_timestamp_boundary_and_null(self):
        self.assertEqual(normalize('entries', ['scanned_at'], [200], 200), [200])
        self.assertEqual(normalize('entries', ['scanned_at'], [201], 200), ['new observation'])
        with self.assertRaisesRegex(ValueError, 'timestamp'):
            normalize('entries', ['scanned_at'], [None], 200)

    def test_mixed_fixture_entry_count_and_mutation(self):
        from unittest.mock import patch
        from bench_v1_fixture import prepare, mutate
        tree = self.root / 'tree'
        with patch('bench_v1_fixture.os.sync'):
            receipt = prepare(tree, 2100)
            count = sum(1 + len(files) for _, _, files in os.walk(tree))
            self.assertEqual(receipt['expected_entries'], count)
            self.assertEqual(len(list((tree / 'small').iterdir())) + 1, 100)
            a = next(tree.rglob('f000000998'))
            b = next(tree.rglob('f000000999'))
            self.assertEqual(a.stat().st_ino, b.stat().st_ino)
            mutate(tree)
            self.assertEqual(sum(1 + len(files) for _, _, files in os.walk(tree)), count)

    def test_early_admission_failure_still_writes_invalid_summary(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        out = self.root / 'out'
        args = arguments(['--baseline', '/fake', '--candidate', '/fake', '--output', str(out)])
        helper = SimpleNamespace(dump=lambda path, value: path.write_text(json.dumps(value)), capture=lambda args: '')
        source = dict(sha256='a'*64, source_revision='revision', build=dict(
            compiler='same', target='same', flags='', profile='release', release_profile={}, cargo_config={}))
        env = dict(GITHUB_ACTIONS='true', RUNNER_ENVIRONMENT='github-hosted',
                   FREESPACE_BENCHMARK_RUN_TOKEN='test', RUNNER_TEMP=str(self.root))
        import contextlib
        import io
        with patch.dict(os.environ, env), patch('bench_v1_pair.legacy', return_value=helper), \
                patch('bench_v1_pair.provenance', return_value=source), \
                patch('bench_v1_pair.subprocess.run'), \
                patch('bench_v1_pair.check_capacity', return_value={'success': False}), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(host(args), 1)
        self.assertFalse(json.loads((out / 'summary.json').read_text())['valid'])
        self.assertTrue(json.loads((out / 'status.json').read_text())['failures'])


if __name__ == '__main__':
    unittest.main()
