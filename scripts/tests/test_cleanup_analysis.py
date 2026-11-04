"""The bars are the experiment's verdict; they are fixed and tested here before any run produces data to judge."""
import contextlib
import io
import json
import sys
import tempfile
import unittest
from fractions import Fraction
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import cleanup_probe_analysis as an
import cleanup_probe_fixture as fx
import cleanup_probe_plan as plan

MIB = 1024 ** 2


def row(time, rss_mib, disk, memory=1 << 30, flagged=False):
    flags = {name: False for name in an.FLAG_NAMES}
    flags['reclaim'] = flagged
    return {'command_elapsed_seconds': time, 'process_peak_rss_kib': rss_mib * 1024, 'cgroup_memory_peak_bytes': memory,
            'peak_disk_bytes': disk, 'flags': flags}


V1 = [row(10, 40, 100 * MIB)] * 3


def judge(probe, scenario='retained'):
    return an.judge_group({'v1': V1, 'probe': probe}, scenario)


def receipt(profile, count):
    return dict(scale=count, controls={'group': {'files': fx.GROUP_FILES}}, **fx.receipt_shape(profile, count))


class FrozenConstantsTest(unittest.TestCase):
    def test_the_numbers_are_those_of_the_pre_registration(self):
        bars = (an.BAR['speed'], an.BAR['rss'], an.BAR['disk'], an.BAR['full_scan_slowdown'])
        self.assertEqual(bars, (Fraction(15, 100), Fraction(25, 100), Fraction(25, 100), Fraction(15, 100)))
        self.assertEqual(an.BAR['ram_exchange_bytes'], 256 * MIB)
        self.assertEqual(an.FLOORS, {'time': (Fraction(5, 100), Fraction(5, 100)), 'memory': (Fraction(2, 100), MIB),
                                     'disk': (Fraction(5, 100), 4096)})
        self.assertEqual(an.CELLS, tuple(f'{p}/{s}' for p in ('files', 'directories') for s in ('full', 'retained', 'omitted', 'wide')))
        self.assertEqual(an.PROBE_DISK_FACTOR, 4)
        self.assertEqual((an.PROFILES, an.SCENARIOS, an.ARMS), (plan.PROFILES, tuple(s for s, _, _ in plan.SCENARIOS), plan.ARMS))


class NoiseTest(unittest.TestCase):
    def test_a_difference_is_noise_unless_it_exceeds_both_floors(self):
        self.assertEqual(an.direction(0.04, 0.09, 'time'), 0)    # 125% but only 0.05 s
        self.assertEqual(an.direction(10, 10.4, 'time'), 0)      # 0.4 s but only 4%
        self.assertEqual(an.direction(10, 11, 'time'), 1)
        self.assertEqual(an.direction(10, 8, 'time'), -1)
        self.assertEqual(an.direction(40 * MIB, 40.5 * MIB, 'rss'), 0)
        self.assertEqual(an.direction(40 * MIB, 42 * MIB, 'rss'), 1)
        self.assertEqual(an.direction(100 * MIB, 100 * MIB + 2048, 'disk'), 0)
        self.assertEqual(an.direction(0, 5000, 'disk'), 1)
        self.assertEqual(an.direction(0, 0, 'time'), 0)

    def test_a_difference_exactly_at_a_floor_is_noise_whatever_the_float_rounding(self):
        # 1.05 - 1.00 is 0.05000000000000004 in binary floating point, but as written it is exactly the allowance.
        self.assertEqual(an.direction(1.00, 1.05, 'time'), 0)
        self.assertEqual(an.direction(1.00, 1.06, 'time'), 1)
        self.assertEqual(an.direction(0.07, 0.02, 'time'), 0)    # the same 0.05 s in the other direction
        self.assertEqual(an.direction(0.07, 0.01, 'time'), -1)
        self.assertEqual(an.direction(100 * MIB, 105 * MIB, 'disk'), 0)   # exactly 5%
        self.assertEqual(an.direction(100 * MIB, 105 * MIB + 1, 'disk'), 1)

    def test_ties_are_decided_on_the_numbers_as_written(self):
        self.assertEqual(an.change(3.0, 3.45), Fraction(15, 100))
        self.assertEqual(an.change(0.0, 1.0), None)
        self.assertEqual(an.exact(0.07), Fraction(7, 100))
        self.assertEqual(an.exact(7), Fraction(7))


