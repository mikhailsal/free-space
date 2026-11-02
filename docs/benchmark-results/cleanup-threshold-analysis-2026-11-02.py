#!/usr/bin/env python3
"""Analyze downloaded cleanup benchmark artifacts; print JSON, exit nonzero if invalid.

Usage: python3 freespace_cleanup_analyze.py ARTIFACT_ROOT
ARTIFACT_ROOT contains measurements/manifest.json and runner-cleanup.json;
the measurements directory itself is also accepted when its parent has cleanup.
"""
import argparse
import json
import re
import math
import statistics
import sys
from pathlib import Path

PROFILES = ('files', 'directories')
SCENARIOS = ('full', 'retained', 'omitted')
ARMS = ('v1', 'v2', 'probe')
METRICS = ('cgroup_memory_peak_bytes', 'process_peak_rss_kib', 'worker_elapsed_seconds',
           'command_elapsed_seconds',
           'sampled_output_peak_estimate_bytes', 'final_allocated_output_bytes')


def read(path):
    with path.open() as f:
        return json.load(f)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def counters(raw):
    result = {}
    for line in raw.splitlines():
        parts = line.split()
        if len(parts) == 2:
            result[parts[0]] = int(parts[1])
    return result


def number(value, label):
    require(isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0, 'invalid ' + label)
    return value


def host_resource(receipt):
    require(receipt.get('success') is True, 'failed host resource receipt')
    for key in ('mem_total_bytes', 'mem_available_bytes'):
        require(type(receipt.get(key)) is int and receipt[key] >= 10 * 1024**3, 'insufficient host memory: ' + key)
    require(receipt['mem_available_bytes'] <= receipt['mem_total_bytes'], 'impossible host available memory')
    require(type(receipt.get('cpu_count')) is int and receipt['cpu_count'] >= 2, 'insufficient host CPUs')
    require(receipt.get('writer_ceiling_bytes') == 8 * 1024**3 and receipt.get('host_overhead_bytes') == 2 * 1024**3, 'wrong host headroom policy')
    return receipt


def point(base, case, memory_mib=4096):
    ident = case['id']
    m = read(base / ident / 'metrics.json')
    state = read(base / ident / 'docker-state.json')
    report = m['report']
    host = host_resource(read(base / (ident + '.host-resource.json'))) if memory_mib == 8192 else None
    require(m['id'] == ident and m['validated'] is True and case['validated'] is True, ident + ': invalid worker/manifest')
    require(m['exit_code'] == 0 and m['time']['exit_code'] == 0 and not m['sample_errors'], ident + ': worker failure')
    require(m['scope_errors'] == [] and m['expected_scope'], ident + ': unvalidated work scope')
    for key, wanted in m['expected_scope'].items():
        require(type(report.get(key)) is type(wanted) and report[key] == wanted, ident + ': work scope mismatch: ' + key)
    require(state.get('Status') == 'exited' and state.get('Running') is False and state.get('ExitCode') == 0, ident + ': container did not exit successfully')
    require('OOMKilled' in state and not state['OOMKilled'], ident + ': OOM or unknown OOM state')
    require(report.get('success', True) is True and not report.get('errors') and not report.get('incomplete'), ident + ': application report failed')
    before, after = m['cgroup_before'], m['cgroup_after']
    require(before.get('memory.max') == str(memory_mib * 1024**2) and before.get('memory.swap.max') == '0', ident + ': limits differ')
    quota, period = before['cpu.max'].split()
    require(quota != 'max' and int(quota) == 2 * int(period), ident + ': CPU limit differs')
    old, new = counters(before['memory.events']), counters(after['memory.events'])
    delta = {k: new.get(k, 0) - old.get(k, 0) for k in set(old) | set(new)}
    require(all(x >= 0 for x in delta.values()), ident + ': cgroup counters went backwards')
    reclaim = m['reclaim_counter_deltas']
    swap = {k: after.get(k) for k in ('memory.swap.current', 'memory.swap.peak', 'memory.swap.events') if k in after}
    swap_values = [int(swap[k]) for k in ('memory.swap.current', 'memory.swap.peak') if k in swap and swap[k].isdigit()]
    swap_events = counters(swap['memory.swap.events']) if 'memory.swap.events' in swap else {}
    before_swap_events = counters(before.get('memory.swap.events', ''))
    swap_delta = {k: value - before_swap_events.get(k, 0) for k, value in swap_events.items()}
    flags = {'capped': delta.get('max', 0) > 0, 'reclaim': any(v > 0 for v in reclaim.values()),
             'oom': bool(state['OOMKilled'] or delta.get('oom', 0) or delta.get('oom_kill', 0)),
             'swap_pressure': any(v > 0 for v in swap_values) or any(v > 0 for v in swap_delta.values()),
             'worker_inconclusive': m['memory_comparison_inconclusive']}
    result = {'id': ident, 'cgroup_memory_peak_bytes': number(int(after['memory.peak']), ident),
              'process_peak_rss_kib': number(m['time']['peak_rss_kib'], ident),
              'worker_elapsed_seconds': number(m['elapsed_seconds'], ident),
              'command_elapsed_seconds': number(m['time']['elapsed_seconds'], ident),
              'sampled_output_peak_estimate_bytes': number(m['output_peak_estimate_bytes'], ident),
              'final_allocated_output_bytes': number(m['final_output']['total']['allocated_bytes'], ident),
              'boundary': report.get('boundary'), 'visited': report.get('visited'),
              'reported_retained_nodes': report.get('retained'),
              'reported_entries': report.get('entries'), 'reported_subtree_entries': report.get('subtree_entries'),
              'host_available_before_bytes': host['mem_available_bytes'] if host else None,
              'fallback_full_root': report.get('fallback_full_root'), 'flags': flags,
              'inconclusive': any(flags.values()), 'memory_events_delta': delta,
              'reclaim_counter_deltas': reclaim, 'swap_observed': swap, 'swap_events_delta': swap_delta}
    return result


