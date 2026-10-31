#!/usr/bin/env python3
"""Validate and describe paired v1 benchmark receipts without running benchmarks."""
import argparse
import json
import math
from pathlib import Path
import re
import statistics

PROTOCOL = 'v1-single-operation-pairs-v2'
SCOPE = 'single CLI and sampler; observer excluded'
ARMS = ('baseline-v1', 'candidate-v1')
HEX = re.compile(r'^[0-9a-f]{64}$')
ID = re.compile(r'^[A-Za-z0-9][A-Za-z0-9_-]*$')


def number(value, positive=False):
    return (isinstance(value, (int, float)) and not isinstance(value, bool)
            and math.isfinite(value) and (value > 0 if positive else value >= 0))


def integer(value, positive=False):
    return isinstance(value, int) and not isinstance(value, bool) and (value > 0 if positive else value >= 0)


def digest(value):
    return isinstance(value, str) and HEX.fullmatch(value) is not None


def read_json(path, issues):
    try:
        if path.is_symlink():
            raise ValueError('symlink not permitted')
        data = json.loads(path.read_text())
        if not isinstance(data, dict):
            raise ValueError('expected object')
        return data
    except (OSError, ValueError) as error:
        issues.append(f'{path}: invalid or missing JSON: {error}')
        return {}


def cgroup_ok(cgroup):
    try:
        events = cgroup['memory.events']
        if not isinstance(events, str):
            return False
        rows = [line.split() for line in events.splitlines()]
        if any(len(row) != 2 for row in rows) or len({row[0] for row in rows}) != len(rows):
            return False
        values = {key: int(value) for key, value in rows}
        return all(value >= 0 for value in values.values()) and values['oom'] == values['oom_kill'] == 0
    except (KeyError, TypeError, ValueError):
        return False


def state_ok(state):
    return (state.get('Status') == 'exited' and type(state.get('ExitCode')) is int and state['ExitCode'] == 0
            and state.get('Running') is False and state.get('OOMKilled') is False and state.get('Error') == '')