class NonInferiorityTest(unittest.TestCase):
    def test_a_faster_smaller_probe_passes_everything(self):
        out = judge([row(8, 10, 1 * MIB)] * 3, 'full')
        for bar in ('D2_peak_disk_no_increase', 'D3_rss_no_increase', 'D8_speed_benefit_15pct', 'D9_rss_benefit_25pct',
                    'D10_disk_benefit_25pct', 'D4_full_scan_limit_and_exchange'):
            self.assertEqual(out[bar], 'passed', bar)

    def test_one_repeat_above_the_floor_fails_even_when_the_median_is_fine(self):
        probe = [row(10, 40, 100 * MIB), row(10, 40, 100 * MIB), row(10, 48, 100 * MIB)]
        self.assertEqual(judge(probe)['D3_rss_no_increase'], 'failed')

    def test_tiny_differences_are_noise_and_pass(self):
        out = judge([row(10.02, 40.0, 1 * MIB)] * 3)  # 0.02 s slower and the same RSS, with a small snapshot
        self.assertEqual((out['D6_refresh_no_slowdown'], out['D3_rss_no_increase'], out['D2_peak_disk_no_increase']), ('passed',) * 3)

    def test_refresh_slowdown_fails_the_refresh_bar_only_when_real(self):
        self.assertEqual(judge([row(10.3, 40, 100 * MIB)] * 3)['D6_refresh_no_slowdown'], 'passed')   # 3%: noise
        self.assertEqual(judge([row(11, 40, 100 * MIB)] * 3)['D6_refresh_no_slowdown'], 'failed')
        self.assertNotIn('D4_full_scan_limit_and_exchange', judge(V1))
        self.assertNotIn('D6_refresh_no_slowdown', judge(V1, 'full'))

    def test_a_refresh_exactly_at_the_allowance_is_not_a_slowdown(self):
        v1 = [row(1.00, 40, 100 * MIB)] * 3
        at_floor = an.judge_group({'v1': v1, 'probe': [row(1.05, 40, 1 * MIB)] * 3}, 'retained')
        self.assertEqual(at_floor['D6_refresh_no_slowdown'], 'passed')
        beyond = an.judge_group({'v1': v1, 'probe': [row(1.06, 40, 1 * MIB)] * 3}, 'retained')
        self.assertEqual(beyond['D6_refresh_no_slowdown'], 'failed')

    def test_the_probe_is_judged_at_four_times_its_observed_disk(self):
        # Observed 30 MiB against 100 MiB looks lower, but its 4x transient bound is 120 MiB: not shown to be no larger.
        self.assertEqual(judge([row(10, 40, 30 * MIB)] * 3)['D2_peak_disk_no_increase'], 'failed')
        out = judge([row(10, 40, 10 * MIB)] * 3)
        self.assertEqual((out['D2_peak_disk_no_increase'], out['D10_disk_benefit_25pct']), ('passed', 'passed'))
        # 20 MiB observed is 80 MiB judged: only -20%, short of the 25% disk bar although observed is -80%.
        out = judge([row(10, 40, 20 * MIB)] * 3)
        self.assertEqual((out['D2_peak_disk_no_increase'], out['D10_disk_benefit_25pct']), ('passed', 'failed'))
        # v1 is never scaled: a probe equal to v1 at 1x is 4x judged.
        self.assertEqual(an.metric_value(row(1, 1, 100), 'disk', 'v1'), 100)
        self.assertEqual(an.metric_value(row(1, 1, 100), 'disk', 'probe'), 400)


class BenefitTest(unittest.TestCase):
    def test_a_benefit_needs_every_repeat_and_the_median(self):
        self.assertEqual(judge([row(8, 40, 100 * MIB)] * 3)['D8_speed_benefit_15pct'], 'passed')
        mixed = [row(8, 40, 100 * MIB), row(8, 40, 100 * MIB), row(9.2, 40, 100 * MIB)]
        self.assertEqual(judge(mixed)['D8_speed_benefit_15pct'], 'failed')
        self.assertEqual(judge([row(8.6, 40, 100 * MIB)] * 3)['D8_speed_benefit_15pct'], 'failed')  # -14%: short of 15%

    def test_a_benefit_must_also_clear_the_noise_floor(self):
        v1 = [row(0.1, 40, 100 * MIB)] * 3
        probe = [row(0.05, 40, 100 * MIB)] * 3   # -50% but only 0.05 s
        self.assertEqual(an.judge_group({'v1': v1, 'probe': probe}, 'retained')['D8_speed_benefit_15pct'], 'failed')


