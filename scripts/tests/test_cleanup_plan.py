from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import cleanup_probe_plan as plan


def receipt(count):
    """The parts of a fixture receipt the plan reads, shaped like cleanup_probe_fixture.build()."""
    files = count + 99 + 5 + 64 + 1
    dirs = count // 1000 + 7
    return dict(scale=count, bulk_files=count, controls={'group': {'files': 64}},
                expected_counts=dict(files=files, dirs=dirs, others=0, entries=files + dirs))


class PlanTests(unittest.TestCase):
    def test_frozen_run_counts(self):
        self.assertEqual(plan.expected('smoke'), dict(cases=24, oracles=8, cycles=8, trims=22))
        self.assertEqual(plan.expected('screen'), dict(cases=2 * (4 + 4 * 3 * 2), oracles=24, cycles=24, trims=2 * (1 + 2 + 24)))

    def test_frozen_scenarios_and_arms(self):
        self.assertEqual(plan.ARMS, ('v1', 'probe'))
        self.assertEqual([name for name, _, _ in plan.SCENARIOS], ['full', 'retained', 'omitted', 'wide'])
        self.assertEqual(plan.SCENARIOS[3], ('wide', 'refresh', '/scratch/tree/bulk/g000050'))

    def test_arm_order_is_balanced_over_the_screen(self):
        first = [plan.order(s, p)[0] for s in range(len(plan.SCENARIOS)) for p in range(plan.STAGES['screen']['pairs'])]
        self.assertEqual((first.count('v1'), first.count('probe')), (6, 6))
        for s in range(len(plan.SCENARIOS)):
            for p in range(3):
                self.assertEqual(sorted(plan.order(s, p)), sorted(plan.ARMS))

    def test_wide_bucket_ranges_are_pinned_across_languages(self):
        # Computed with the independent Python reference; the Rust probe must report the same window.
        self.assertEqual(plan.wide_range(100_000), (804, 971, 4004))
        self.assertEqual(plan.wide_range(1_000_000), (945, 956, 4004))

    def test_probe_scopes_for_the_screen_fixture(self):
        fixture = receipt(1_000_000)
        entries = fixture['expected_counts']['entries']
        self.assertEqual(plan.scope('probe', 'full', fixture),
                         dict(visited=2 * entries, boundary='/scratch/tree', scope='full', fallback_full_root=False, range=None))
        self.assertEqual(plan.scope('probe', 'retained', fixture)['visited'], 148)
        omitted = plan.scope('probe', 'omitted', fixture)
        self.assertEqual((omitted['visited'], omitted['boundary'], omitted['scope']), (202, '/scratch/tree', 'range'))
        self.assertEqual(omitted['range'], dict(lo=0, hi=4096, old_entries=100, new_entries=100))
        wide = plan.scope('probe', 'wide', fixture)
        self.assertEqual((wide['visited'], wide['boundary'], wide['scope']), (8010, '/scratch/tree/bulk', 'range'))
        self.assertEqual(wide['range'], dict(lo=945, hi=956, old_entries=4004, new_entries=4004))
        self.assertFalse(any(plan.scope('probe', name, fixture)['fallback_full_root'] for name, _, _ in plan.SCENARIOS))

    def test_v1_scopes_for_the_screen_fixture(self):
        fixture = receipt(1_000_000)
        self.assertEqual(plan.scope('v1', 'full', fixture), dict(entries=fixture['expected_counts']['entries']))
        self.assertEqual([plan.scope('v1', s, fixture) for s in ('retained', 'omitted', 'wide')],
                         [dict(subtree_entries=74), dict(subtree_entries=100), dict(subtree_entries=1001)])

    def test_changed_fixture_contract_is_rejected(self):
        fixture = receipt(100_000)
        fixture['controls']['group']['files'] = 65
        with self.assertRaisesRegex(ValueError, 'subtree contract'):
            plan.subtree_sizes(fixture)


if __name__ == '__main__':
    unittest.main()
