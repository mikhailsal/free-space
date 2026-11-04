from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import cleanup_probe_fixture as fx
import cleanup_probe_plan as plan


def receipt(profile, count):
    """The parts of a fixture receipt the plan reads; the counts come from the fixture builder's own arithmetic."""
    return dict(scale=count, controls={'group': {'files': fx.GROUP_FILES}}, **fx.receipt_shape(profile, count))


class PlanTests(unittest.TestCase):
    def test_frozen_run_counts(self):
        self.assertEqual(plan.expected('smoke'), dict(cases=24, oracles=8, cycles=8, trims=22))
        self.assertEqual(plan.expected('screen'), dict(cases=2 * (4 + 4 * 3 * 2), oracles=24, cycles=24, trims=2 * (1 + 2 + 24)))

    def test_frozen_scenarios_and_arms(self):
        self.assertEqual(plan.ARMS, ('v1', 'probe'))
        self.assertEqual([name for name, _, _ in plan.SCENARIOS], ['full', 'retained', 'omitted', 'wide'])
        self.assertEqual(plan.SCENARIOS[3], ('wide', 'refresh', '/scratch/tree/bulk/g000050'))

    def test_arm_order_is_balanced_per_scenario_over_both_profiles(self):
        for stage, pairs in (('smoke', 1), ('screen', 3)):
            self.assertEqual(plan.STAGES[stage]['pairs'], pairs)
            overall = []
            for s in range(len(plan.SCENARIOS)):
                first = [plan.order(s, p, q)[0] for q in range(len(plan.PROFILES)) for p in range(pairs)]
                overall += first
                self.assertEqual(first.count('v1'), first.count('probe'), (stage, s))
                for q in range(len(plan.PROFILES)):
                    for p in range(pairs):
                        self.assertEqual(sorted(plan.order(s, p, q)), sorted(plan.ARMS))
            self.assertEqual(overall.count('v1'), overall.count('probe'))

    def test_chunk_size_follows_the_fixture_builder(self):
        self.assertEqual(plan.CHUNK_ENTRIES, fx.CHUNK_CHILDREN + 1)
        self.assertEqual(len(plan.chunk_names(1_000_000)), 1000)
        self.assertEqual(plan.chunk_names(100_000)[50], b'g000050')

    def test_wide_bucket_ranges_are_pinned_across_languages(self):
        # Computed with the independent Python reference; buckets.rs pins the same windows.
        self.assertEqual(plan.wide_range(100_000), (804, 971, 4004))
        self.assertEqual(plan.wide_range(1_000_000), (945, 956, 4004))

    def test_probe_scopes_for_both_profiles_of_the_screen_fixture(self):
        for profile in plan.PROFILES:
            fixture = receipt(profile, 1_000_000)
            entries = fixture['expected_counts']['entries']
            self.assertEqual(plan.subtree_sizes(fixture), (74, 100), profile)
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

    def test_v1_scopes_for_both_profiles_of_the_screen_fixture(self):
        for profile in plan.PROFILES:
            fixture = receipt(profile, 1_000_000)
            self.assertEqual(plan.scope('v1', 'full', fixture), dict(entries=fixture['expected_counts']['entries']))
            self.assertEqual([plan.scope('v1', s, fixture) for s in ('retained', 'omitted', 'wide')],
                             [dict(subtree_entries=74), dict(subtree_entries=100), dict(subtree_entries=1001)], profile)

    def test_smoke_fixture_scopes_use_the_smoke_window(self):
        for profile in plan.PROFILES:
            wide = plan.scope('probe', 'wide', receipt(profile, 100_000))
            self.assertEqual(wide['range'], dict(lo=804, hi=971, old_entries=4004, new_entries=4004))
            self.assertEqual(wide['visited'], 8010)

    def test_changed_fixture_contract_is_rejected(self):
        fixture = receipt('files', 100_000)
        fixture['controls']['group']['files'] = 65
        with self.assertRaisesRegex(ValueError, 'subtree contract'):
            plan.subtree_sizes(fixture)


if __name__ == '__main__':
    unittest.main()
