import importlib.util
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).parents[1]
sys.path.insert(0, str(SCRIPTS))
import cleanup_buckets as cb

spec = importlib.util.spec_from_file_location('oracle', SCRIPTS / 'cleanup_oracle.py')
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)

V1_SCHEMA = '''CREATE TABLE roots(entry_id INTEGER,path BLOB,fs_total INTEGER DEFAULT 10000);
    CREATE TABLE entries(id INTEGER,parent_id INTEGER,name BLOB,kind INTEGER,allocated INTEGER,apparent INTEGER,items INTEGER,errors INTEGER,subtree_end INTEGER,newest_mtime INTEGER DEFAULT 10);'''
V1_COLUMNS = 'id,parent_id,name,kind,allocated,apparent,items,errors,subtree_end'

COARSE_SCHEMA = '''CREATE TABLE meta(root BLOB,threshold INTEGER,ppm INTEGER,capacity INTEGER,has_hardlinks INTEGER,format INTEGER,slot_bits INTEGER,bucket_entries INTEGER);
    INSERT INTO meta VALUES(x'2f72',10,1000,10000,0,2,12,4096);
    CREATE TABLE nodes(path BLOB PRIMARY KEY,parent BLOB,kind INTEGER,own_allocated INTEGER,own_apparent INTEGER,own_errors INTEGER,scanned_at INTEGER,allocated INTEGER,apparent INTEGER,files INTEGER,dirs INTEGER,others INTEGER,errors INTEGER,links INTEGER,min_scanned_at INTEGER,newest_mtime INTEGER);
    CREATE TABLE buckets(parent BLOB,lo INTEGER,hi INTEGER,allocated INTEGER,apparent INTEGER,files INTEGER,dirs INTEGER,others INTEGER,errors INTEGER,links INTEGER,min_scanned_at INTEGER,newest_mtime INTEGER,PRIMARY KEY(parent,lo));'''

ROOT, D, F = "x'2f72'", "x'2f722f64'", "x'2f722f642f66'"


def put_buckets(c, parent, slots):
    for lo, hi, st in cb.tile(slots):
        c.execute('INSERT INTO buckets VALUES(?,?,?,?,?,?,?,?,?,0,?,?)',
                  (parent, lo, hi, st.allocated, st.apparent, st.files, st.dirs, st.others, st.errors, 20 if st.entries else None, st.newest))


def folded(allocated, apparent, files, dirs=0):
    stats = cb.Stats()
    stats.add_totals([allocated, apparent, files, dirs, 0, 0], 10)
    return stats


class OracleTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.v1 = Path(self.tmp.name) / 'v1.db'
        self.coarse = Path(self.tmp.name) / 'coarse.db'
        b = sqlite3.connect(self.v1)
        b.executescript(V1_SCHEMA + f'''INSERT INTO roots(entry_id,path) VALUES(1,x'2f72');
            INSERT INTO entries({V1_COLUMNS}) VALUES(1,NULL,x'72',1,21,110,3,0,4);
            INSERT INTO entries({V1_COLUMNS}) VALUES(2,1,x'64',1,10,20,1,0,3);
            INSERT INTO entries({V1_COLUMNS}) VALUES(3,2,x'66',0,9,19,0,0,3);
            INSERT INTO entries({V1_COLUMNS}) VALUES(4,1,x'73',0,1,89,0,0,4);''')
        b.commit(); b.close()
        c = sqlite3.connect(self.coarse)
        # d holds exactly the threshold and stays; its child f and the root's child s fold into one bucket each.
        c.executescript(COARSE_SCHEMA + f'''
            INSERT INTO nodes VALUES({ROOT},NULL,1,10,1,0,20,21,110,2,2,0,0,0,20,10);
            INSERT INTO nodes VALUES({D},{ROOT},1,1,1,0,20,10,20,1,1,0,0,0,20,10);
            INSERT INTO buckets VALUES({ROOT},0,4096,1,89,1,0,0,0,0,20,10);
            INSERT INTO buckets VALUES({D},0,4096,9,19,1,0,0,0,0,20,10);''')
        c.commit(); c.close()

    def mutate(self, sql):
        with sqlite3.connect(self.coarse) as c:
            c.execute(sql)

    def test_valid_sparse_apparent_and_equal_threshold(self):
        result = oracle.verify(self.v1, self.coarse)
        self.assertEqual((result['retained'], result['buckets'], result['max_bucket_entries']), (2, 2, 1))

    def test_rejects_missing_extra_and_wrong_bucket(self):
        for sql in (f"DELETE FROM nodes WHERE path={D}",
                    f"UPDATE buckets SET allocated=allocated+1 WHERE parent={ROOT}",
                    "UPDATE nodes SET kind=0 WHERE parent IS NULL",
                    f"DELETE FROM buckets WHERE parent={ROOT}",
                    f"INSERT INTO buckets VALUES({ROOT},5,6,0,0,0,0,0,0,0,NULL,NULL)",
                    f"UPDATE buckets SET hi=4095 WHERE parent={ROOT}",
                    f"UPDATE buckets SET links=1 WHERE parent={D}",
                    "UPDATE nodes SET links=1 WHERE parent IS NULL",
                    f"UPDATE buckets SET min_scanned_at=NULL WHERE parent={ROOT}",
                    f"UPDATE buckets SET files=0,others=1 WHERE parent={D}"):
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
            c.execute(f"UPDATE nodes SET allocated=11 WHERE path={D}")
            c.execute(f"UPDATE buckets SET allocated=0,apparent=0,files=0,min_scanned_at=NULL,newest_mtime=NULL WHERE parent={D}")
            c.execute(f"INSERT INTO nodes VALUES({F},{D},0,10,19,0,20,10,19,1,0,0,0,0,20,10)")
        result = oracle.verify(self.v1, self.coarse)
        self.assertEqual((result['retained'], result['buckets']), (3, 2))
        with sqlite3.connect(self.coarse) as c:
            c.execute(f"INSERT INTO buckets VALUES({F},0,4096,0,0,0,0,0,0,0,NULL,NULL)")
        with self.assertRaisesRegex(ValueError, 'non-directory'):
            oracle.verify(self.v1, self.coarse)
        with sqlite3.connect(self.coarse) as c:
            c.execute(f"DELETE FROM buckets WHERE parent={F}")
            c.execute(f"DELETE FROM nodes WHERE path={F}")
        with self.assertRaisesRegex(ValueError, 'missing retained node'):
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
        self.mutate(f'UPDATE buckets SET newest_mtime=11 WHERE parent={ROOT}')
        with self.assertRaisesRegex(ValueError, 'newest_mtime'):
            oracle.verify(self.v1, self.coarse)
        self.mutate(f'UPDATE buckets SET newest_mtime=10 WHERE parent={ROOT}')
        self.mutate('UPDATE nodes SET min_scanned_at=21 WHERE parent IS NULL')
        with self.assertRaisesRegex(ValueError, 'min_scanned_at'):
            oracle.verify(self.v1, self.coarse)

    def test_rejects_threshold_and_extra_node(self):
        self.mutate('UPDATE meta SET threshold=9')
        with self.assertRaises(ValueError):
            oracle.verify(self.v1, self.coarse)
        self.mutate('UPDATE meta SET threshold=10')
        self.mutate("INSERT INTO nodes SELECT x'2f7827',path,kind,own_allocated,own_apparent,own_errors,scanned_at,allocated,apparent,files,dirs,others,errors,links,min_scanned_at,newest_mtime FROM nodes WHERE parent IS NULL")
        with self.assertRaisesRegex(ValueError, 'extra coarse nodes'):
            oracle.verify(self.v1, self.coarse)

    def test_rejects_extra_bucket_rows_of_unknown_parents(self):
        self.mutate("INSERT INTO buckets VALUES(x'2f7827',0,4096,0,0,0,0,0,0,0,NULL,NULL)")
        with self.assertRaisesRegex(ValueError, 'extra coarse buckets'):
            oracle.verify(self.v1, self.coarse)

    def test_rejects_other_layout_and_hardlinks(self):
        for column, value in (('format', 1), ('slot_bits', 11), ('bucket_entries', 4095)):
            with self.subTest(column=column):
                self.setUp()
                self.mutate(f'UPDATE meta SET {column}={value}')
                with self.assertRaisesRegex(ValueError, 'layout'):
                    oracle.verify(self.v1, self.coarse)
        self.setUp()
        self.mutate('UPDATE meta SET has_hardlinks=1')
        receipt = {'capacity_bytes': 10000, 'threshold_bytes': 10,
                   'expected_counts': {'entries': 4, 'files': 2, 'dirs': 2, 'others': 0},
                   'expected_controls': {'retained': ['d'], 'omitted': ['s']}}
        with self.assertRaisesRegex(ValueError, 'hardlinks'):
            oracle.verify(self.v1, self.coarse, fixture=receipt)


