import copy
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).parents[1]))
from cause_outcomes import MIB, evaluate


def rows():
    result = []
    for n in range(1, 5):
        for mode, size in (('idle', 2), ('names', 10), ('stat', 1000), ('scan', 1200)):
            result.append(dict(round=n, mode=mode, group='latin',
                kernel_memory_peak_bytes=(size + 10)*MIB, frozen_post_cli_memory_stat=dict(slab_reclaimable=size*MIB),
                elapsed_seconds=10, peak_rss_kib=20000))
    for n in (1, 2, 3):
        for sampler in ('full', 'reserve'):
            result.append(dict(round=n, mode='scan', group='collector', sampler=sampler,
                kernel_memory_peak_bytes=1200*MIB, elapsed_seconds=10, peak_rss_kib=20000))
    return result


class OutcomeTests(unittest.TestCase):
    def test_support_requires_all_four_primary_and_collector_comparability(self):
        source = rows()
        result = evaluate(source, 'screen')
        self.assertEqual(result['primary_metadata_effect'], 'supported')
        self.assertEqual(result['metadata_scan_comparability'], 'supported')
        source[-1]['elapsed_seconds'] = 20
        result = evaluate(source, 'screen')
        self.assertEqual(result['primary_metadata_effect'], 'supported')
        self.assertEqual(result['metadata_scan_comparability'], 'unresolved')

    def test_one_opposite_round_or_large_control_spread_is_unresolved(self):
        source = rows()
        source[2]['frozen_post_cli_memory_stat']['slab_reclaimable'] = 9*MIB
        self.assertEqual(evaluate(source, 'screen')['primary_metadata_effect'], 'unresolved')
        source = rows()
        source[1]['frozen_post_cli_memory_stat']['slab_reclaimable'] = 1100*MIB
        self.assertEqual(evaluate(source, 'screen')['primary_metadata_effect'], 'unresolved')

    def test_rejects_only_observed_small_material_effect_and_smoke_ineligible(self):
        source = rows()
        for row in source:
            if row.get('mode') == 'stat':
                row['frozen_post_cli_memory_stat']['slab_reclaimable'] = 11*MIB
                row['kernel_memory_peak_bytes'] = 21*MIB
        self.assertEqual(evaluate(source, 'screen')['primary_metadata_effect'], 'rejected')
        self.assertEqual(evaluate(source, 'smoke')['status'], 'smoke-ineligible')


if __name__ == '__main__':
    unittest.main()
