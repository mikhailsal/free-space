#!/usr/bin/env python3
"""Judge downloaded bucket-probe artifacts against the pre-registered bars and print JSON.

Usage: cleanup_probe_analysis.py ARTIFACT_ROOT [--smoke-root SMOKE_ROOT]

ARTIFACT_ROOT contains measurements/manifest.json and runner-cleanup.json (the measurements directory itself is also
accepted when its parent has the cleanup file). Exit status is nonzero when an artifact is invalid.

The bars and the evidence rules are those of docs/designs/cleanup-bucket-experiment.md ("Decision bars", "Peak disk").
They are implemented once, in the pure functions of the first section, are covered by
scripts/tests/test_cleanup_analysis.py and are committed with the source that is measured, before any run of it.
Nothing here is tuned after results are seen. The artifact plumbing in the second section may be corrected after a
run if a real artifact shows a layout mistake; such a correction never touches the first section.
"""
import argparse
import json
import math
import re
import statistics
import sys
from fractions import Fraction
from pathlib import Path

PROFILES = ('files', 'directories')
SCENARIOS = ('full', 'retained', 'omitted', 'wide')
ARMS = ('v1', 'probe')
CELLS = tuple(f'{p}/{s}' for p in PROFILES for s in SCENARIOS)
METRICS = ('cgroup_memory_peak_bytes', 'process_peak_rss_kib', 'worker_elapsed_seconds', 'command_elapsed_seconds',
           'sampled_output_peak_estimate_bytes', 'final_allocated_output_bytes', 'peak_disk_bytes')
MIB, KIB = 1024 ** 2, 1024
# Noise floors: a difference counts only when it exceeds BOTH the relative and the absolute floor, which is the same as
# allowing max(relative * v1, absolute) of difference. Every comparison is exact (Fractions of the decimal values as
# written), so a tie such as 1.05 s against 1.00 s is a tie and not a float rounding error beyond the floor.
FLOORS = {'time': (Fraction('0.05'), Fraction('0.05')), 'memory': (Fraction('0.02'), Fraction(MIB)),
          'disk': (Fraction('0.05'), Fraction(4 * KIB))}
BAR = {'speed': Fraction('0.15'), 'rss': Fraction('0.25'), 'disk': Fraction('0.25'), 'full_scan_slowdown': Fraction('0.15'),
       'ram_exchange_bytes': 256 * MIB}
# The sampler looks at the output filesystem every 50 ms, so a transient shorter than that can be missed. Publishing a
# snapshot holds at most four snapshot-sized files at once (current, previous, the copy that becomes previous, and the
# new one), so the probe's disk is judged at four times its observed value; v1 is judged at its observed value, which can
# only understate it. Reported tables keep the observed numbers.
PROBE_DISK_FACTOR = 4
FLAG_NAMES = ('capped', 'reclaim', 'oom', 'swap_pressure', 'worker_inconclusive')


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


# ---------------------------------------------------------------- the bars (pure functions, frozen with the source)

def exact(value):
    """The number as written: a float is read through its shortest repr, so 0.07 is 7/100 and not a binary neighbour."""
    if isinstance(value, Fraction):
        return value
    return Fraction(str(value)) if isinstance(value, float) else Fraction(value)


def metric_value(row, kind, arm):
    """Value of a metric as a bar judges it, in its natural unit (seconds, bytes)."""
    if kind == 'disk':
        return row['peak_disk_bytes'] * (PROBE_DISK_FACTOR if arm == 'probe' else 1)
    return {'time': row['command_elapsed_seconds'], 'rss': row['process_peak_rss_kib'] * KIB, 'memory': row['cgroup_memory_peak_bytes']}[kind]


def floors_kind(kind):
    return {'time': 'time', 'rss': 'memory', 'memory': 'memory', 'disk': 'disk'}[kind]


def direction(base, probe, kind):
    """+1 when the probe is worse beyond noise, -1 when better beyond noise, 0 when within the floors."""
    relative, absolute = FLOORS[floors_kind(kind)]
    base, probe = exact(base), exact(probe)
    diff = probe - base
    if abs(diff) <= absolute or (base > 0 and abs(diff) / base <= relative):
        return 0
    return 1 if diff > 0 else -1


def change(base, probe):
    base = exact(base)
    return None if base == 0 else (exact(probe) - base) / base


