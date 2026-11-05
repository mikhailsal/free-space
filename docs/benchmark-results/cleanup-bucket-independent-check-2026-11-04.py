"""Independent re-check of the cleanup-bucket screen (2026-11-04), written from the pre-registration's rules.

It does not import or read the frozen analysis script. Two checks:

1. Extraction: every value of the 48 timed cases in the points CSV is re-derived from the raw per-case files
   (time.json and metrics.json) of the unpacked artifact capsule.
2. Verdict: every bar and every prediction is recomputed from the CSV with exact rational arithmetic and compared
   with the frozen analysis JSON, together with the overall success rule.

Usage (from this directory, with the capsule unpacked next to it):
    mkdir capsule && tar -xzf cleanup-bucket-screen-2026-11-04.tar.gz -C capsule
    python3 cleanup-bucket-independent-check-2026-11-04.py --capsule capsule
"""
import argparse
import csv
import json
import statistics
import sys
from fractions import Fraction
from pathlib import Path

HERE = Path(__file__).resolve().parent
KIB = 1024
# noise floors: (relative, absolute); a difference is a change only if it exceeds BOTH, with v1 as the base
FLOOR = {'time': (Fraction(5, 100), Fraction(5, 100)),
         'rss': (Fraction(2, 100), Fraction(1024 * 1024)),
         'disk': (Fraction(5, 100), Fraction(4096))}
NAME = {'D2': 'D2_peak_disk_no_increase', 'D3': 'D3_rss_no_increase', 'D6': 'D6_refresh_no_slowdown', 'D8': 'D8_speed_benefit_15pct',
        'D9': 'D9_rss_benefit_25pct', 'D10': 'D10_disk_benefit_25pct', 'D4': 'D4_full_scan_limit_and_exchange'}
PRED = {'rss_lower': 'rss_lower_than_v1', 'disk4x_under_tenth': 'disk_at_4x_under_a_tenth_of_v1', 'snapshot_under_1MiB': 'snapshot_pair_under_1_mib',
        'faster': 'faster_than_v1', 'tenth_and_0.1s': 'under_a_tenth_of_v1_and_0.1_s', 'tenth_and_1s': 'under_a_tenth_of_v1_and_1_s'}


def exact(text):
    return Fraction(str(text))


def check_extraction(rows, capsule):
    mismatches = []
    for r in rows:
        case = f"{r['profile']}-{r['scenario']}-{r['repeat']}-{r['arm']}"
        base = capsule / 'measurements' / case
        t = json.loads((base / 'time.json').read_text())
        m = json.loads((base / 'metrics.json').read_text())
        if t['exit_code'] != 0 or m['exit_code'] != 0:
            mismatches.append((case, 'non-zero exit status'))
        raw = {'command_elapsed_seconds': t['elapsed_seconds'], 'worker_elapsed_seconds': m['elapsed_seconds'],
               'process_peak_rss_kib': t['peak_rss_kib'], 'cgroup_memory_peak_bytes': int(m['cgroup_after']['memory.peak']),
               'sampled_output_peak_estimate_bytes': m['output_peak_estimate_bytes'],
               'final_allocated_output_bytes': m['final_output']['total']['allocated_bytes']}
        raw['peak_disk_bytes'] = max(raw['sampled_output_peak_estimate_bytes'], raw['final_allocated_output_bytes'])
        mismatches += [(case, k, r[k], v) for k, v in raw.items() if exact(r[k]) != exact(v)]
    print(f'extraction: {len(rows)} rows x 7 fields compared with the raw per-case files, mismatches: {mismatches}')
    return not mismatches


class Cells:
    def __init__(self, rows):
        self.rows = {}
        for r in rows:
            self.rows.setdefault((r['profile'], r['scenario'], r['arm']), {})[int(r['repeat'])] = r

    def series(self, profile, scenario, arm, kind):
        out = []
        for i in (1, 2, 3):
            r = self.rows[(profile, scenario, arm)][i]
            if kind == 'time':
                out.append(exact(r['command_elapsed_seconds']))
            elif kind == 'rss':
                out.append(exact(r['process_peak_rss_kib']) * KIB)
            else:  # the probe is judged at four times its observed peak disk
                out.append(exact(r['peak_disk_bytes']) * (4 if arm == 'probe' else 1))
        return out

    def raw_median(self, profile, scenario, arm, column):
        return statistics.median([exact(self.rows[(profile, scenario, arm)][i][column]) for i in (1, 2, 3)])


def beyond(base, probe, kind, sign):
    """sign +1: probe worse than v1 beyond the noise floors; sign -1: probe better beyond them."""
    rel, ab = FLOOR[kind]
    diff = (probe - base) * sign
    return diff > 0 and diff > ab and diff > rel * base


def no_increase(cells, profile, scenario, kind):
    b, p = cells.series(profile, scenario, 'v1', kind), cells.series(profile, scenario, 'probe', kind)
    return all(not beyond(x, y, kind, 1) for x, y in zip(b, p))