def summarize(root: Path, require_runner_cleanup=True) -> dict:
    root = Path(root)
    issues, pairs = [], []
    manifest = read_json(root / 'manifest.json', issues)
    status = read_json(root / 'status.json', issues)
    admission = read_json(root / 'capacity-admission.json', issues)
    if require_runner_cleanup:
        cleanup = read_json(root / 'runner-cleanup.json', issues)
        if cleanup.get('success') is not True:
            issues.append('runner cleanup failed or missing')
    if (status.get('success') is not True or status.get('completed') is not True
            or status.get('cleanup_success') is not True or status.get('failures') != []):
        issues.append('status: experiment failed or incomplete')
    if admission.get('success') is not True:
        issues.append('capacity admission failed or missing')
    ids = manifest.get('expected_arms')
    count, files, operation = manifest.get('pairs'), manifest.get('files'), manifest.get('operation')
    receipt = manifest.get('fixture_receipt')
    expected = receipt.get('expected_entries') if isinstance(receipt, dict) else None
    provenance = manifest.get('provenance')
    if manifest.get('measurement_protocol') != PROTOCOL:
        issues.append('manifest: incorrect measurement protocol')
    if not integer(count, True) or not integer(files, True) or operation not in ('scan', 'refresh') or not integer(expected):
        issues.append('manifest: invalid pair count, files, operation or fixture receipt')
    if (not isinstance(ids, list) or not ids or any(not isinstance(x, str) or not ID.fullmatch(x) for x in ids)
            or len(set(ids)) != len(ids) or not integer(count, True) or len(ids) != 2 * count):
        issues.append('manifest: invalid expected arm ids')
        ids = []
    if not isinstance(provenance, dict):
        provenance = {}
    for arm in ARMS:
        p = provenance.get(arm)
        if not isinstance(p, dict) or not digest(p.get('sha256')) or not isinstance(p.get('source_revision'), str) or not p['source_revision']:
            issues.append(f'manifest: invalid provenance for {arm}')
    # Arm IDs are direct children and must never resolve through links.
    try:
        for path in root.iterdir():
            if path.is_symlink():
                issues.append(f'unexpected symlink: {path.name}')
            elif path.is_dir() and path.name not in ids:
                issues.append(f'unexpected directory: {path.name}')
    except OSError as error:
        issues.append(f'cannot list artifact root: {error}')
    rows = {}
    for ident in ids:
        directory = root / ident
        if directory.is_symlink() or not directory.is_dir():
            issues.append(f'{ident}: missing or unsafe directory')
            continue
        for child in directory.rglob('*'):
            if child.is_symlink() or (child.name == 'spec.json' and child.parent != directory):
                issues.append(f'{ident}: unexpected symlink or nested specification: {child}')
        spec = read_json(directory / 'spec.json', issues)
        metric = read_json(directory / 'metrics.json', issues)
        obs = read_json(directory / 'observer.json', issues)
        flush = read_json(directory / 'cache-flush.json', issues)
        writer = read_json(directory / 'writer-state.json', issues)
        observer_state = read_json(directory / 'observer-state.json', issues)
        arm, pair = spec.get('arm'), spec.get('pair')
        p = provenance.get(arm, {}) if isinstance(arm, str) else {}
        if not isinstance(p, dict):
            p = {}
        valid_spec = (spec.get('id') == ident and arm in ARMS and integer(pair, True) and integer(count, True) and pair <= count
                      and spec.get('operation') == operation and spec.get('backend') == 'v1'
                      and spec.get('files') == files and integer(spec.get('memory_mib'), True)
                      and spec.get('expected_entries') == expected
                      and (operation != 'refresh' or integer(spec.get('expected_subtree_entries')))
                      and spec.get('binary_sha256') == p.get('sha256')
                      and spec.get('source_revision') == p.get('source_revision')
                      and (digest(spec.get('seed_sha256')) or (operation == 'scan' and spec.get('seed_sha256') == 'empty')))
        if not valid_spec:
            issues.append(f'{ident}: invalid spec or provenance')
        report, time = metric.get('report'), metric.get('time')
        if not isinstance(report, dict):
            report = {}
        if not isinstance(time, dict):
            time = {}
        bound_keys = ('id', 'arm', 'pair', 'operation', 'backend', 'expected_entries', 'binary_sha256',
                      'source_revision', 'seed_sha256', 'memory_mib')
        bound = all(k in metric and k in spec and type(metric[k]) is type(spec[k]) and metric[k] == spec[k]
                    for k in bound_keys)
        cg = metric.get('cgroup_after')
        limits_ok = (isinstance(cg, dict) and integer(spec.get('memory_mib'), True)
                     and cg.get('memory.max') == str(spec['memory_mib'] * 1024 * 1024)
                     and cg.get('memory.swap.max') == '0')
        if not (bound and limits_ok and metric.get('validated') is True and type(metric.get('exit_code')) is int and metric['exit_code'] == 0
                and metric.get('timed_out') is False and metric.get('sampler_errors') == []
                and integer(report.get('entries')) and report.get('entries') == expected
                and type(report.get('errors')) is int and report.get('errors') == 0 and report.get('incomplete') is False
                and (operation != 'refresh' or (integer(report.get('subtree_entries'))
                     and report['subtree_entries'] == spec.get('expected_subtree_entries')))
                and number(metric.get('elapsed_seconds'), True) and number(time.get('peak_rss_kib'), True)
                and number(metric.get('app_disk_peak_bytes')) and metric.get('memory_peak_scope') == SCOPE
                and cgroup_ok(metric.get('cgroup_after'))):
            issues.append(f'{ident}: invalid metrics or cgroup')
        resources = obs.get('resources')
        if not isinstance(resources, dict):
            resources = {}
        if not (obs.get('validated') is True and obs.get('logical_content_unchanged') is True
                and digest(obs.get('semantic_sha256')) and integer(obs.get('current_entries')) and obs.get('current_entries') == expected
                and (obs.get('previous_entries') is None or integer(obs.get('previous_entries')))
                and 'previous_entries' in obs and obs.get('errors') == {} and obs.get('seed_sha256') == spec.get('seed_sha256')
                and number(resources.get('elapsed_seconds'), True) and cgroup_ok(resources.get('cgroup'))):
            issues.append(f'{ident}: invalid observer')
        if flush.get('success') is not True or not state_ok(writer) or not state_ok(observer_state):
            issues.append(f'{ident}: cache flush or container state failed')
        if valid_spec:
            key = (pair, arm)
            if key in rows:
                issues.append(f'{ident}: duplicate pair/arm')
            rows[key] = (spec, metric, obs)
    if integer(count, True):
        for pair in range(1, count + 1):
            a, b = rows.get((pair, ARMS[0])), rows.get((pair, ARMS[1]))
            if not a or not b:
                issues.append(f'pair {pair}: missing baseline or candidate')
                continue
            sa, ma, oa = a
            sb, mb, ob = b
            ignored = {'id', 'arm', 'binary_sha256', 'source_revision', 'initial_snapshot_allocated_bytes'}
            if {k: v for k, v in sa.items() if k not in ignored} != {k: v for k, v in sb.items() if k not in ignored}:
                issues.append(f'pair {pair}: configuration or seed mismatch')
            if any(oa.get(k) != ob.get(k) for k in ('semantic_sha256', 'current_entries', 'previous_entries')):
                issues.append(f'pair {pair}: observer mismatch')
            try:
                values = {}
                for name, va, vb in (('elapsed_seconds', ma['elapsed_seconds'], mb['elapsed_seconds']),
                                     ('peak_rss_kib', ma['time']['peak_rss_kib'], mb['time']['peak_rss_kib']),
                                     ('app_disk_peak_bytes', ma['app_disk_peak_bytes'], mb['app_disk_peak_bytes'])):
                    if not number(va) or not number(vb):
                        raise ValueError('invalid measurement')
                    values[name] = {'baseline': va, 'candidate': vb, 'delta': vb - va}
                pairs.append({'pair': pair, **values})
            except (KeyError, TypeError, ValueError):
                issues.append(f'pair {pair}: missing measurements')
    summary = {}
    for name in ('elapsed_seconds', 'peak_rss_kib', 'app_disk_peak_bytes'):
        values = [row[name]['delta'] for row in pairs]
        if values:
            summary[name] = {'median_delta': statistics.median(values), 'max_abs_delta': max(map(abs, values))}
    identical = (isinstance(provenance.get(ARMS[0]), dict) and isinstance(provenance.get(ARMS[1]), dict)
                 and digest(provenance[ARMS[0]].get('sha256'))
                 and provenance[ARMS[0]].get('sha256') == provenance[ARMS[1]].get('sha256'))
    return {'valid': not issues, 'issues': issues, 'pairs': pairs, 'summary': summary,
            'identical_binaries': bool(identical), 'measurement_protocol': PROTOCOL}