def non_inferior(pairs, kind):
    """Passed only when shown in every repeat: the probe is never worse than v1 beyond the noise floors."""
    return all(direction(b, p, kind) <= 0 for b, p in pairs)


def confirmed_benefit(pairs, kind, bar):
    """All repeats better beyond noise, each by at least `bar`, and so is the median."""
    bar = exact(bar)
    changes = [change(b, p) for b, p in pairs]
    return (all(c is not None and -c >= bar for c in changes) and all(direction(b, p, kind) < 0 for b, p in pairs)
            and -statistics.median(changes) >= bar)


def verdict(passed, inconclusive):
    return 'inconclusive' if inconclusive else ('passed' if passed else 'failed')


def full_scan_bar(pairs_time, others, memory_flagged=False):
    """D4/D5/D14. Every repeat must be at most +15% slower, with no noise floor added to that tolerance. A repeat slower
    beyond the noise floors needs an exchange: another metric better in EVERY repeat by at least twice the WORST repeat's
    slowdown, and for memory also by at least 256 MiB absolute. `others` maps a name to (pairs, kind, absolute_bytes).
    With a memory flag the memory exchanges cannot be judged: if no disk exchange qualifies the bar is inconclusive."""
    changes = [change(b, p) for b, p in pairs_time]
    if any(c is None or c > BAR['full_scan_slowdown'] for c in changes):
        return 'failed', 'a repeat is more than +15% slower'
    if all(direction(b, p, 'time') <= 0 for b, p in pairs_time):
        return 'passed', 'no slowdown beyond noise'
    worst = max(c for c in changes if c > 0)
    for name, (pairs, kind, saved) in others.items():
        if kind != 'disk' and memory_flagged:
            continue
        if confirmed_benefit(pairs, kind, 2 * worst) and all(b - p >= saved for b, p in pairs):
            return 'passed', f'exchange: confirmed {name} reduction of at least {float(2 * worst):.1%} (twice the worst slowdown)'
    if memory_flagged:
        return 'inconclusive', 'slowdown beyond noise; memory exchanges cannot be judged under a memory flag'
    return 'failed', 'slowdown beyond noise without a qualifying exchange'


def judge_group(rows_by_arm, scenario):
    """Per-bar verdicts for one profile and scenario from the paired repeats (v1 rows and probe rows in repeat order)."""
    v1, probe = rows_by_arm['v1'], rows_by_arm['probe']
    require(len(v1) == len(probe) and v1, 'unpaired repeats')
    pairs = {kind: [(metric_value(a, kind, 'v1'), metric_value(b, kind, 'probe')) for a, b in zip(v1, probe)]
             for kind in ('time', 'rss', 'memory', 'disk')}
    flagged = any(r['flags'][k] for r in v1 + probe for k in FLAG_NAMES)
    out = {'memory_flags': flagged,
           'D2_peak_disk_no_increase': verdict(non_inferior(pairs['disk'], 'disk'), False),
           'D3_rss_no_increase': verdict(non_inferior(pairs['rss'], 'rss'), flagged),
           'D8_speed_benefit_15pct': verdict(confirmed_benefit(pairs['time'], 'time', BAR['speed']), False),
           'D9_rss_benefit_25pct': verdict(confirmed_benefit(pairs['rss'], 'rss', BAR['rss']), flagged),
           'D10_disk_benefit_25pct': verdict(confirmed_benefit(pairs['disk'], 'disk', BAR['disk']), False)}
    if scenario == 'full':
        # D5 names peak disk and peak RSS as exchange metrics. Total cgroup memory holds reclaimable page cache, so it is reported
        # but never buys a slowdown. D5 also names refresh time; that exchange is deliberately not implemented (stricter).
        result, why = full_scan_bar(pairs['time'], {
            'disk': (pairs['disk'], 'disk', 0),
            'rss': (pairs['rss'], 'rss', BAR['ram_exchange_bytes'])}, flagged)
        out['D4_full_scan_limit_and_exchange'] = result
        out['D4_reason'] = why
    else:
        out['D6_refresh_no_slowdown'] = verdict(non_inferior(pairs['time'], 'time'), False)
    # Reports only. `judged` carries the probe's disk at the 4x factor the bars use; `observed` carries it as measured.
    out['judged_median_change'] = median_changes(pairs)
    out['observed_median_change'] = median_changes({**pairs, 'disk': [(a['peak_disk_bytes'], b['peak_disk_bytes']) for a, b in zip(v1, probe)]})
    return out