def benefit(cells, profile, scenario, kind, bar):
    b, p = cells.series(profile, scenario, 'v1', kind), cells.series(profile, scenario, 'probe', kind)
    reductions = [(x - y) / x for x, y in zip(b, p)]
    return (all(beyond(x, y, kind, -1) for x, y in zip(b, p)) and all(r >= bar for r in reductions) and statistics.median(reductions) >= bar)


def full_scan(cells, profile):
    b, p = cells.series(profile, 'full', 'v1', 'time'), cells.series(profile, 'full', 'probe', 'time')
    if any((y - x) / x > Fraction(15, 100) for x, y in zip(b, p)):
        return 'failed'
    if not any(beyond(x, y, 'time', 1) for x, y in zip(b, p)):
        return 'passed'
    return 'needs an exchange (not evaluated by this script)'


def judge(cells):
    mine = {}
    for profile in ('files', 'directories'):
        for scenario in ('full', 'retained', 'omitted', 'wide'):
            bars = {'D2': no_increase(cells, profile, scenario, 'disk'), 'D3': no_increase(cells, profile, scenario, 'rss'),
                    'D8': benefit(cells, profile, scenario, 'time', Fraction(15, 100)),
                    'D9': benefit(cells, profile, scenario, 'rss', Fraction(25, 100)),
                    'D10': benefit(cells, profile, scenario, 'disk', Fraction(25, 100))}
            bars = {k: ('passed' if v else 'failed') for k, v in bars.items()}
            if scenario == 'full':
                bars['D4'] = full_scan(cells, profile)
            else:
                bars['D6'] = 'passed' if no_increase(cells, profile, scenario, 'time') else 'failed'
            tp, tv = cells.raw_median(profile, scenario, 'probe', 'command_elapsed_seconds'), cells.raw_median(profile, scenario, 'v1', 'command_elapsed_seconds')
            preds = {'rss_lower': cells.raw_median(profile, scenario, 'probe', 'process_peak_rss_kib') < cells.raw_median(profile, scenario, 'v1', 'process_peak_rss_kib'),
                     'disk4x_under_tenth': 4 * cells.raw_median(profile, scenario, 'probe', 'peak_disk_bytes') < cells.raw_median(profile, scenario, 'v1', 'peak_disk_bytes') / 10,
                     'snapshot_under_1MiB': cells.raw_median(profile, scenario, 'probe', 'final_allocated_output_bytes') < 1024 * 1024}
            if scenario == 'full':
                preds['faster'] = tp < tv
            elif scenario == 'retained':
                preds['tenth_and_0.1s'] = tp < tv / 10 and tp < Fraction(1, 10)
            else:
                preds['tenth_and_1s'] = tp < tv / 10 and tp < 1
            mine[f'{profile}/{scenario}'] = (bars, preds)
    return mine


def check_verdict(rows, analysis):
    mine = judge(Cells(rows))
    disagreements = []
    for cell, (bars, preds) in mine.items():
        for k, v in bars.items():
            if v != analysis['bars'][cell][NAME[k]]:
                disagreements.append((cell, k, v, analysis['bars'][cell][NAME[k]]))
        for k, v in preds.items():
            if v != analysis['predictions']['cells'][cell][PRED[k]]:
                disagreements.append((cell, 'prediction ' + k, v, analysis['predictions']['cells'][cell][PRED[k]]))
    every = [b for bars, _ in mine.values() for b in bars.values()]
    success = (all(b['D2'] == 'passed' and b['D3'] == 'passed' for b, _ in mine.values())
               and all(b['D6'] == 'passed' for b, _ in mine.values() if 'D6' in b)
               and all(b['D4'] == 'passed' for b, _ in mine.values() if 'D4' in b)
               and any(b[k] == 'passed' for b, _ in mine.values() for k in ('D8', 'D9', 'D10')))
    misses = [(cell, k) for cell, (_, preds) in mine.items() for k, v in preds.items() if not v]
    print(f'verdict: {len(every)} bar verdicts and {sum(len(p) for _, p in mine.values())} predictions recomputed, disagreements with the analysis: {disagreements}')
    print(f"verdict: independent overall success {success}, analysis overall success {analysis['overall']['success']}, prediction misses {misses}")
    return not disagreements and success == analysis['overall']['success']


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--capsule', type=Path, required=True, help='the unpacked cleanup-bucket-screen-2026-11-04.tar.gz')
    parser.add_argument('--points', type=Path, default=HERE / 'cleanup-bucket-screen-points-2026-11-04.csv')
    parser.add_argument('--analysis', type=Path, default=HERE / 'cleanup-bucket-screen-analysis-2026-11-04.json')
    args = parser.parse_args()
    rows = list(csv.DictReader(args.points.open()))
    ok = check_extraction(rows, args.capsule)
    ok = check_verdict(rows, json.loads(args.analysis.read_text())) and ok
    print('ALL CHECKS AGREE' if ok else 'DISAGREEMENT')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