class FlagTest(unittest.TestCase):
    def test_flags_make_memory_bars_inconclusive_and_leave_time_and_disk_alone(self):
        out = judge([row(8, 10, 1 * MIB, flagged=True)] * 3)
        self.assertEqual((out['D3_rss_no_increase'], out['D9_rss_benefit_25pct']), ('inconclusive', 'inconclusive'))
        self.assertEqual((out['D2_peak_disk_no_increase'], out['D8_speed_benefit_15pct'], out['D10_disk_benefit_25pct']), ('passed',) * 3)

    def test_a_flag_in_the_v1_arm_counts_too(self):
        out = an.judge_group({'v1': [row(10, 40, 100 * MIB, flagged=True)] * 3, 'probe': [row(8, 10, 1 * MIB)] * 3}, 'retained')
        self.assertEqual(out['D3_rss_no_increase'], 'inconclusive')


class FullScanTest(unittest.TestCase):
    def verdict(self, probe):
        return judge(probe, 'full')['D4_full_scan_limit_and_exchange']

    def test_slowdown_up_to_fifteen_percent_needs_an_exchange(self):
        self.assertEqual(self.verdict([row(11, 10, 1 * MIB)] * 3), 'passed')      # +10% for -99% disk
        self.assertEqual(self.verdict([row(11, 40, 100 * MIB)] * 3), 'failed')    # +10% for nothing
        self.assertEqual(self.verdict([row(12, 10, 1 * MIB)] * 3), 'failed')      # +20% is never allowed

    def test_the_limit_is_judged_on_every_repeat_without_a_floor(self):
        self.assertEqual(self.verdict([row(10, 40, 100 * MIB), row(10, 40, 100 * MIB), row(11.6, 10, 1 * MIB)]), 'failed')
        self.assertEqual(self.verdict([row(10.04, 40, 100 * MIB)] * 3), 'passed')  # below the noise floor: no exchange needed

    def test_exactly_fifteen_percent_is_within_the_limit(self):
        v1 = [row(3.0, 40, 100 * MIB)] * 3    # 3.45 / 3.0 is exactly +15%, although it is 0.15000000000000008 in floats
        within = an.judge_group({'v1': v1, 'probe': [row(3.45, 10, 1 * MIB)] * 3}, 'full')
        self.assertEqual(within['D4_full_scan_limit_and_exchange'], 'passed')   # +15% for -96% disk
        beyond = an.judge_group({'v1': v1, 'probe': [row(3.46, 10, 1 * MIB)] * 3}, 'full')
        self.assertEqual(beyond['D4_full_scan_limit_and_exchange'], 'failed')   # +15.3%

    def test_the_exchange_is_sized_by_the_worst_repeat(self):
        # Slowdowns of +6%, +6% and +12% need a 24% gain in every repeat; a 20% gain in one repeat is not enough
        # even though twice the median slowdown (12%) would be.
        times = [row(10.6, 40, 100 * MIB), row(10.6, 40, 100 * MIB), row(11.2, 40, 100 * MIB)]
        short = [row(t['command_elapsed_seconds'], 40, d) for t, d in zip(times, (10 * MIB, 10 * MIB, 20 * MIB))]
        self.assertEqual(self.verdict(short), 'failed')
        enough = [row(t['command_elapsed_seconds'], 40, d) for t, d in zip(times, (10 * MIB, 10 * MIB, 10 * MIB))]
        self.assertEqual(self.verdict(enough), 'passed')

    def test_a_memory_exchange_also_needs_256_mib_absolute(self):
        self.assertEqual(self.verdict([row(11, 10, 100 * MIB)] * 3), 'failed')   # RSS -75% but only 30 MiB
        big = [row(10, 600, 100 * MIB)] * 3                                      # a v1 whose RSS leaves room for a real saving
        for rss, expected in ((300, 'passed'), (344, 'passed'), (345, 'failed')):  # saves 300, exactly 256 and 255 MiB; all above 2x
            probe = [row(11, rss, 100 * MIB)] * 3
            self.assertEqual(an.judge_group({'v1': big, 'probe': probe}, 'full')['D4_full_scan_limit_and_exchange'], expected, rss)

    def test_total_cgroup_memory_never_buys_a_slowdown(self):
        # Page cache and slab count in the cgroup peak and are reclaimable: -768 MiB there is not a RAM saving.
        self.assertEqual(self.verdict([row(11, 40, 100 * MIB, memory=1 << 28)] * 3), 'failed')

    def test_under_a_memory_flag_only_a_disk_exchange_can_pass(self):
        self.assertEqual(self.verdict([row(11, 10, 100 * MIB, memory=1 << 28, flagged=True)] * 3), 'inconclusive')
        self.assertEqual(self.verdict([row(11, 10, 1 * MIB, flagged=True)] * 3), 'passed')
        self.assertEqual(self.verdict([row(11, 40, 100 * MIB, flagged=True)] * 3), 'inconclusive')
        self.assertEqual(self.verdict([row(12, 10, 1 * MIB, flagged=True)] * 3), 'failed')