def median_changes(pairs):
    """Median relative change per metric as plain floats for the report; no bar reads it."""
    return {kind: float(statistics.median([change(b, p) for b, p in v])) if all(b for b, _ in v) else None for kind, v in pairs.items()}


def gate_status(verdicts):
    """One gate over its cells: failed if any failed, passed only if every one passed, otherwise inconclusive (also empty)."""
    if any(v == 'failed' for v in verdicts):
        return 'failed'
    return 'passed' if verdicts and all(v == 'passed' for v in verdicts) else 'inconclusive'


def overall(groups):
    """The pre-registered success rule: every non-inferiority bar passed in every cell it applies to, and one benefit bar
    confirmed. All eight profile/scenario cells must be present, so no gate can pass by being left out."""
    require(set(groups) == set(CELLS), 'every profile and scenario must be judged exactly once')
    gates = ('D2_peak_disk_no_increase', 'D3_rss_no_increase', 'D6_refresh_no_slowdown', 'D4_full_scan_limit_and_exchange')
    status = {gate: [g[gate] for g in groups.values() if gate in g] for gate in gates}
    non_inferiority = {gate: gate_status(vals) for gate, vals in status.items()}
    benefits = sorted({f'{name}:{bar}' for name, g in groups.items() for bar in
                       ('D8_speed_benefit_15pct', 'D9_rss_benefit_25pct', 'D10_disk_benefit_25pct') if g[bar] == 'passed'})
    return {'non_inferiority': non_inferiority, 'confirmed_benefits': benefits,
            'success': all(v == 'passed' for v in non_inferiority.values()) and bool(benefits),
            'open_by_design': ['D3 minimum tested successful memory budget (not measured)', 'D11 capacity alternative (not measured)']}


def report_bars(groups, judged):
    """Per-bar verdicts as reported. The smoke has one pair per cell and judges nothing, so it must never print 'passed'."""
    return groups if judged else {cell: 'not judged: the smoke has one pair per cell' for cell in groups}


def predictions(tables):
    """The predictions recorded before any run, scored hit or miss per cell on the medians of the same run. They are
    reported beside `overall` and never change it: `overall` is the verdict, a miss here is a surprise about the idea."""
    scored = {}
    for cell, t in tables.items():
        scenario = cell.split('/')[1]
        v1, probe = t['v1'], t['probe']
        v1_time, probe_time = exact(v1['command_elapsed_seconds']), exact(probe['command_elapsed_seconds'])
        checks = {'rss_lower_than_v1': exact(probe['process_peak_rss_kib']) < exact(v1['process_peak_rss_kib']),
                  'disk_at_4x_under_a_tenth_of_v1': exact(probe['peak_disk_bytes']) * PROBE_DISK_FACTOR * 10 < exact(v1['peak_disk_bytes']),
                  'snapshot_pair_under_1_mib': exact(probe['final_allocated_output_bytes']) < MIB}
        if scenario == 'full':
            checks['faster_than_v1'] = probe_time < v1_time
        elif scenario == 'retained':
            checks['under_a_tenth_of_v1_and_0.1_s'] = probe_time * 10 < v1_time and probe_time < Fraction(1, 10)
        else:
            checks['under_a_tenth_of_v1_and_1_s'] = probe_time * 10 < v1_time and probe_time < 1
        scored[cell] = checks
    return {'cells': scored, 'all_hit': all(all(c.values()) for c in scored.values())}


# ---------------------------------------------------------------- artifact validation and plumbing

def host_resource(receipt):
    require(receipt.get('success') is True, 'failed host resource receipt')
    for key in ('mem_total_bytes', 'mem_available_bytes'):
        require(type(receipt.get(key)) is int and receipt[key] >= 10 * 1024**3, 'insufficient host memory: ' + key)
    require(receipt['mem_available_bytes'] <= receipt['mem_total_bytes'], 'impossible host available memory')
    require(type(receipt.get('cpu_count')) is int and receipt['cpu_count'] >= 2, 'insufficient host CPUs')
    require(receipt.get('writer_ceiling_bytes') == 8 * 1024**3 and receipt.get('host_overhead_bytes') == 2 * 1024**3, 'wrong host headroom policy')
    return receipt


