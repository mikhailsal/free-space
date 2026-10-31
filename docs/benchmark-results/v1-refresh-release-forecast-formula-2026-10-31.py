"""Read-only preregistered musl screening to full-refresh planning estimate.

Usage: python3 FORMULA.py MEASUREMENTS NEW_OUTPUT_JSON CHECKED_OUT_SOURCE_SCRIPTS
Use the frozen benchmark revision for CHECKED_OUT_SOURCE_SCRIPTS.
"""
import json
import math
from pathlib import Path
import sys

def forecast(root, scripts):
    sys.path.insert(0, str(Path(scripts).resolve()))
    from bench_v1_profile import analyze
    root = Path(root)
    profile = analyze(root)
    if not profile['valid_structure'] or not profile['strict_summary']['identical_binaries']:
        raise ValueError('Complete valid identical-binary musl screening is required')
    manifest = json.loads((root / 'manifest.json').read_text())
    if manifest['files'] != 1000000 or manifest['pairs'] != 3:
        raise ValueError('Unexpected screening size or pair count')
    if any(p['build']['target'] != 'x86_64-unknown-linux-musl'
           for p in manifest['provenance'].values()):
        raise ValueError('Release musl target required')
    read = lambda p: json.loads(p.read_text())
    seeds = [read(root.with_name(root.name + '-setup') / f'seed-{i}' / 'metrics.json') for i in (1, 2)]
    if any(x.get('validated') is not True for x in seeds):
        raise ValueError('Validated seed scans required')
    arms = [root / x for x in manifest['expected_arms']]
    metrics = [read(p / 'metrics.json') for p in arms]
    observers = [read(p / 'observer.json') for p in arms]
    copies = [read(p / 'setup.json') for p in arms]
    entry_ratio = 7297076 / manifest['fixture_receipt']['expected_entries']
    file_ratio = 7264383 / manifest['files']
    components = {
        'fixture_seconds': manifest['fixture_receipt']['setup_seconds'] * file_ratio,
        'two_seed_scans_seconds': 2 * max(x['elapsed_seconds'] for x in seeds) * entry_ratio,
        'two_refresh_commands_seconds': 2 * max(x['elapsed_seconds'] for x in metrics) * entry_ratio,
        'two_observers_seconds': 2 * max(x['resources']['elapsed_seconds'] for x in observers) * entry_ratio,
        'two_copies_seconds': 2 * max(x['elapsed_seconds'] for x in copies) * entry_ratio,
        'fixed_setup_calibration_cleanup_allowance_seconds': 450,
    }
    if any(not math.isfinite(x) or x < 0 for x in components.values()):
        raise ValueError('Invalid planning component')
    seconds = math.ceil(sum(components.values()))
    return dict(source_run=manifest['github'], operation='refresh', files=7264383,
                expected_entries=7297076, pairs=1, components=components,
                forecast_seconds=seconds, with_30pct_margin_seconds=math.ceil(seconds * 1.3),
                paired_step_cap_seconds=7200, maximum_aggregate_job_seconds=9000,
                admitted=seconds * 1.3 <= 7200,
                limitations='Linear entry/file projection of worst observed screen arms; '
                'memory pressure, cache and device nonlinearities may exceed it. '
                'This is admission, not promised latency; timeout is invalid evidence. '
                'Build reserves a separate 1500 seconds; aggregate campaign admission is separate.')


if __name__ == '__main__':
    result = forecast(sys.argv[1], sys.argv[3])
    with Path(sys.argv[2]).open('x') as stream:
        json.dump(result, stream, indent=2, allow_nan=False)
        stream.write('\n')
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result['admitted'] else 1)