def fixture_receipt(base, profile, stage):
    f = read(base / ('fixture-' + profile + '.json'))
    count = 100000 if stage == 'smoke' else 1000000
    files = (count if profile == 'files' else count // 2) + 169
    dirs = (0 if profile == 'files' else count // 2) + count // 1000 + 7
    require(f['profile'] == profile and f['scale'] == count, profile + ': invalid fixture identity')
    require(f['expected_counts'] == {'entries': files + dirs, 'files': files, 'dirs': dirs, 'others': 0}, profile + ': wrong fixture counts')
    capacity, threshold = f['capacity_bytes'], f['threshold_bytes']
    require(isinstance(capacity, int) and capacity > 0 and threshold == (capacity + 999) // 1000, profile + ': invalid fixture threshold')
    expected_retained = {'large', 'large/above', 'large/deep', 'large/deep/deeper', 'large/deep/deeper/large', 'large/group'}
    expected_omitted = {'large/below', 'large/sparse'} | {f'large/group/f{i:06}' for i in range(64)}
    controls = f['expected_controls']
    require(len(controls['retained']) == len(expected_retained) and set(controls['retained']) == expected_retained and len(controls['omitted']) == len(expected_omitted) and set(controls['omitted']) == expected_omitted, profile + ': invalid fixture control classifications')
    actual = f['controls']; group = actual['group']
    require(actual['below']['allocated'] < threshold <= actual['above']['allocated'] and actual['deep']['allocated'] >= threshold and actual['sparse']['allocated'] < threshold and actual['sparse']['apparent'] >= threshold, profile + ': measured control allocations invalid')
    require(group['files'] == 64 and group['minimum_child_allocated'] <= group['maximum_child_allocated'] < threshold <= group['child_allocated'], profile + ': invalid group controls')
    return f


def provenance(root, stage, smoke_root=None):
    directory = root / 'provenance'
    runtime = read(directory / 'runtime.json')
    versions = read(directory / 'runtime-versions.json')
    images = read(directory / 'image.json')
    require(isinstance(images, list) and len(images) == 1 and re.fullmatch(r'sha256:[0-9a-f]{64}', runtime['image_id']) and images[0]['Id'] == runtime['image_id'], 'runtime image identity mismatch')
    require(runtime['versions'] == versions and all(isinstance(versions.get(k), str) and versions[k] for k in ('python', 'sqlite', 'fio', 'time')), 'runtime version identity mismatch')
    packages = versions['packages']
    require(isinstance(packages, list) and len(packages) > 0 and len(set(packages)) == len(packages) and all(isinstance(p, str) and '=' in p and all(p.split('=', 1)) for p in packages), 'invalid runtime package identity')
    comparison = 'not supplied'
    archive_observation = 'not required for screen'
    if stage == 'smoke':
        archive = directory / 'runtime-image.tar.gz'
        if archive.is_file():
            require(archive.stat().st_size > 0, 'empty saved smoke runtime image')
            archive_observation = 'downloaded archive present; not loaded locally'
        else:
            inventory = read(root / 'zip-inventory.json')
            entries = [e for e in inventory if e['name'] == 'provenance/runtime-image.tar.gz']
            require(len(entries) == 1 and entries[0]['size'] > 0 and entries[0]['compressed_size'] > 0 and entries[0]['selected'] is False, 'missing archived smoke runtime image inventory')
            archive_observation = 'present in remote ZIP inventory; image bytes intentionally omitted locally, not verified by this analysis'
    if smoke_root is not None:
        smoke = read(smoke_root / 'provenance' / 'runtime.json')
        require(runtime == smoke, 'screen runtime differs from supplied smoke runtime')
        comparison = 'matched supplied smoke runtime'
    return {'image_id': runtime['image_id'], 'versions': versions, 'smoke_runtime_comparison': comparison,
            'runtime_archive_observation': archive_observation}


def calibration(root, required):
    path = root / 'disk-calibration.json'
    if not path.is_file():
        require(not required, 'missing screen disk calibration evidence')
        return {'checked_locally': False, 'note': 'Early selective smoke download omitted calibration; separate supplement is preserved.'}
    c = read(path)
    limits = {'rbps':33554432, 'wbps':33554432, 'riops':2200, 'wiops':3600}
    require(c.get('success') is True and c.get('profile') == 'local' and c.get('requested_limits') == limits, 'invalid disk calibration')
    installed = {p[0]:dict(t.split('=', 1) for t in p[1:]) for line in c['io_max'].splitlines() if (p := line.split())}
    require(all(installed.get(c['device'], {}).get(k) == str(v) for k,v in limits.items()), 'calibration limits not installed')
    reports = {j['mode']:j for j in c['jobs']}
    require(len(c['jobs']) == 4 and set(reports) == {'read','write','randread','randwrite'}, 'incomplete calibration modes')
    for mode,direction,metric,bound in [('read','read','bw_bytes',33554432),('write','write','bw_bytes',33554432),
                                         ('randread','read','iops',2200),('randwrite','write','iops',3600)]:
        raw = read(root / ('fio-' + mode + '.json'))['jobs']
        require(len(raw) == 1 and raw[0]['error'] == 0, 'fio calibration failed: ' + mode)
        value = raw[0][direction][metric]
        require(0 < value <= 1.25 * bound and reports[mode]['observed'] == value and reports[mode]['limit'] == bound, 'calibration observation mismatch: ' + mode)
    device = [line.split() for line in c['io_stat_after'].splitlines() if line.startswith(c['device'] + ' ')]
    require(len(device) == 1, 'missing calibration device activity')
    io = {k:int(v) for k,v in (item.split('=',1) for item in device[0][1:])}
    require(io.get('rbytes',0) > 0 and io.get('wbytes',0) > 0, 'no calibration read/write activity')
    return {'checked_locally': True, 'profile':c['profile'], 'device':c['device'], 'observations':c['jobs'],
            'scope':'Verified requested ceiling and direct I/O activity; physical local-device equivalence is not inferred.'}


def analyze(root, smoke_root=None):
    base = root / 'measurements' if (root / 'measurements' / 'manifest.json').is_file() else root
    manifest = read(base / 'manifest.json')
    cleanup = read(base.parent / 'runner-cleanup.json')
    require(cleanup.get('success') is True and cleanup.get('exit_code') == 0, 'runner cleanup failed or missing')
    stage = manifest['stage']
    require(stage in ('smoke', 'screen') and manifest.get('success') is True, 'invalid/unsuccessful manifest')
    if manifest.get('protocol') == 'cleanup-threshold-roomy-v2':
        require(manifest.get('resource_policy') == {'memory_mib':8192, 'swap_mib':0, 'cpus':2}, 'wrong roomy resource policy')
        memory_mib = 8192
        host_resource(manifest['resource_context'])
        host_resource(read(base.parent / 'provenance' / 'host-resource.json'))
    else:
        require(manifest.get('protocol') is None and manifest.get('source_sha') == 'd718f11fd3e64e7c8874c49d575d60067a0c9aee', 'unknown legacy protocol')
        memory_mib = 4096
    runtime = provenance(base.parent, stage, smoke_root)
    disk_calibration = calibration(base.parent, stage == 'screen')
    fixtures = {p: fixture_receipt(base, p, stage) for p in PROFILES}
    rounds = 1 if stage == 'smoke' else 3
    expected = {f'{p}-{s}-{n}-{a}' for p in PROFILES for s in SCENARIOS for n in range(1, rounds + 1) for a in ARMS}
    seeds = {f'seed-{p}-{a}-{n}' for p in PROFILES for a in ARMS for n in (1, 2)}
    cases = manifest['cases']
    ids = [c['id'] for c in cases]
    require(len(ids) == len(set(ids)) and set(ids) == expected | seeds, 'manifest case set/count mismatch')
    by_id = {c['id']: c for c in cases}
    phases = manifest['phases']
    names = [p['name'] for p in phases]
    require(len(names) == len(set(names)), 'duplicate phases')
    phase_map = {p['name']: p for p in phases}
    required_phases = expected | seeds | {f'fixture-{p}' for p in PROFILES} | {f'{p}-{s}-{n}-{suffix}' for p in PROFILES for s in SCENARIOS for n in range(1, rounds + 1) for suffix in ('oracle', 'cycle')}
    require(all(k in phase_map and phase_map[k].get('success') is True for k in required_phases), 'missing or failed required phase')
    for ident in sorted(seeds):
        point(base, by_id[ident], memory_mib)
    groups = {}
    for p in PROFILES:
        for s in SCENARIOS:
            group = {'arms': {}, 'comparisons': {}}
            for n in range(1, rounds + 1):
                oracle_id = f'{p}-{s}-{n}-oracle'
                o = read(base / (oracle_id + '.json'))
                if memory_mib == 8192:
                    host_resource(read(base / (oracle_id + '.host-resource.json')))
                require(o.get('success') is True and o.get('v2_checked') is True and o.get('fixture_checked') is True and all(isinstance(o.get(k), int) and o[k] > 0 for k in ('visited', 'retained', 'threshold')), oracle_id + ': invalid oracle output')
                require(o['visited'] == fixtures[p]['expected_counts']['entries'] and o['threshold'] == fixtures[p]['threshold_bytes'] and o['retained'] <= o['visited'], oracle_id + ': invalid oracle counts/threshold')
            for arm in ARMS:
                rows = []
                for n in range(1, rounds + 1):
                    ident = f'{p}-{s}-{n}-{arm}'
                    case = by_id[ident]
                    require(case['arm'] == arm and case['operation'] == ('scan' if s == 'full' else 'refresh'), ident + ': wrong arm/operation')
                    row = point(base, case, memory_mib)
                    if arm == 'probe':
                        expected_visited = 148 if s == 'retained' else 2 * fixtures[p]['expected_counts']['entries']
                        expected_boundary = '/scratch/tree/large' if s == 'retained' else '/scratch/tree'
                        require(row['visited'] == expected_visited and row['boundary'] == expected_boundary and row['fallback_full_root'] is (s == 'omitted'), ident + ': unexpected visits/boundary/fallback')
                        oracle = read(base / f'{p}-{s}-{n}-oracle.json')
                        require(row['reported_retained_nodes'] == oracle['retained'], ident + ': retained count differs from independent oracle')
                    else:
                        require(row['reported_entries'] == fixtures[p]['expected_counts']['entries'], ident + ': wrong full entry count')
                        if s != 'full':
                            require(row['reported_subtree_entries'] == (74 if s == 'retained' else 100), ident + ': wrong refresh size')
                    rows.append(row)
                group['arms'][arm] = rows
            for reference in ('v1', 'v2'):
                changes = {}
                for metric in METRICS:
                    pairs = []
                    for probe, base_row in zip(group['arms']['probe'], group['arms'][reference]):
                        denominator = base_row[metric]
                        pairs.append(None if denominator == 0 else 100 * (probe[metric] / denominator - 1))
                    changes[metric] = {'paired_percent_changes': pairs,
                        'median_paired_percent_change': statistics.median(pairs) if all(x is not None for x in pairs) else None}
                changes['inconclusive'] = any(r['inconclusive'] for arm in ('probe', reference) for r in group['arms'][arm])
                group['comparisons'][reference] = changes
            groups[p + '/' + s] = group
    return {'valid': True, 'stage': stage, 'source_sha': manifest.get('source_sha'), 'timed_cases': len(expected),
            'seed_cases_excluded': len(seeds), 'rounds': rounds, 'protocol_only': stage == 'smoke',
            'optimization_acceptance': 'not evaluated', 'memory_ceiling_mib': memory_mib, 'runtime_provenance': runtime,
            'disk_calibration': disk_calibration, 'fixtures': fixtures, 'groups': groups,
            'notes': ['Output peak is a sampled lower-bound estimate, not exact peak allocation.',
                      'Worker elapsed includes timeout completion polling (up to about50ms); GNU command elapsed is also retained, with0.01s resolution. Small fast-operation differences need cautious interpretation.',
                      'Memory peak includes the writer container wrapper and sampler.',
                      'Flags preserve observed cap, reclaim, OOM and swap pressure; flagged comparisons are inconclusive.']}


def main():
    try:
        parser = argparse.ArgumentParser()
        parser.add_argument('artifact_root', type=Path)
        parser.add_argument('--smoke-root', type=Path, help='optional smoke artifact root for cross-run runtime identity')
        args = parser.parse_args()
        print(json.dumps(analyze(args.artifact_root, args.smoke_root), indent=2, allow_nan=False))
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as exc:
        print(json.dumps({'valid': False, 'error': str(exc)}))
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