def point(base, case, memory_mib=8192):
    ident = case['id']
    m = read(base / ident / 'metrics.json')
    state = read(base / ident / 'docker-state.json')
    report = m['report']
    host_resource(read(base / (ident + '.host-resource.json')))
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
    sampled = number(m['output_peak_estimate_bytes'], ident)
    final = number(m['final_output']['total']['allocated_bytes'], ident)
    return {'id': ident, 'cgroup_memory_peak_bytes': number(int(after['memory.peak']), ident),
            'process_peak_rss_kib': number(m['time']['peak_rss_kib'], ident),
            'worker_elapsed_seconds': number(m['elapsed_seconds'], ident),
            'command_elapsed_seconds': number(m['time']['elapsed_seconds'], ident),
            'sampled_output_peak_estimate_bytes': sampled, 'final_allocated_output_bytes': final,
            'peak_disk_bytes': max(sampled, final),
            'boundary': report.get('boundary'), 'visited': report.get('visited'), 'scope': report.get('scope'),
            'reported_retained_nodes': report.get('retained'), 'reported_buckets': report.get('buckets'),
            'reported_range': report.get('range'), 'reported_entries': report.get('entries'),
            'reported_subtree_entries': report.get('subtree_entries'), 'fallback_full_root': report.get('fallback_full_root'),
            'flags': flags, 'inconclusive': any(flags.values()), 'memory_events_delta': delta,
            'reclaim_counter_deltas': reclaim, 'swap_observed': swap, 'swap_events_delta': swap_delta}


