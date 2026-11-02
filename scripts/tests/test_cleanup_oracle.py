import importlib.util
import sqlite3
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('oracle', Path(__file__).parents[1] / 'cleanup_oracle.py')
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)


class OracleTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.v1 = Path(self.tmp.name) / 'v1.db'
        self.coarse = Path(self.tmp.name) / 'coarse.db'
        b = sqlite3.connect(self.v1)
        b.executescript('''CREATE TABLE roots(entry_id INTEGER,path BLOB);
            CREATE TABLE entries(id INTEGER,parent_id INTEGER,name BLOB,kind INTEGER,allocated INTEGER,apparent INTEGER,items INTEGER,errors INTEGER,subtree_end INTEGER);
            INSERT INTO roots VALUES(1,x'2f72');
            INSERT INTO entries VALUES(1,NULL,x'72',1,21,110,3,0,4);
            INSERT INTO entries VALUES(2,1,x'64',1,10,20,1,0,3);
            INSERT INTO entries VALUES(3,2,x'66',0,9,19,0,0,3);
            INSERT INTO entries VALUES(4,1,x'73',0,1,89,0,0,4);''')
        b.execute('ALTER TABLE roots ADD COLUMN fs_total INTEGER DEFAULT 10000')
        b.execute('ALTER TABLE entries ADD COLUMN newest_mtime INTEGER DEFAULT 10')
        b.commit(); b.close()
        c = sqlite3.connect(self.coarse)
        c.executescript('''CREATE TABLE meta(root BLOB,threshold INTEGER,ppm INTEGER,capacity INTEGER);
            INSERT INTO meta VALUES(x'2f72',10,1000,10000);
            CREATE TABLE nodes(path BLOB PRIMARY KEY,parent BLOB,kind INTEGER,own_allocated INTEGER,own_apparent INTEGER,own_errors INTEGER,allocated INTEGER,apparent INTEGER,files INTEGER,dirs INTEGER,others INTEGER,errors INTEGER,rest_allocated INTEGER,rest_apparent INTEGER,rest_files INTEGER,rest_dirs INTEGER,rest_others INTEGER,rest_errors INTEGER);
            INSERT INTO nodes VALUES(x'2f72',NULL,1,10,1,0,21,110,2,2,0,0,1,89,1,0,0,0);
            INSERT INTO nodes VALUES(x'2f722f64',x'2f72',1,1,1,0,10,20,1,1,0,0,9,19,1,0,0,0);''')
        c.execute('ALTER TABLE nodes ADD COLUMN scanned_at INTEGER DEFAULT 20')
        c.execute('ALTER TABLE nodes ADD COLUMN min_scanned_at INTEGER DEFAULT 20')
        c.execute('ALTER TABLE nodes ADD COLUMN newest_mtime INTEGER DEFAULT 10')
        c.execute('ALTER TABLE nodes ADD COLUMN rest_min_scanned_at INTEGER DEFAULT 20')
        c.execute('ALTER TABLE nodes ADD COLUMN rest_newest_mtime INTEGER DEFAULT 10')
        c.commit(); c.close()

    def mutate(self, sql):
        with sqlite3.connect(self.coarse) as c:
            c.execute(sql)

    def test_valid_sparse_apparent_and_equal_threshold(self):
        self.assertEqual(oracle.verify(self.v1, self.coarse)['retained'], 2)

    def test_rejects_missing_extra_and_wrong_rest(self):
        for sql in ("DELETE FROM nodes WHERE path=x'2f722f64'", "UPDATE nodes SET rest_allocated=rest_allocated+1 WHERE parent IS NULL", "UPDATE nodes SET kind=0 WHERE parent IS NULL"):
            with self.subTest(sql=sql):
                self.setUp()
                with self.assertRaises((ValueError, sqlite3.Error)):
                    self.mutate(sql)
                    oracle.verify(self.v1, self.coarse)

    def test_refresh_crossing_and_previous_state(self):
        # The same path crosses the threshold; it was not newly created.
        self.assertEqual(oracle.verify(self.v1, self.coarse)['retained'], 2)
        with sqlite3.connect(self.v1) as b:
            b.execute("UPDATE entries SET allocated=10 WHERE id=3")
            b.execute("UPDATE entries SET allocated=11 WHERE id=2")
            b.execute("UPDATE entries SET allocated=22 WHERE id=1")
        with sqlite3.connect(self.coarse) as c:
            c.execute("UPDATE nodes SET allocated=22 WHERE parent IS NULL")
            c.execute("UPDATE nodes SET allocated=11,rest_allocated=0,rest_apparent=0,rest_files=0,rest_min_scanned_at=NULL,rest_newest_mtime=NULL WHERE path=x'2f722f64'")
            c.execute("INSERT INTO nodes(path,parent,kind,own_allocated,own_apparent,own_errors,allocated,apparent,files,dirs,others,errors,rest_allocated,rest_apparent,rest_files,rest_dirs,rest_others,rest_errors,rest_min_scanned_at,rest_newest_mtime) VALUES(x'2f722f642f66',x'2f722f64',0,10,19,0,10,19,1,0,0,0,0,0,0,0,0,0,NULL,NULL)")
        self.assertEqual(oracle.verify(self.v1, self.coarse)['retained'], 3)
        with sqlite3.connect(self.coarse) as c:
            c.execute("DELETE FROM nodes WHERE path=x'2f722f642f66'")
        with self.assertRaises(ValueError):
            oracle.verify(self.v1, self.coarse)

    def test_fixture_independent_capacity_counts_and_controls(self):
        receipt = {'capacity_bytes': 10000, 'threshold_bytes': 10,
                   'expected_counts': {'entries': 4, 'files': 2, 'dirs': 2, 'others': 0},
                   'expected_controls': {'retained': ['d'], 'omitted': ['d/f', 's']}}
        self.assertTrue(oracle.verify(self.v1, self.coarse, fixture=receipt)['success'])
        receipt['capacity_bytes'] = 9000
        with self.assertRaisesRegex(ValueError, 'capacity'):
            oracle.verify(self.v1, self.coarse, fixture=receipt)
        receipt['capacity_bytes'] = 10000
        receipt['expected_counts']['files'] = 3
        with self.assertRaisesRegex(ValueError, 'counts'):
            oracle.verify(self.v1, self.coarse, fixture=receipt)
        receipt['expected_counts']['files'] = 2
        receipt['expected_controls']['retained'].append('s')
        receipt['expected_controls']['omitted'].remove('s')
        with self.assertRaisesRegex(ValueError, 'classification'):
            oracle.verify(self.v1, self.coarse, fixture=receipt)

    def test_v2_capacity_and_kind_counts(self):
        v2 = Path(self.tmp.name) / 'v2.db'
        with sqlite3.connect(v2) as db:
            db.executescript("""CREATE TABLE live_roots(entry_id INTEGER,path BLOB,total INTEGER);
                CREATE TABLE live_rows(id INTEGER,kind INTEGER,allocated INTEGER,apparent INTEGER,items INTEGER,errors INTEGER);
                INSERT INTO live_roots VALUES(1,x'2f72',10000);
                INSERT INTO live_rows VALUES(1,1,21,110,3,0),(2,1,10,20,1,0),(3,0,9,19,0,0),(4,0,1,89,0,0);""")
        receipt = {'capacity_bytes': 10000, 'threshold_bytes': 10,
                   'expected_counts': {'entries': 4, 'files': 2, 'dirs': 2, 'others': 0},
                   'expected_controls': {'retained': ['d'], 'omitted': ['s']}}
        self.assertTrue(oracle.verify(self.v1, self.coarse, v2, receipt)['v2_checked'])
        with sqlite3.connect(v2) as db:
            db.execute('UPDATE live_rows SET kind=2 WHERE id=4')
        with self.assertRaisesRegex(ValueError, 'v2 fixture entry counts'):
            oracle.verify(self.v1, self.coarse, v2, receipt)

    def test_rejects_freshness_corruption(self):
        self.mutate('UPDATE nodes SET rest_newest_mtime=11 WHERE parent IS NULL')
        with self.assertRaisesRegex(ValueError, 'rest_newest_mtime'):
            oracle.verify(self.v1, self.coarse)
        self.mutate('UPDATE nodes SET rest_newest_mtime=10,min_scanned_at=21 WHERE parent IS NULL')
        with self.assertRaisesRegex(ValueError, 'min_scanned_at'):
            oracle.verify(self.v1, self.coarse)

    def test_rejects_threshold_and_extra_node(self):
        self.mutate('UPDATE meta SET threshold=9')
        with self.assertRaises(ValueError):
            oracle.verify(self.v1, self.coarse)
        self.mutate('UPDATE meta SET threshold=10')
        self.mutate("INSERT INTO nodes SELECT x'2f7827',path,kind,own_allocated,own_apparent,own_errors,allocated,apparent,files,dirs,others,errors,rest_allocated,rest_apparent,rest_files,rest_dirs,rest_others,rest_errors,scanned_at,min_scanned_at,newest_mtime,rest_min_scanned_at,rest_newest_mtime FROM nodes WHERE parent IS NULL")
        with self.assertRaises(ValueError):
            oracle.verify(self.v1, self.coarse)


if __name__ == '__main__':
    unittest.main()
