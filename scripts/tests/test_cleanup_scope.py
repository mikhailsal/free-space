from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from cleanup_probe_worker import scope_errors


class ScopeTests(unittest.TestCase):
    def test_refresh_scope(self):
        for expected in ({'subtree_entries':74}, {'subtree_entries':100},
                         {'visited':148, 'boundary':'/scratch/tree/large', 'fallback_full_root':False},
                         {'visited':2000, 'boundary':'/scratch/tree', 'fallback_full_root':True}):
            self.assertEqual(scope_errors(expected,expected), [])
            for field in expected:
                altered=dict(expected)
                altered[field] = None
                self.assertTrue(scope_errors(altered, expected), field)

    def test_full_and_type_mismatch(self):
        self.assertEqual(scope_errors({'entries':1000},{'entries':1000}), [])
        self.assertTrue(scope_errors({'entries':True},{'entries':1}))
        self.assertTrue(scope_errors({'visited':1999},{'visited':2000}))


if __name__=='__main__': unittest.main()