def fixture_receipt(base, profile, stage):
    f = read(base / ('fixture-' + profile + '.json'))
    count = 100000 if stage == 'smoke' else 1000000
    files = (count if profile == 'files' else count // 2) + 169
    dirs = (0 if profile == 'files' else count // 2) + count // 1000 + 7
    require(f['profile'] == profile and f['scale'] == count, profile + ': invalid fixture identity')
    require(f['expected_counts'] == {'entries': files + dirs, 'files': files, 'dirs': dirs, 'others': 0}, profile + ': wrong fixture counts')
    capacity, threshold = f['capacity_bytes'], f['threshold_bytes']
    require(isinstance(capacity, int) and capacity > 0 and threshold == (capacity + 999) // 1000, profile + ': invalid fixture threshold')
    expected_retained = {'large', 'large/above', 'large/deep', 'large/deep/deeper', 'large/deep/deeper/large', 'large/group', 'bulk'}
    expected_omitted = {'large/below', 'large/sparse', 'small', 'bulk/g000050'} | {f'large/group/f{i:06}' for i in range(64)}
    controls = f['expected_controls']
    require(set(controls['retained']) == expected_retained and len(controls['retained']) == len(expected_retained)
            and set(controls['omitted']) == expected_omitted and len(controls['omitted']) == len(expected_omitted),
            profile + ': invalid fixture control classifications')
    actual = f['controls']
    group = actual['group']
    require(actual['below']['allocated'] < threshold <= actual['above']['allocated'] and actual['deep']['allocated'] >= threshold
            and actual['sparse']['allocated'] < threshold and actual['sparse']['apparent'] >= threshold, profile + ': measured control allocations invalid')
    require(group['files'] == 64 and group['minimum_child_allocated'] <= group['maximum_child_allocated'] < threshold <= group['child_allocated'],
            profile + ': invalid group controls')
    return f


def expected_probe(scenario, entries, stage):
    """What the probe must report, derived here by hand from the plan's reasoning and not imported from it."""
    root = '/scratch/tree'
    if scenario == 'full':
        return dict(visited=2 * entries, boundary=root, scope='full', fallback_full_root=False, range=None)
    if scenario == 'retained':
        return dict(visited=148, boundary=root + '/large', scope='retained', fallback_full_root=False, range=None)
    if scenario == 'omitted':
        return dict(visited=202, boundary=root, scope='range', fallback_full_root=False,
                    range=dict(lo=0, hi=4096, old_entries=100, new_entries=100))
    lo, hi = (804, 971) if stage == 'smoke' else (945, 956)
    return dict(visited=8010, boundary=root + '/bulk', scope='range', fallback_full_root=False,
                range=dict(lo=lo, hi=hi, old_entries=4004, new_entries=4004))


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
        return {'checked_locally': False, 'note': 'Not required for the smoke stage; the screen stage must provide it.'}
    c = read(path)
    limits = {'rbps': 33554432, 'wbps': 33554432, 'riops': 2200, 'wiops': 3600}
    require(c.get('success') is True and c.get('profile') == 'local' and c.get('requested_limits') == limits, 'invalid disk calibration')
    installed = {p[0]: dict(t.split('=', 1) for t in p[1:]) for line in c['io_max'].splitlines() if (p := line.split())}
    require(all(installed.get(c['device'], {}).get(k) == str(v) for k, v in limits.items()), 'calibration limits not installed')
    reports = {j['mode']: j for j in c['jobs']}
    require(len(c['jobs']) == 4 and set(reports) == {'read', 'write', 'randread', 'randwrite'}, 'incomplete calibration modes')
    for mode, side, metric, bound in [('read', 'read', 'bw_bytes', 33554432), ('write', 'write', 'bw_bytes', 33554432),
                                       ('randread', 'read', 'iops', 2200), ('randwrite', 'write', 'iops', 3600)]:
        raw = read(root / ('fio-' + mode + '.json'))['jobs']
        require(len(raw) == 1 and raw[0]['error'] == 0, 'fio calibration failed: ' + mode)
        value = raw[0][side][metric]
        require(0 < value <= 1.25 * bound and reports[mode]['observed'] == value and reports[mode]['limit'] == bound, 'calibration observation mismatch: ' + mode)
    device = [line.split() for line in c['io_stat_after'].splitlines() if line.startswith(c['device'] + ' ')]
    require(len(device) == 1, 'missing calibration device activity')
    io = {k: int(v) for k, v in (item.split('=', 1) for item in device[0][1:])}
    require(io.get('rbytes', 0) > 0 and io.get('wbytes', 0) > 0, 'no calibration read/write activity')
    return {'checked_locally': True, 'profile': c['profile'], 'device': c['device'], 'observations': c['jobs'],
            'scope': 'Verified requested ceiling and direct I/O activity; physical local-device equivalence is not inferred.'}


def check_oracle(base, oracle_id, fixture):
    o = read(base / (oracle_id + '.json'))
    host_resource(read(base / (oracle_id + '.host-resource.json')))
    require(o.get('success') is True and o.get('v2_checked') is False and o.get('fixture_checked') is True, oracle_id + ': invalid oracle output')
    require(all(isinstance(o.get(k), int) and o[k] > 0 for k in ('visited', 'retained', 'buckets', 'threshold')), oracle_id + ': invalid oracle counts')
    require(isinstance(o.get('max_bucket_entries'), int) and o['max_bucket_entries'] >= 0, oracle_id + ': invalid bucket size')
    require(o['visited'] == fixture['expected_counts']['entries'] and o['threshold'] == fixture['threshold_bytes'] and o['retained'] <= o['visited'],
            oracle_id + ': invalid oracle counts/threshold')
    # The oracle compared the stored scan stamps with the seed under the frozen plan's expectation.
    require(o.get('refresh_checked') is True and isinstance(o.get('fresh_nodes'), int) and isinstance(o.get('fresh_buckets'), int)
            and o['fresh_nodes'] + o['fresh_buckets'] > 0, oracle_id + ': the oracle did not prove which items were rewritten')
    return o


def check_arm_row(row, arm, scenario, fixture, stage, ident, oracle):
    entries = fixture['expected_counts']['entries']
    if arm == 'probe':
        want = expected_probe(scenario, entries, stage)
        got = dict(visited=row['visited'], boundary=row['boundary'], scope=row['scope'], fallback_full_root=row['fallback_full_root'],
                   range=row['reported_range'])
        require(got == want, f'{ident}: unexpected scope {got} != {want}')
        require(row['reported_retained_nodes'] == oracle['retained'], ident + ': retained count differs from independent oracle')
        require(row['reported_buckets'] == oracle['buckets'], ident + ': bucket count differs from independent oracle')
    elif scenario == 'full':
        require(row['reported_entries'] == entries, ident + ': wrong full entry count')
    else:  # a v1 refresh reports the size of the refreshed subtree; the plan asks the worker for nothing else
        want = {'retained': 74, 'omitted': 100, 'wide': 1001}[scenario]
        require(row['reported_subtree_entries'] == want, ident + ': wrong refresh size')


def analyze(root, smoke_root=None):
    base = root / 'measurements' if (root / 'measurements' / 'manifest.json').is_file() else root
    manifest = read(base / 'manifest.json')
    cleanup = read(base.parent / 'runner-cleanup.json')
    require(cleanup.get('success') is True and cleanup.get('exit_code') == 0, 'runner cleanup failed or missing')
    stage = manifest['stage']
    require(stage in ('smoke', 'screen') and manifest.get('success') is True, 'invalid/unsuccessful manifest')
    require(manifest.get('protocol') == 'cleanup-threshold-roomy-v2'
            and manifest.get('resource_policy') == {'memory_mib': 8192, 'swap_mib': 0, 'cpus': 2}, 'wrong protocol or resource policy')
    host_resource(manifest['resource_context'])
    host_resource(read(base.parent / 'provenance' / 'host-resource.json'))
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
    phase_map = {p['name']: p for p in manifest['phases']}
    require(len(phase_map) == len(manifest['phases']), 'duplicate phases')
    required_phases = expected | seeds | {f'fixture-{p}' for p in PROFILES} | {
        f'{p}-{s}-{n}-{suffix}' for p in PROFILES for s in SCENARIOS for n in range(1, rounds + 1) for suffix in ('oracle', 'cycle')}
    require(all(k in phase_map and phase_map[k].get('success') is True for k in required_phases), 'missing or failed required phase')
    for ident in sorted(seeds):
        point(base, by_id[ident])
    groups, tables = {}, {}
    for p in PROFILES:
        for s in SCENARIOS:
            arms = {a: [] for a in ARMS}
            for n in range(1, rounds + 1):
                oracle = check_oracle(base, f'{p}-{s}-{n}-oracle', fixtures[p])
                for arm in ARMS:
                    ident = f'{p}-{s}-{n}-{arm}'
                    case = by_id[ident]
                    require(case['arm'] == arm and case['operation'] == ('scan' if s == 'full' else 'refresh'), ident + ': wrong arm/operation')
                    row = point(base, case)
                    check_arm_row(row, arm, s, fixtures[p], stage, ident, oracle)
                    arms[arm].append(row)
            groups[f'{p}/{s}'] = judge_group(arms, s)
            tables[f'{p}/{s}'] = {arm: {m: statistics.median([r[m] for r in rows]) for m in METRICS} for arm, rows in arms.items()}
            tables[f'{p}/{s}']['repeats'] = {arm: [{m: r[m] for m in METRICS} for r in rows] for arm, rows in arms.items()}
    judged = stage == 'screen'
    return {'valid': True, 'stage': stage, 'source_sha': manifest.get('source_sha'), 'timed_cases': len(expected),
            'seed_cases_excluded': len(seeds), 'rounds': rounds, 'bars_are_judged': judged,
            'memory_ceiling_mib': 8192, 'runtime_provenance': runtime, 'disk_calibration': disk_calibration,
            'fixtures': fixtures, 'medians_and_repeats': tables,
            'bars': report_bars(groups, judged),
            'overall': overall(groups) if judged else 'not evaluated (the smoke checks the protocol only; one pair cannot confirm a bar)',
            'predictions': predictions(tables) if judged else 'not scored (the smoke has one pair per cell)',
            'notes': ['Elapsed time is the GNU command wall time (0.01 s resolution); worker elapsed time is kept alongside.',
                      f'Peak disk is the larger of the sampled output peak (a lower bound) and the final allocated output; the probe is judged at {PROBE_DISK_FACTOR}x that value.',
                      'RSS is the writer process peak; cgroup memory includes cache and slab and is reported separately.',
                      'Flags (cap, reclaim, OOM, swap) make only the memory comparisons inconclusive; inconclusive is not a pass.']}


def main(argv=None):
    try:
        parser = argparse.ArgumentParser()
        parser.add_argument('artifact_root', type=Path)
        parser.add_argument('--smoke-root', type=Path, help='optional smoke artifact root for cross-run runtime identity')
        args = parser.parse_args(argv)
        print(json.dumps(analyze(args.artifact_root, args.smoke_root), indent=2, allow_nan=False))
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as exc:
        print(json.dumps({'valid': False, 'error': repr(exc)}))
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