class OverallTest(unittest.TestCase):
    def groups(self, probe_full, probe_refresh):
        """All eight cells: both profiles' full scans judged on `probe_full`, every refresh scenario on `probe_refresh`."""
        return {cell: judge(probe_full if cell.endswith('/full') else probe_refresh, cell.split('/')[1]) for cell in an.CELLS}

    def test_success_needs_every_gate_and_one_confirmed_benefit(self):
        good = self.groups([row(8, 10, 1 * MIB)] * 3, [row(8, 10, 1 * MIB)] * 3)
        result = an.overall(good)
        self.assertTrue(result['success'])
        self.assertIn('files/full:D8_speed_benefit_15pct', result['confirmed_benefits'])
        self.assertIn('directories/wide:D10_disk_benefit_25pct', result['confirmed_benefits'])
        self.assertIn('D3 minimum tested successful memory budget (not measured)', result['open_by_design'])

    def test_an_incomplete_set_of_cells_is_refused(self):
        good = self.groups([row(8, 10, 1 * MIB)] * 3, [row(8, 10, 1 * MIB)] * 3)
        for cell in an.CELLS:
            with self.assertRaisesRegex(ValueError, 'every profile and scenario', msg=cell):
                an.overall({k: v for k, v in good.items() if k != cell})
        with self.assertRaisesRegex(ValueError, 'every profile and scenario'):
            an.overall({**good, 'files/extra': good['files/full']})

    def test_one_failed_cell_fails_the_gate_and_the_experiment(self):
        bad = self.groups([row(8, 10, 1 * MIB)] * 3, [row(30, 40, 100 * MIB)] * 3)
        result = an.overall(bad)
        self.assertFalse(result['success'])
        self.assertEqual(result['non_inferiority']['D6_refresh_no_slowdown'], 'failed')

    def test_no_benefit_is_not_success(self):
        # 24 MiB observed is 96 MiB judged: no larger than v1's 100 MiB, yet no 25% gain. Time and RSS equal v1.
        same = self.groups([row(10, 40, 24 * MIB)] * 3, [row(10, 40, 24 * MIB)] * 3)
        result = an.overall(same)
        self.assertEqual(set(result['non_inferiority'].values()), {'passed'})   # every gate holds, so only the missing benefit remains
        self.assertEqual(result['confirmed_benefits'], [])
        self.assertFalse(result['success'])

    def test_inconclusive_is_not_a_pass(self):
        # The flagged full-scan group: +10% time, a 4% judged disk gain (no exchange) and memory that cannot be judged.
        flagged = judge([row(11, 10, 24 * MIB, memory=1 << 28, flagged=True)] * 3, 'full')
        self.assertEqual(flagged['D4_full_scan_limit_and_exchange'], 'inconclusive')
        # Every other cell is a clear win that passes every gate, so only the flagged cell's inconclusive gates stand in the way.
        groups = self.groups([row(8, 10, 1 * MIB)] * 3, [row(8, 10, 1 * MIB)] * 3)
        groups['files/full'] = flagged
        result = an.overall(groups)
        self.assertTrue(result['confirmed_benefits'])
        # The flagged cell's inconclusive RSS benefit is not a confirmed benefit although every other cell's is.
        self.assertNotIn('files/full:D9_rss_benefit_25pct', result['confirmed_benefits'])
        self.assertIn('directories/full:D9_rss_benefit_25pct', result['confirmed_benefits'])
        gates = result['non_inferiority']
        self.assertEqual((gates['D2_peak_disk_no_increase'], gates['D6_refresh_no_slowdown']), ('passed', 'passed'))
        self.assertEqual((gates['D3_rss_no_increase'], gates['D4_full_scan_limit_and_exchange']), ('inconclusive', 'inconclusive'))
        self.assertFalse(result['success'])


