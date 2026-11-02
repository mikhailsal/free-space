#!/usr/bin/env python3
"""Prospective 1m admission from a completed valid 100k smoke; no dispatch."""
import argparse
import json
import math
from pathlib import Path

from cause_summary import summarize
from cause_validate import load, need


def duration(value):
    need(type(value) in (int, float) and math.isfinite(value) and value >= 0, 'invalid host duration')
    return value


def forecast(root, step_seconds):
    result = summarize(root, require_cleanup=True)
    need(result['valid'] and result['stage'] == 'smoke', 'valid completed smoke required')
    manifest, status = load(root, 'manifest.json'), load(root, 'status.json')
    setup = sum(duration(manifest['setup'][k]) for k in ('fixture_seconds', 'seed_setup_seconds'))
    times = {}
    for row in manifest['cases']:
        receipt = load(root, row['id'] + '/host-timing.json')
        fields = ('copy_and_seed_hash_seconds', 'cache_flush_seconds', 'worker_and_capture_seconds',
                  'observer_seconds', 'case_cleanup_seconds')
        subtotal = sum(duration(receipt[k]) for k in fields)
        total = duration(receipt['total_seconds'])
        need(total + .001 >= subtotal, 'overlapping host duration categories')
        times[row['id']] = total
    host = duration(status['elapsed_seconds'])
    fixed_host = host - setup - sum(times.values())
    need(fixed_host >= -.01, 'host timing exceeds complete host interval')
    # Every case copies/hashes/syncs the snapshot. Scale entire idle case too,
    # including its constant one-second sleep, as a conservative allowance.
    projected = 10 * (setup + max(0, fixed_host)) + 40 * times['smoke-idle']
    projected += 40 * (times['smoke-names'] + times['smoke-stat'])
    projected += 70 * times['smoke-scan'] + 30 * times['smoke-reserve'] + 10 * times['disk-pin']
    fixed_outer = duration(step_seconds) - host
    need(fixed_outer >= 0, 'workflow step shorter than host interval')
    whole = projected + fixed_outer
    return dict(protocol='resource-cause-forecast-v1', stage='screen', admitted=projected * 1.3 < 2100
                and whole * 1.3 < 2700 and projected * 1.3 + fixed_outer + 240 < 2700,
                host_forecast_seconds=projected, host_with_30pct_margin_seconds=projected * 1.3,
                host_cap_seconds=2100, outer_fixed_observed_seconds=fixed_outer,
                whole_step_forecast_seconds=whole, whole_step_with_30pct_margin_seconds=whole * 1.3,
                step_cap_seconds=2700, maximum_job_seconds=3600,
                cleanup_reserve_seconds=240,
                method='100k-to-1m linear scale10; fixed23-case schedule:4 each idle/names/stat,7 normal full scans,3 reserve scans,1 disk pin; observed outer setup retained;30percent margin',
                limitations='Planning projection, not measured screen time or a physical upper bound. Nonlinearity can invalidate it; hard deadlines remain active.')


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('root', type=Path)
    parser.add_argument('--step-seconds', required=True, type=float)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args(argv)
    result = forecast(args.root, args.step_seconds)
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
    print(json.dumps(result))
    return 0 if result['admitted'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