class ManyEntriesTest(unittest.TestCase):
    """A directory with more than BUCKET_ENTRIES entries stays retained even with no bytes."""
    BIG = 5000

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.v1 = Path(self.tmp.name) / 'v1.db'
        self.coarse = Path(self.tmp.name) / 'coarse.db'
        self.names = [f'f{i:04}'.encode() for i in range(self.BIG)]
        rows = [(1, None, b'r', 1, 0, 35021, self.BIG + 5, 0, self.BIG + 6),
                (2, 1, b'big', 1, 0, 35000, self.BIG, 0, self.BIG + 2)]
        rows += [(3 + i, 2, name, 0, 0, 7, 0, 0, 3 + i) for i, name in enumerate(self.names)]
        first = self.BIG + 3
        rows.append((first, 1, b'small', 1, 0, 21, 3, 0, first + 3))
        rows += [(first + 1 + i, first, f'x{i}'.encode(), 0, 0, 7, 0, 0, first + 1 + i) for i in range(3)]
        b = sqlite3.connect(self.v1)
        b.executescript(V1_SCHEMA + "INSERT INTO roots(entry_id,path) VALUES(1,x'2f72');")
        b.executemany(f'INSERT INTO entries({V1_COLUMNS}) VALUES(?,?,?,?,?,?,?,?,?)', rows)
        b.commit(); b.close()
        c = sqlite3.connect(self.coarse)
        c.executescript(COARSE_SCHEMA)
        big = b'/r/big'
        c.execute('INSERT INTO nodes VALUES(?,NULL,1,0,0,0,20,0,35021,5003,3,0,0,0,20,10)', (b'/r',))
        c.execute('INSERT INTO nodes VALUES(?,?,1,0,0,0,20,0,35000,5000,1,0,0,0,20,10)', (big, b'/r'))
        put_buckets(c, b'/r', {cb.slot_of(b'small'): folded(0, 21, 3, 1)})
        slots = {}
        for name in self.names:  # 5000 names into 4096 slots: collisions must accumulate, not overwrite
            slots.setdefault(cb.slot_of(name), cb.Stats()).add(folded(0, 7, 1))
        put_buckets(c, big, slots)
        c.commit(); c.close()

    def test_entries_rule_retains_the_directory_and_buckets_tile_it(self):
        result = oracle.verify(self.v1, self.coarse)
        self.assertEqual(result['retained'], 2)
        self.assertGreaterEqual(result['buckets'], 3)  # one bucket for the root, at least two for the large directory
        self.assertLessEqual(result['max_bucket_entries'], cb.BUCKET_ENTRIES)

    def test_rejects_a_directory_folded_despite_the_entries_rule(self):
        with sqlite3.connect(self.coarse) as c:
            c.execute("DELETE FROM buckets WHERE parent=?", (b'/r/big',))
            c.execute("DELETE FROM nodes WHERE path=?", (b'/r/big',))
        with self.assertRaisesRegex(ValueError, 'missing retained node'):
            oracle.verify(self.v1, self.coarse)

    def test_rejects_a_shifted_bucket_boundary(self):
        with sqlite3.connect(self.coarse) as c:
            hi = c.execute('SELECT hi FROM buckets WHERE parent=? AND lo=0', (b'/r/big',)).fetchone()[0]
            c.execute('UPDATE buckets SET hi=hi+1 WHERE parent=? AND lo=0', (b'/r/big',))
            c.execute('UPDATE buckets SET lo=lo+1 WHERE parent=? AND lo=?', (b'/r/big', hi))
        with self.assertRaisesRegex(ValueError, 'bucket'):
            oracle.verify(self.v1, self.coarse)

    def test_rejects_entries_moved_between_buckets(self):
        with sqlite3.connect(self.coarse) as c:
            c.execute('UPDATE buckets SET files=files-1 WHERE parent=? AND lo=0', (b'/r/big',))
            c.execute('UPDATE buckets SET files=files+1 WHERE parent=? AND hi=4096', (b'/r/big',))
        with self.assertRaisesRegex(ValueError, 'files'):
            oracle.verify(self.v1, self.coarse)