class GateStatusTest(unittest.TestCase):
    def test_a_gate_passes_only_when_every_cell_passed(self):
        self.assertEqual(an.gate_status(['passed'] * 3), 'passed')
        self.assertEqual(an.gate_status(['passed', 'inconclusive']), 'inconclusive')
        self.assertEqual(an.gate_status(['inconclusive', 'failed', 'passed']), 'failed')
        self.assertEqual(an.gate_status([]), 'inconclusive')   # a gate nobody reported is not passed


class PairingTest(unittest.TestCase):
    def test_unequal_or_empty_repeats_cannot_be_judged(self):
        for v1, probe in ((V1, V1[:2]), (V1[:2], V1), ([], [])):
            with self.assertRaisesRegex(ValueError, 'unpaired repeats', msg=(len(v1), len(probe))):
                an.judge_group({'v1': v1, 'probe': probe}, 'retained')


class ReportTest(unittest.TestCase):
    def test_the_smoke_prints_no_verdicts(self):
        groups = {'files/full': {'D2_peak_disk_no_increase': 'passed'}}
        self.assertEqual(an.report_bars(groups, True), groups)
        self.assertEqual(an.report_bars(groups, False), {'files/full': 'not judged: the smoke has one pair per cell'})

    def test_judged_and_observed_changes_are_reported_apart(self):
        out = judge([row(10, 40, 10 * MIB)] * 3)   # the probe's 10 MiB observed disk against v1's 100 MiB
        self.assertAlmostEqual(out['observed_median_change']['disk'], -0.9)
        self.assertAlmostEqual(out['judged_median_change']['disk'], -0.6)   # at the 4x factor the bars use
        self.assertEqual(out['judged_median_change']['rss'], out['observed_median_change']['rss'])
        self.assertNotIn('median_change', out)
        json.dumps(out, allow_nan=False)   # reportable as it is: no exact fractions leak into the output


def medians(time, rss_mib, disk, final=64 * 1024):
    return {'command_elapsed_seconds': time, 'process_peak_rss_kib': rss_mib * 1024, 'peak_disk_bytes': disk,
            'final_allocated_output_bytes': final}