def markdown(result):
    lines = [f'# {PROTOCOL}', '', 'Descriptive paired results; no automatic promotion or uncertainty estimate.',
             'Application disk peak is sampled; RSS excludes the observer.', '',
             '| Pair | Elapsed baseline → candidate (s) | RSS baseline → candidate (MiB) | App disk baseline → candidate (GiB) |',
             '|---:|---:|---:|---:|']
    for row in result['pairs']:
        def fmt(key, scale):
            x = row[key]
            return f"{x['baseline'] / scale:.3f} → {x['candidate'] / scale:.3f} (Δ {x['delta'] / scale:+.3f})"
        lines.append(f"| {row['pair']} | {fmt('elapsed_seconds', 1)} | {fmt('peak_rss_kib', 1024)} | {fmt('app_disk_peak_bytes', 1024 ** 3)} |")
    if result['identical_binaries'] and result['summary']:
        lines += ['', 'Identical binaries: observed maximum absolute paired differences (descriptive band):']
        for name, value in result['summary'].items():
            lines.append(f"- {name}: median Δ {value['median_delta']:+.3f}; max |Δ| {value['max_abs_delta']:.3f}")
    lines += ['', 'VALID' if result['valid'] else 'INVALID / INCOMPLETE']
    lines.extend(f'- {issue}' for issue in result['issues'])
    return '\n'.join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args(argv)
    result = summarize(args.directory)
    try:
        (args.directory / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
    except OSError as error:
        result['valid'] = False
        result['issues'].append(f'cannot write summary.json: {error}')
    print(markdown(result))
    return 0 if result['valid'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
