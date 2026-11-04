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

    def test_bucket_range_scope(self):
        wide = {'visited':8010, 'boundary':'/scratch/tree/bulk', 'scope':'range', 'fallback_full_root':False,
                'range':{'lo':945, 'hi':956, 'old_entries':4004, 'new_entries':4004}}
        self.assertEqual(scope_errors(wide, wide), [])
        for field in wide:
            altered=dict(wide)
            altered[field] = None
            self.assertTrue(scope_errors(altered, wide), field)
        for key in wide['range']:
            altered=dict(wide, range=dict(wide['range'], **{key: wide['range'][key] + 1}))
            self.assertTrue(scope_errors(altered, wide), key)
        self.assertTrue(scope_errors(dict(wide, scope='full'), wide))
        self.assertTrue(scope_errors(dict(wide, fallback_full_root=True), wide))
        self.assertTrue(scope_errors({k: v for k, v in wide.items() if k != 'range'}, wide))

    def test_full_scope_expects_no_range(self):
        full = {'visited':2000, 'boundary':'/scratch/tree', 'scope':'full', 'fallback_full_root':False, 'range':None}
        self.assertEqual(scope_errors(full, full), [])
        self.assertTrue(scope_errors(dict(full, range={'lo':0, 'hi':4096, 'old_entries':1, 'new_entries':1}), full))

    def test_full_and_type_mismatch(self):
        self.assertEqual(scope_errors({'entries':1000},{'entries':1000}), [])
        self.assertTrue(scope_errors({'entries':True},{'entries':1}))
        self.assertTrue(scope_errors({'visited':1999},{'visited':2000}))


if __name__=='__main__': unittest.main()
