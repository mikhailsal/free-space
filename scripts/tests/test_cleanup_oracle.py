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
        b.commit(); b.close()
        c = sqlite3.connect(self.coarse)
        c.executescript('''CREATE TABLE meta(root BLOB,threshold INTEGER,ppm INTEGER,capacity INTEGER);
            INSERT INTO meta VALUES(x'2f72',10,1000,10000);
            CREATE TABLE nodes(path BLOB PRIMARY KEY,parent BLOB,kind INTEGER,own_allocated INTEGER,own_apparent INTEGER,own_errors INTEGER,allocated INTEGER,apparent INTEGER,files INTEGER,dirs INTEGER,others INTEGER,errors INTEGER,rest_allocated INTEGER,rest_apparent INTEGER,rest_files INTEGER,rest_dirs INTEGER,rest_others INTEGER,rest_errors INTEGER);
            INSERT INTO nodes VALUES(x'2f72',NULL,1,10,1,0,21,110,2,2,0,0,1,89,1,0,0,0);
            INSERT INTO nodes VALUES(x'2f722f64',x'2f72',1,1,1,0,10,20,1,1,0,0,9,19,1,0,0,0);''')
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
            c.execute("UPDATE nodes SET allocated=11,rest_allocated=0,rest_apparent=0,rest_files=0 WHERE path=x'2f722f64'")
            c.execute("INSERT INTO nodes VALUES(x'2f722f642f66',x'2f722f64',0,10,19,0,10,19,1,0,0,0,0,0,0,0,0,0)")
        self.assertEqual(oracle.verify(self.v1, self.coarse)['retained'], 3)
        with sqlite3.connect(self.coarse) as c:
            c.execute("DELETE FROM nodes WHERE path=x'2f722f642f66'")
        with self.assertRaises(ValueError):
            oracle.verify(self.v1, self.coarse)

    def test_rejects_threshold_and_extra_node(self):
        self.mutate('UPDATE meta SET threshold=9')
        with self.assertRaises(ValueError):
            oracle.verify(self.v1, self.coarse)
        self.mutate('UPDATE meta SET threshold=10')
        self.mutate("INSERT INTO nodes SELECT x'2f7827',path,kind,own_allocated,own_apparent,own_errors,allocated,apparent,files,dirs,others,errors,rest_allocated,rest_apparent,rest_files,rest_dirs,rest_others,rest_errors FROM nodes WHERE parent IS NULL")
        with self.assertRaises(ValueError):
            oracle.verify(self.v1, self.coarse)


if __name__ == '__main__':
    unittest.main()
