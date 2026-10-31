"""Small semantic-oracle fixtures; no scanner, Docker, cache flush or disk load."""
from pathlib import Path
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bench_v1_runtime import observe_database, seed_hash, normalize
from bench_v1_pair import arguments, host, seed_timestamp


class OracleTests(unittest.TestCase):
    def test_runtime_import_does_not_load_summary_or_admission(self):
        script = "import sys; import bench_v1_runtime; assert 'bench_v1_summary' not in sys.modules; assert 'bench_v1_admission' not in sys.modules"
        result = subprocess.run([sys.executable, '-c', script], cwd=Path(__file__).resolve().parents[1],
                                capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_changed_full_host_rejects_noncanonical_baseline_before_fixture(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as tmp:
            args = arguments(['--baseline', '/fake', '--candidate', '/fake', '--output', str(Path(tmp)/'out'),
                              '--operation', 'refresh', '--scenario', 'changed-full'])
            source = dict(sha256='a'*64, source_revision='afce6fd', build={})
            env = dict(GITHUB_ACTIONS='true', RUNNER_ENVIRONMENT='github-hosted')
            with patch.dict(os.environ, env), patch('bench_v1_pair.legacy', return_value=SimpleNamespace()), \
                 patch('bench_v1_pair.provenance', return_value=source), \
                 patch('bench_v1_pair.prepare') as prepare_mock, patch('bench_v1_pair.subprocess.run') as run:
                with self.assertRaisesRegex(ValueError, 'canonical baseline'):
                    host(args)
                prepare_mock.assert_not_called()
                run.assert_not_called()
    def test_signed_sqlite_identity_and_lookup_deadline(self):
        from unittest.mock import Mock, patch
        from bench_v1_runtime import full_link_facts
        from bench_v1_summary import sqlite_i64
        self.assertEqual([sqlite_i64(x) for x in (0, 2**63-1, 2**63, 2**64-1)],
                         [0, 2**63-1, -(2**63), -1])
        for value in (-1, 2**64, True):
            with self.assertRaises(ValueError):
                sqlite_i64(value)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'snapshot.db'
            with self.assertRaises(TimeoutError):
                full_link_facts(path, {}, 'seed', time.monotonic() - 1)
            db = Mock()
            db.execute.side_effect = sqlite3.OperationalError('interrupted')
            with patch('bench_v1_runtime.sqlite3.connect', return_value=db):
                with self.assertRaisesRegex(sqlite3.OperationalError, 'interrupted'):
                    full_link_facts(path, {}, 'seed', time.monotonic() + 10)
            db.set_progress_handler.assert_called_once()
            db.close.assert_called_once()
            for caller_deadline, elapsed in ((100, 11), (5, 6)):
                db = Mock()
                def interrupted(*_):
                    self.assertEqual(db.set_progress_handler.call_args.args[0](), 1)
                    raise sqlite3.OperationalError('interrupted')
                db.execute.side_effect = interrupted
                with patch('bench_v1_runtime.sqlite3.connect', return_value=db), \
                     patch('bench_v1_runtime.time.monotonic', side_effect=[0, 0, elapsed]):
                    with self.assertRaisesRegex(sqlite3.OperationalError, 'interrupted'):
                        full_link_facts(path, {}, 'seed', caller_deadline)
                db.close.assert_called_once()

    def test_changed_full_inside_wires_facts_and_reports_lookup_failure(self):
        from unittest.mock import Mock, patch
        from types import SimpleNamespace
        from bench_v1_runtime import inside
        with tempfile.TemporaryDirectory() as tmp:
            spec = Path(tmp) / 'spec.json'
            spec.write_text(json.dumps(dict(id='x', seed_sha256='a'*64, seed_timestamp=1,
                                            timeout_seconds=10, expected_entries=3,
                                            scenario='changed-full', mutation_plan={'key': 1})))
            output = []
            helper = SimpleNamespace(cgroup=lambda: None, cg_read=lambda _: {},
                                     snapshot_integrity=lambda _: {'snapshot.prev.db': {'sha256': 'b'*64}},
                                     dump=lambda _, value: output.append(value))
            with patch('bench_v1_runtime.legacy', return_value=helper), \
                 patch('bench_v1_runtime.observe_database', return_value=(3, 'digest')), \
                 patch('bench_v1_runtime.full_link_facts', side_effect=[{'current': 1}, {'previous': 2}]) as facts:
                self.assertEqual(inside('observer', spec), 0)
                self.assertEqual([c.args[2] for c in facts.call_args_list], ['refresh', 'seed'])
                self.assertTrue(all(c.args[1] == {'key': 1} for c in facts.call_args_list))
                self.assertEqual(output[-1]['previous_sha256'], 'b'*64)
                self.assertEqual(output[-1]['full_link_facts']['previous'], {'previous': 2})
            with patch('bench_v1_runtime.legacy', return_value=helper), \
                 patch('bench_v1_runtime.observe_database', return_value=(3, 'digest')), \
                 patch('bench_v1_runtime.full_link_facts', side_effect=TimeoutError('deadline')):
                self.assertEqual(inside('observer', spec), 1)
                self.assertIn('deadline', output[-1]['errors']['observer'])

    def test_changed_full_host_wires_seed_and_both_arm_specs(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        import contextlib
        import io
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            out = root / 'out'
            prep = root / 'prep.json'
            prep.write_text('{}')
            args = arguments(['--baseline', '/fake', '--candidate', '/fake', '--output', str(out),
                              '--operation', 'refresh', '--scenario', 'changed-full'])
            plan = {'example': 'plan'}
            fixture = {'expected_entries': 103, 'expected_subtree_entries': 100, 'mutation_plan': plan}
            source = dict(sha256='a'*64, source_revision='e8265778ef78ec4acd9da1375eedb0be37a123f5', build=dict(
                compiler='same', target='same', flags='', profile='release', release_profile={}, cargo_config={}))
            def flush(directory, name):
                (directory / (name + '.cache-flush.json')).write_text('{}')
            helper = SimpleNamespace(dump=lambda path, value: path.write_text(json.dumps(value)),
                                     capture=lambda argv: '' if argv[:2] == ['docker', 'ps'] else '{}',
                                     occupancy=lambda _: {'total': {'allocated_bytes': 1}},
                                     filesystem_used=lambda _: 0, flush_caches=flush)
            def fake_container(image, binary, seed, tree, output, spec_path, spec, mode, label):
                if spec['id'].startswith('seed-'):
                    (output / spec['id'] / 'metrics.json').write_text('{"validated": true}')
            env = dict(GITHUB_ACTIONS='true', RUNNER_ENVIRONMENT='github-hosted',
                       FREESPACE_BENCHMARK_RUN_TOKEN='test', RUNNER_TEMP=tmp, BENCH_PREP_RECEIPT=str(prep))
            with patch.dict(os.environ, env), patch('bench_v1_pair.legacy', return_value=helper), \
                 patch('bench_v1_pair.provenance', return_value=source), \
                 patch('bench_v1_pair.subprocess.run'), patch('bench_v1_pair.check_capacity', return_value={'success': True}), \
                 patch('bench_v1_pair.runner_preparation_context', return_value={}), \
                 patch('bench_v1_pair.prepare', return_value=fixture) as prepare_mock, \
                 patch('bench_v1_pair.container', side_effect=fake_container), \
                 patch('bench_v1_pair.seed_hash', side_effect=lambda _, **kw: ('c'*64, {'snapshot.prev.db': 'b'*64}) if kw else 'c'*64), \
                 patch('bench_v1_pair.file_hash', return_value='b'*64), \
                 patch('bench_v1_pair.seed_timestamp', return_value=1), \
                 patch('bench_v1_pair.full_link_facts', side_effect=[{'current': 1}, {'previous': 2}]) as facts, \
                 patch('bench_v1_pair.mutate_full', return_value={'recipe': 'full'}) as mutate_mock, \
                 patch('bench_v1_pair.time.sleep'), \
                 patch('bench_v1_pair.summarize', return_value={'valid': True}), \
                 patch('bench_v1_pair.markdown', return_value='ok'), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(host(args), 0)
            self.assertEqual(prepare_mock.call_args.args[-1], 'changed-full')
            mutate_mock.assert_called_once()
            self.assertEqual([c.args[0].name for c in facts.call_args_list], ['snapshot.db', 'snapshot.prev.db'])
            manifest = json.loads((out / 'manifest.json').read_text())
            self.assertEqual(manifest['seed_link_facts'], {'snapshot.db': {'current': 1}, 'snapshot.prev.db': {'previous': 2}})
            self.assertEqual(manifest['seed_previous_sha256'], 'b'*64)
            for arm in ('baseline', 'candidate'):
                self.assertEqual(json.loads((out / f'p1-{arm}-v1' / 'spec.json').read_text())['mutation_plan'], plan)

    def test_targeted_hardlink_seed_and_refresh_facts(self):
        from unittest.mock import patch
        from bench_v1_fixture import prepare
        from bench_v1_runtime import full_link_facts
        from bench_v1_summary import full_link_evidence_ok, sqlite_i64
        with tempfile.TemporaryDirectory() as tmp, patch('bench_v1_fixture.os.sync'):
            root = Path(tmp)
            plan = prepare(root / 'tree', 100, 100, 'changed-full')['mutation_plan']
            plan.update(old_owner_dev=2**63, old_owner_inode=2**64-1,
                        outside_owner_dev=2**63+1, outside_owner_inode=2**64-2)
            dbpath = root / 'snapshot.db'
            with sqlite3.connect(dbpath) as db:
                db.executescript('CREATE TABLE roots(entry_id INTEGER,path BLOB); CREATE TABLE entries(id INTEGER PRIMARY KEY,parent_id INTEGER,name BLOB,flags INTEGER,allocated INTEGER,apparent INTEGER,mtime INTEGER,nlink INTEGER); CREATE TABLE hardlinks(entry_id INTEGER,dev INTEGER,ino INTEGER,allocated INTEGER,apparent INTEGER);')
                db.execute('INSERT INTO roots VALUES (0,?)', (b'/scratch/tree',))
                def row(i, parent, name, flags=0, alloc=0, apparent=0, mtime=1700000000, nlink=1):
                    db.execute('INSERT INTO entries VALUES (?,?,?,?,?,?,?,?)', (i, parent, name.encode(), flags, alloc, apparent, mtime, nlink))
                row(0, None, 'root'); row(1, 0, 'small'); row(2, 1, 'nested'); row(3, 0, 'unrelated')
                row(4, 2, 'owner', 0, plan['old_owner_allocated'], plan['old_owner_size'], nlink=2)
                row(5, 3, 'old-link', 2, 0, 0, nlink=2)
                row(6, 3, 'outside-owner', 0, plan['outside_owner_allocated'], plan['outside_owner_size'])
                for ident in (4, 5):
                    db.execute('INSERT INTO hardlinks VALUES (?,?,?,?,?)', (ident, sqlite_i64(plan['old_owner_dev']), sqlite_i64(plan['old_owner_inode']), plan['old_owner_allocated'], plan['old_owner_size']))
            seed = full_link_facts(dbpath, plan, 'seed', time.monotonic() + 10)
            self.assertTrue(full_link_evidence_ok(seed, plan, 'seed'))
            with sqlite3.connect(dbpath) as db:
                db.execute('DELETE FROM entries WHERE id IN (2,4)')
                db.execute('DELETE FROM hardlinks WHERE entry_id=4')
                db.execute('UPDATE entries SET flags=0, allocated=?, apparent=? WHERE id=5', (plan['old_owner_allocated'], plan['old_owner_size']))
                db.execute('INSERT INTO entries VALUES (7,1,?,0,0,0,1700000000,1)', (b'replacement',))
                db.execute('INSERT INTO entries VALUES (8,7,?,2,0,0,1700000000,2)', (b'new-link',))
                db.execute('INSERT INTO hardlinks VALUES (8,?,?,?,?)', (sqlite_i64(plan['outside_owner_dev']), sqlite_i64(plan['outside_owner_inode']), plan['outside_owner_allocated'], plan['outside_owner_size']))
            refreshed = full_link_facts(dbpath, plan, 'refresh', time.monotonic() + 10)
            self.assertTrue(full_link_evidence_ok(refreshed, plan, 'refresh'))
            refreshed[plan['new_link_path']][0] = 0
            self.assertFalse(full_link_evidence_ok(refreshed, plan, 'refresh'))

    def test_full_mutation_preserves_counts_and_cross_boundary_inodes(self):
        from unittest.mock import patch
        from bench_v1_fixture import prepare, mutate_full, STAMP
        from bench_v1_summary import full_mutation_ok
        for files, size in ((100, 100), (1000, 100)):
            with self.subTest(files=files), tempfile.TemporaryDirectory() as tmp, patch('bench_v1_fixture.os.sync'):
                tree = Path(tmp) / 'tree'
                fixture = prepare(tree, files, size, 'changed-full')
                count = lambda: sum(1 + len(names) for _, _, names in os.walk(tree))
                subtree = lambda: sum(1 + len(names) for _, _, names in os.walk(tree / 'small'))
                self.assertEqual((count(), subtree()), (fixture['expected_entries'], size))
                self.assertEqual(fixture['distribution']['small_files'] + fixture['distribution']['outer_files'], files)
                old = (tree / 'small/nested/owner').stat().st_ino
                renamed = (tree / 'small/f00000').stat().st_ino
                deleted = (tree / 'small/f00002').stat().st_ino
                self.assertEqual(old, (tree / 'unrelated/old-link').stat().st_ino)
                new = (tree / 'unrelated/outside-owner').stat().st_ino
                receipt = mutate_full(tree)
                self.assertEqual((count(), subtree()), (fixture['expected_entries'], size))
                self.assertFalse((tree / 'small/nested').exists())
                self.assertFalse((tree / 'small/f00002').exists())
                self.assertEqual((tree / 'small/renamed').stat().st_ino, renamed)
                self.assertNotEqual((tree / 'small/added').stat().st_ino, deleted)
                self.assertEqual((tree / 'small/f00001').stat().st_size, 8192)
                for path in ('small/f00001', 'small/added', 'small/replacement', 'small/replacement/new-file', 'small'):
                    self.assertEqual((tree / path).stat().st_mtime, STAMP + 60)
                self.assertEqual((tree / 'small/f00003').stat().st_mtime, STAMP)
                self.assertEqual(old, (tree / 'unrelated/old-link').stat().st_ino)
                self.assertEqual(new, (tree / 'small/replacement/new-link').stat().st_ino)
                self.assertTrue(full_mutation_ok(receipt, fixture))
                receipt['new_link_inode'] = old
                self.assertFalse(full_mutation_ok(receipt, fixture))
        for scenario, size in (('invalid', 100), ('changed-full', 7)):
            with tempfile.TemporaryDirectory() as tmp, self.assertRaises(ValueError):
                prepare(Path(tmp) / 'tree', 100, size, scenario)

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
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            arguments(base + ['--scenario', 'changed-full'])
        self.assertEqual(arguments(base + ['--operation', 'refresh', '--scenario', 'changed-full']).scenario, 'changed-full')

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
        combined, files = seed_hash(self.root, include_files=True)
        self.assertEqual(combined, before)
        self.assertEqual(files['snapshot.prev.db'], __import__('hashlib').sha256((self.root / 'snapshot.prev.db').read_bytes()).hexdigest())
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
        prep = self.root / 'prep.json'
        prep.write_text(json.dumps({'success': True, 'actions': [], 'before': {'available_bytes': 44 * 1024**3},
                                    'after': {'available_bytes': 44 * 1024**3}, 'duration_seconds': 0,
                                    'floor_bytes': 42 * 1024**3, 'target_bytes': 44 * 1024**3}))
        env = dict(GITHUB_ACTIONS='true', RUNNER_ENVIRONMENT='github-hosted',
                   FREESPACE_BENCHMARK_RUN_TOKEN='test', RUNNER_TEMP=str(self.root), BENCH_PREP_RECEIPT=str(prep))
        import contextlib
        import io
        with patch.dict(os.environ, env), patch('bench_v1_pair.legacy', return_value=helper), \
                patch('bench_v1_pair.provenance', return_value=source), \
                patch('bench_v1_pair.subprocess.run'), \
                patch('bench_v1_pair.check_capacity', return_value={'success': False}), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(host(args), 1)
        self.assertFalse(json.loads((out / 'summary.json').read_text())['valid'])

    def test_missing_or_malformed_preparation_is_recorded_in_host_failure(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        import contextlib
        import io
        helper = SimpleNamespace(dump=lambda path, value: path.write_text(json.dumps(value)), capture=lambda args: '')
        source = dict(sha256='a'*64, source_revision='revision', build=dict(
            compiler='same', target='same', flags='', profile='release', release_profile={}, cargo_config={}))
        bad = self.root / 'bad.json'
        bad.write_text('{not-json')
        for case, prep in (('missing', ''), ('malformed', str(bad))):
            with self.subTest(case=case):
                out = self.root / ('out-' + case)
                args = arguments(['--baseline', '/fake', '--candidate', '/fake', '--output', str(out)])
                env = dict(GITHUB_ACTIONS='true', RUNNER_ENVIRONMENT='github-hosted',
                           FREESPACE_BENCHMARK_RUN_TOKEN='test', RUNNER_TEMP=str(self.root), BENCH_PREP_RECEIPT=prep)
                with patch.dict(os.environ, env), patch('bench_v1_pair.legacy', return_value=helper), \
                        patch('bench_v1_pair.provenance', return_value=source), patch('bench_v1_pair.subprocess.run'), \
                        contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(host(args), 1)
                status = json.loads((out / 'status.json').read_text())
                self.assertFalse(status['success'])
                self.assertIn('runner preparation receipt is required' if case == 'missing' else 'JSONDecodeError', status['failures'][0])
                self.assertFalse(json.loads((out / 'summary.json').read_text())['valid'])
        self.assertTrue(json.loads((out / 'status.json').read_text())['failures'])


if __name__ == '__main__':
    unittest.main()