class PredictionTest(unittest.TestCase):
    """The predictions are scored on the medians of the same run, beside the verdict and never inside it."""

    def tables(self, override=None):
        wins = {'full': 6.0, 'retained': 0.03, 'omitted': 0.05, 'wide': 0.4}
        out = {cell: {'v1': medians(10.0, 40, 200 * MIB, 200 * MIB), 'probe': medians(wins[cell.split('/')[1]], 10, 1 * MIB)}
               for cell in an.CELLS}
        for cell, probe in (override or {}).items():
            out[cell]['probe'] = {**out[cell]['probe'], **probe}
        return out

    def missed(self, override):
        scored = an.predictions(self.tables(override))
        return sorted(name for cell, checks in scored['cells'].items() for name, hit in checks.items() if not hit), scored['all_hit']

    def test_a_clear_win_hits_every_prediction(self):
        scored = an.predictions(self.tables())
        self.assertTrue(scored['all_hit'])
        self.assertEqual(sorted(scored['cells']), sorted(an.CELLS))
        self.assertEqual(sorted(scored['cells']['files/full']), ['disk_at_4x_under_a_tenth_of_v1', 'faster_than_v1', 'rss_lower_than_v1',
                                                                  'snapshot_pair_under_1_mib'])

    def test_each_prediction_misses_exactly_at_its_boundary(self):
        self.assertEqual(self.missed({'files/full': {'command_elapsed_seconds': 10.0}}), (['faster_than_v1'], False))
        self.assertEqual(self.missed({'files/retained': {'command_elapsed_seconds': 0.0999}})[0], [])
        self.assertEqual(self.missed({'files/retained': {'command_elapsed_seconds': 0.1}})[0], ['under_a_tenth_of_v1_and_0.1_s'])
        self.assertEqual(self.missed({'files/omitted': {'command_elapsed_seconds': 0.99}})[0], [])
        self.assertEqual(self.missed({'files/omitted': {'command_elapsed_seconds': 1.0}})[0], ['under_a_tenth_of_v1_and_1_s'])
        self.assertEqual(self.missed({'directories/wide': {'process_peak_rss_kib': 40 * 1024}})[0], ['rss_lower_than_v1'])
        self.assertEqual(self.missed({'files/wide': {'peak_disk_bytes': 5 * MIB}})[0], ['disk_at_4x_under_a_tenth_of_v1'])   # 4x is 20 MiB of 200
        self.assertEqual(self.missed({'files/wide': {'peak_disk_bytes': 5 * MIB - 1}})[0], [])
        self.assertEqual(self.missed({'files/full': {'final_allocated_output_bytes': MIB}})[0], ['snapshot_pair_under_1_mib'])
        self.assertEqual(self.missed({'files/full': {'final_allocated_output_bytes': MIB - 1}})[0], [])

    def hit(self, cell, v1_time, probe_time):
        tables = self.tables()
        tables[cell]['v1'] = medians(v1_time, 40, 200 * MIB, 200 * MIB)
        tables[cell]['probe'] = medians(probe_time, 10, 1 * MIB)
        (name,) = (n for n in an.predictions(tables)['cells'][cell] if n.startswith('under_a_tenth'))
        return an.predictions(tables)['cells'][cell][name]

    def test_the_relative_and_the_absolute_bounds_each_bind(self):
        # omitted and wide: under a tenth of v1 AND under 1 s
        self.assertFalse(self.hit('files/wide', 5.0, 0.6))     # under 1 s but not under a tenth of 5 s
        self.assertFalse(self.hit('files/wide', 20.0, 1.5))    # under a tenth of 20 s but not under 1 s
        self.assertTrue(self.hit('files/wide', 20.0, 0.9))
        self.assertFalse(self.hit('files/wide', 5.0, 0.5))     # exactly a tenth is not under
        self.assertTrue(self.hit('files/wide', 5.0, 0.49))
        # retained: under a tenth of v1 AND under 0.1 s; exactly a tenth is not under
        self.assertFalse(self.hit('files/retained', 0.5, 0.05))
        self.assertTrue(self.hit('files/retained', 0.5, 0.049))
        self.assertFalse(self.hit('files/retained', 20.0, 0.11))   # a tenth of 20 s is 2 s, but 0.11 s is over 0.1 s