META_WITH_STAMPS = '''CREATE TABLE meta(singleton INTEGER PRIMARY KEY,root BLOB,threshold INTEGER,ppm INTEGER,capacity INTEGER,generation INTEGER,
    scanned_at INTEGER,has_hardlinks INTEGER,format INTEGER,slot_bits INTEGER,bucket_entries INTEGER);'''
NODES_AND_BUCKETS = COARSE_SCHEMA.split("INSERT INTO meta VALUES(x'2f72',10,1000,10000,0,2,12,4096);")[1]
RP, DP = b'/r', b'/r/d'
SEED_NODES = {RP: 10, DP: 10}
SEED_BUCKETS = {(RP, 0, 2048): 10, (RP, 2048, 4096): 10, (DP, 0, 4096): 10}


def stamped(path, generation, scanned_at, nodes, buckets, root=b'/r'):
    c = sqlite3.connect(path)
    c.executescript(META_WITH_STAMPS + NODES_AND_BUCKETS)
    c.execute('INSERT INTO meta VALUES(1,?,10,1000,10000,?,?,0,2,12,4096)', (root, generation, scanned_at))
    for node, stamp in nodes.items():
        c.execute('INSERT INTO nodes VALUES(?,?,1,0,0,0,?,0,0,0,0,0,0,0,?,NULL)', (node, None if node == RP else RP, stamp, stamp))
    for (parent, lo, hi), stamp in buckets.items():
        c.execute('INSERT INTO buckets VALUES(?,?,?,0,0,0,0,0,0,0,?,NULL)', (parent, lo, hi, stamp))
    c.commit(); c.close()


