"""Small semantic-oracle fixtures; no scanner, Docker, cache flush or disk load."""
from pathlib import Path
import sqlite3
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bench_v1_runtime import observe_database, seed_hash
from bench_v1_pair import arguments, host


class OracleTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def database(self, name, old_time=100, new_time=300, generation=3):
        path = self.root / name
        with sqlite3.connect(path) as db:
            db.executescript('''
                CREATE TABLE meta(generation INTEGER, snapshot_id TEXT, scan_seconds REAL);
                CREATE TABLE entries(id INTEGER PRIMARY KEY, scanned_at INTEGER, min_scanned_at INTEGER);
                CREATE TABLE roots(entry_id INTEGER, path BLOB, fs_used INTEGER);
            ''')
            db.execute('INSERT INTO meta VALUES(?,?,?)', (generation, name, 1.2))
            db.executemany('INSERT INTO entries VALUES(?,?,?)', [(1, old_time, old_time), (2, new_time, new_time)])
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


if __name__ == '__main__':
    unittest.main()