class ExpectationsAgreeWithThePlanTest(unittest.TestCase):
    """The analysis derives what a run must show by hand; the plan computes it. They must agree for both profiles and stages."""

    def test_probe_scopes(self):
        for profile in plan.PROFILES:
            for stage, count in (('smoke', 100_000), ('screen', 1_000_000)):
                fixture = receipt(profile, count)
                entries = fixture['expected_counts']['entries']
                for scenario in an.SCENARIOS:
                    self.assertEqual(an.expected_probe(scenario, entries, stage), plan.scope('probe', scenario, fixture), (profile, stage, scenario))

    def test_fixture_counts_follow_the_builder(self):
        for profile in plan.PROFILES:
            for count in (100_000, 1_000_000):
                files = (count if profile == 'files' else count // 2) + 169
                dirs = (0 if profile == 'files' else count // 2) + count // 1000 + 7
                self.assertEqual(fx.receipt_shape(profile, count)['expected_counts'], {'entries': files + dirs, 'files': files, 'dirs': dirs, 'others': 0})

    def test_v1_refresh_sizes(self):
        fixture = receipt('files', 1_000_000)
        self.assertEqual([plan.scope('v1', s, fixture) for s in ('retained', 'omitted', 'wide')],
                         [{'subtree_entries': 74}, {'subtree_entries': 100}, {'subtree_entries': 1001}])


class PlumbingTest(unittest.TestCase):
    def test_arm_rows_must_match_the_plan_and_the_oracle(self):
        fixture = receipt('files', 1_000_000)
        entries = fixture['expected_counts']['entries']
        base = {'visited': 8010, 'boundary': '/scratch/tree/bulk', 'scope': 'range', 'fallback_full_root': False,
                'reported_range': dict(lo=945, hi=956, old_entries=4004, new_entries=4004), 'reported_retained_nodes': 8, 'reported_buckets': 9,
                'reported_entries': None, 'reported_subtree_entries': None}
        oracle = {'retained': 8, 'buckets': 9}
        an.check_arm_row(base, 'probe', 'wide', fixture, 'screen', 'x', oracle)
        for change in ({'visited': 8011}, {'scope': 'full'}, {'fallback_full_root': True}, {'boundary': '/scratch/tree'},
                       {'reported_range': dict(lo=0, hi=4096, old_entries=4004, new_entries=4004)},
                       {'reported_retained_nodes': 9}, {'reported_buckets': 8}):
            with self.assertRaises(ValueError, msg=change):
                an.check_arm_row({**base, **change}, 'probe', 'wide', fixture, 'screen', 'x', oracle)
        with self.assertRaises(ValueError):
            an.check_arm_row(base, 'probe', 'wide', fixture, 'smoke', 'x', oracle)  # the smoke window is a different one
        v1 = {'reported_entries': entries, 'reported_subtree_entries': 1001}
        an.check_arm_row(v1, 'v1', 'wide', fixture, 'screen', 'x', None)
        with self.assertRaises(ValueError):
            an.check_arm_row({**v1, 'reported_subtree_entries': 100}, 'v1', 'wide', fixture, 'screen', 'x', None)

    def test_a_v1_refresh_is_checked_by_its_subtree_size_not_by_a_total_it_need_not_report(self):
        fixture = receipt('files', 1_000_000)
        entries = fixture['expected_counts']['entries']
        no_total = {'reported_entries': None, 'reported_subtree_entries': 1001}
        an.check_arm_row(no_total, 'v1', 'wide', fixture, 'screen', 'x', None)
        with self.assertRaises(ValueError):
            an.check_arm_row({**no_total, 'reported_subtree_entries': None}, 'v1', 'wide', fixture, 'screen', 'x', None)
        with self.assertRaises(ValueError):   # a full scan must report the whole tree
            an.check_arm_row({'reported_entries': None, 'reported_subtree_entries': None}, 'v1', 'full', fixture, 'screen', 'x', None)
        an.check_arm_row({'reported_entries': entries, 'reported_subtree_entries': None}, 'v1', 'full', fixture, 'screen', 'x', None)

    def test_the_oracle_must_prove_what_was_rewritten(self):
        fixture = receipt('files', 100_000)
        resource = {'success': True, 'mem_total_bytes': 16 << 30, 'mem_available_bytes': 14 << 30, 'cpu_count': 4,
                    'writer_ceiling_bytes': 8 << 30, 'host_overhead_bytes': 2 << 30}
        fixture = {**fixture, 'threshold_bytes': 7}
        proof = {'success': True, 'v2_checked': False, 'fixture_checked': True, 'visited': fixture['expected_counts']['entries'],
                 'retained': 8, 'buckets': 9, 'threshold': 7, 'max_bucket_entries': 4004, 'refresh_checked': True,
                 'fresh_nodes': 1, 'fresh_buckets': 1}
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            (base / 'o.host-resource.json').write_text(json.dumps(resource))

            def check(output):
                (base / 'o.json').write_text(json.dumps(output))
                return an.check_oracle(base, 'o', fixture)

            self.assertEqual(check(proof)['fresh_nodes'], 1)
            for broken in ({'refresh_checked': False}, {'fresh_nodes': 0, 'fresh_buckets': 0}, {'v2_checked': True},
                           {'fixture_checked': False}, {'success': False}, {'visited': 5}, {'threshold': 8}):
                with self.assertRaises(ValueError, msg=broken):
                    check({**proof, **broken})

    def test_invalid_input_exits_nonzero_with_a_json_reason(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            status = an.main(['/nonexistent/artifacts'])
        self.assertEqual(status, 1)
        self.assertEqual(json.loads(out.getvalue())['valid'], False)


if __name__ == '__main__':
    unittest.main()