class RefreshStampTest(unittest.TestCase):
    """The stored scan stamps must show exactly the planned items rewritten, whatever the operation reports."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def check(self, nodes, buckets, expect, generation=3, scanned_at=20, seed=None, seed_root=b'/r'):
        seed_path, after_path = Path(self.tmp.name) / 'seed.db', Path(self.tmp.name) / 'after.db'
        for path in (seed_path, after_path):
            path.unlink(missing_ok=True)
        stamped(seed_path, 2, 10, *(seed or (SEED_NODES, SEED_BUCKETS)), root=seed_root)
        stamped(after_path, generation, scanned_at, nodes, buckets)
        conns = [oracle.connect(p) for p in (after_path, seed_path)]
        for conn in conns:
            conn.row_factory = sqlite3.Row
        return oracle.check_refresh(*conns, expect)

    @staticmethod
    def stamps(nodes=(), buckets=()):
        return ({**SEED_NODES, **{n: 20 for n in nodes}}, {**SEED_BUCKETS, **{b: 20 for b in buckets}})

    def test_full_scan_must_rewrite_everything(self):
        every = self.stamps([RP, DP], SEED_BUCKETS)
        self.assertEqual(self.check(*every, {'scope': 'full'}), {'refresh_checked': True, 'fresh_nodes': 2, 'fresh_buckets': 3})
        with self.assertRaisesRegex(ValueError, 'expected fresh'):
            self.check(*self.stamps(), {'scope': 'full'})  # a no-op publish of the seed
        with self.assertRaisesRegex(ValueError, 'expected fresh'):
            self.check(*self.stamps([RP], SEED_BUCKETS), {'scope': 'full'})

    def test_retained_refresh_rewrites_only_the_subtree(self):
        expect = {'scope': 'retained', 'boundary': DP}
        self.assertEqual(self.check(*self.stamps([DP], [(DP, 0, 4096)]), expect), {'refresh_checked': True, 'fresh_nodes': 1, 'fresh_buckets': 1})
        with self.assertRaisesRegex(ValueError, 'expected same'):
            self.check(*self.stamps([RP, DP], [(DP, 0, 4096)]), expect)  # an ancestor was rewritten too
        with self.assertRaisesRegex(ValueError, 'expected fresh'):
            self.check(*self.stamps([DP], []), expect)  # the subtree's bucket was never rewritten
        with self.assertRaisesRegex(ValueError, 'expected fresh'):
            self.check(*self.stamps([], [(DP, 0, 4096)]), expect)  # the boundary node itself was not
        with self.assertRaisesRegex(ValueError, 'expected same'):
            self.check(*self.stamps([DP], [(DP, 0, 4096), (RP, 0, 2048)]), expect)  # a bucket outside the subtree

    def test_range_refresh_rewrites_the_parent_and_the_planned_window_only(self):
        expect = {'scope': 'range', 'boundary': RP, 'window': (2048, 4096)}
        self.assertEqual(self.check(*self.stamps([RP], [(RP, 2048, 4096)]), expect), {'refresh_checked': True, 'fresh_nodes': 1, 'fresh_buckets': 1})
        with self.assertRaisesRegex(ValueError, 'expected fresh'):
            self.check(*self.stamps([], [(RP, 2048, 4096)]), expect)  # the parent directory was not re-read
        with self.assertRaisesRegex(ValueError, 'expected fresh'):
            self.check(*self.stamps([RP], []), expect)  # the window was not rewritten
        with self.assertRaisesRegex(ValueError, 'expected same'):
            self.check(*self.stamps([RP], [(RP, 2048, 4096), (RP, 0, 2048)]), expect)  # more than the window
        with self.assertRaisesRegex(ValueError, 'expected same'):
            self.check(*self.stamps([RP, DP], [(RP, 2048, 4096)]), expect)  # a retained sibling was revisited

    def test_older_stamps_and_stale_metadata_are_rejected(self):
        expect = {'scope': 'full'}
        nodes, buckets = self.stamps([RP, DP], SEED_BUCKETS)
        with self.assertRaisesRegex(ValueError, 'is older'):
            self.check({**nodes, DP: 5}, buckets, expect)
        with self.assertRaisesRegex(ValueError, 'advance the generation'):
            self.check(nodes, buckets, expect, generation=2)
        with self.assertRaisesRegex(ValueError, 'advance the generation'):
            self.check(nodes, buckets, expect, generation=4)
        with self.assertRaisesRegex(ValueError, 'advance the generation'):
            self.check(nodes, buckets, expect, scanned_at=10)

    def test_the_layout_of_an_unchanged_tree_cannot_change(self):
        expect = {'scope': 'full'}
        nodes, buckets = self.stamps([RP, DP], SEED_BUCKETS)
        with self.assertRaisesRegex(ValueError, 'retained nodes'):
            self.check({**nodes, b'/r/e': 20}, buckets, expect)
        split = {key: value for key, value in buckets.items() if key != (RP, 2048, 4096)}
        split[(RP, 2048, 3000)] = split[(RP, 3000, 4096)] = 20
        with self.assertRaisesRegex(ValueError, 'bucket layout'):
            self.check(nodes, split, expect)
        wider = {key: value for key, value in buckets.items() if key != (RP, 0, 2048)}
        wider[(RP, 0, 2049)] = 20
        with self.assertRaisesRegex(ValueError, 'changed shape'):
            self.check(nodes, wider, expect)

    def test_empty_slot_ranges_carry_no_stamp_and_must_stay_empty(self):
        seeded = ({RP: 10, DP: 10}, {**SEED_BUCKETS, (RP, 0, 2048): None})
        ok = self.check({RP: 20, DP: 20}, {**SEED_BUCKETS, (RP, 0, 2048): None, (RP, 2048, 4096): 20, (DP, 0, 4096): 20}, {'scope': 'full'}, seed=seeded)
        self.assertEqual((ok['fresh_nodes'], ok['fresh_buckets']), (2, 2))
        with self.assertRaisesRegex(ValueError, 'changed shape'):
            self.check({RP: 20, DP: 20}, {key: 20 for key in SEED_BUCKETS}, {'scope': 'full'}, seed=seeded)

    def test_a_seed_of_another_root_is_rejected(self):
        nodes, buckets = self.stamps([RP, DP], SEED_BUCKETS)
        with self.assertRaisesRegex(ValueError, 'another root'):
            self.check(nodes, buckets, {'scope': 'full'}, seed_root=b'/q')

    def test_incomplete_expectations_are_refused(self):
        nodes, buckets = self.stamps([RP, DP], SEED_BUCKETS)
        for expect in ({'scope': 'range', 'boundary': RP}, {'scope': 'retained'}, {'scope': 'partial', 'boundary': RP}):
            with self.assertRaisesRegex(ValueError, 'incomplete refresh expectation'):
                self.check(nodes, buckets, expect)


class CommandLineTest(unittest.TestCase):
    def test_expectation_flags_and_seed_belong_together(self):
        self.assertIsNone(oracle.expectation(oracle.parse(['b', 'c'])))
        with self.assertRaisesRegex(ValueError, 'needs --seed'):
            oracle.expectation(oracle.parse(['b', 'c', '--expect-scope', 'full']))
        with self.assertRaisesRegex(ValueError, 'needs --seed'):
            oracle.expectation(oracle.parse(['b', 'c', '--expect-window', '1', '2']))
        with self.assertRaisesRegex(ValueError, 'needs --expect-scope'):
            oracle.expectation(oracle.parse(['b', 'c', '--seed', 's']))
        args = oracle.parse(['b', 'c', '--seed', 's', '--expect-scope', 'range', '--expect-boundary', '/r', '--expect-window', '4', '9'])
        self.assertEqual(oracle.expectation(args), {'scope': 'range', 'window': [4, 9], 'boundary': b'/r'})

    def test_main_reports_a_rejected_snapshot_as_json_and_a_failing_status(self):
        import contextlib, io, json
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            status = oracle.main(['/nonexistent/baseline.db', '/nonexistent/coarse.db', '--seed', 's'])
        self.assertEqual(status, 1)
        self.assertEqual(json.loads(out.getvalue())['success'], False)


class RefreshThroughVerifyTest(unittest.TestCase):
    """verify() reports the check and refuses a snapshot that equals its seed."""

    def setUp(self):
        OracleTest.setUp(self)  # the small tree: a v1 image and its exact coarse fold, stamped 20
        with sqlite3.connect(self.coarse) as c:
            c.executescript('ALTER TABLE meta ADD COLUMN generation INTEGER DEFAULT 3; ALTER TABLE meta ADD COLUMN scanned_at INTEGER DEFAULT 20;')
        self.seed = Path(self.tmp.name) / 'seed.db'
        self.seed.write_bytes(self.coarse.read_bytes())
        with sqlite3.connect(self.seed) as c:
            c.executescript('UPDATE nodes SET scanned_at=10,min_scanned_at=10; UPDATE buckets SET min_scanned_at=10; '
                            'UPDATE meta SET generation=2,scanned_at=10;')

    def test_reports_the_rewritten_items(self):
        result = oracle.verify(self.v1, self.coarse, seed=self.seed, expect={'scope': 'full'})
        self.assertEqual((result['refresh_checked'], result['fresh_nodes'], result['fresh_buckets']), (True, 2, 2))
        self.assertFalse(oracle.verify(self.v1, self.coarse)['refresh_checked'])

    def test_a_snapshot_equal_to_its_seed_is_not_a_refresh(self):
        with sqlite3.connect(self.coarse) as c:
            c.executescript('UPDATE nodes SET scanned_at=10,min_scanned_at=10; UPDATE buckets SET min_scanned_at=10; UPDATE meta SET scanned_at=20;')
        with self.assertRaisesRegex(ValueError, 'expected fresh'):
            oracle.verify(self.v1, self.coarse, seed=self.seed, expect={'scope': 'full'})


if __name__ == '__main__':
    unittest.main()
