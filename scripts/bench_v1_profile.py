#!/usr/bin/env python3
"""Describe refresh allocation traces under profile-v2; never accept a product change."""
import argparse
import json
import math
import os
from pathlib import Path
import stat

from bench_v1_summary import ID, integer, read_json, summarize

POLICY = 'profile-v2'
LIMIT = 128 * 1024
WARNING = ('Boundary samples and buffer movement do not establish copying, physical overlap, '
           'or a phase peak. This description is not a product acceptance gate.')


def number(value):
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        return False
    try:
        return math.isfinite(value) and value >= 0
    except OverflowError:
        return False


def analyze_events(events, expected_entries, subtree_entries, removed):
    issues, anomalies, descriptions, name_changes = [], [], [], []
    if (not integer(expected_entries, True) or not integer(subtree_entries)
            or type(removed) is not bool or (removed and subtree_entries != 0)
            or (not removed and subtree_entries < 1)):
        issues.append('invalid expected entries or refresh outcome')
    if not isinstance(events, list) or any(not isinstance(e, dict) for e in events):
        return dict(valid_structure=False, issues=issues + ['events must be a list of objects'],
                    hwm_anomalies=[], original_hwm_monotonic_contract_pass=False,
                    node_capacity_changes=[], name_capacity_changes=[], events=events, warning=WARNING)
    phases = [e.get('phase') for e in events]
    required = ['refresh_start', 'refresh_loaded', 'refresh_before_graft']
    if removed is False:
        required += ['refresh_after_names', 'refresh_after_nodes']
    required += ['refresh_after_graft', 'refresh_after_finalize', 'index']
    if (any(not isinstance(p, str) or not p for p in phases)
            or len(set(p for p in phases if isinstance(p, str))) != len(phases)):
        issues.append('phase names must be nonempty and unique')
    if phases != required:
        issues.append('phase sequence differs from profile-v2')
    if removed is True and any(p in phases for p in ('refresh_after_names', 'refresh_after_nodes')):
        issues.append('removed subtree cannot have append phases')
    previous_time = previous_hwm = None
    for event in events:
        phase, elapsed = event.get('phase'), event.get('elapsed_seconds')
        rss, hwm = event.get('rss_kib'), event.get('hwm_kib')
        if not number(elapsed) or not integer(rss) or not integer(hwm):
            issues.append(f'{phase}: invalid elapsed, RSS or HWM')
        if number(elapsed):
            if previous_time is not None and elapsed < previous_time:
                issues.append(f'{phase}: elapsed time decreased')
            previous_time = elapsed
        if integer(hwm):
            if previous_hwm is not None and hwm < previous_hwm[1]:
                anomalies.append(dict(previous_phase=previous_hwm[0], phase=phase,
                                      previous_hwm_kib=previous_hwm[1], hwm_kib=hwm,
                                      delta_kib=hwm - previous_hwm[1]))
            previous_hwm = (phase, hwm)
    alloc_names = ['refresh_loaded', 'refresh_before_graft', 'refresh_after_names',
                   'refresh_after_nodes', 'refresh_after_graft', 'refresh_after_finalize']
    allocation = [e for e in events if e.get('phase') in alloc_names]
    previous = None
    for event in allocation:
        phase = event['phase']
        keys = ('node_len', 'node_capacity', 'name_len', 'name_capacity', 'node_size_bytes')
        if (any(not integer(event.get(k)) for k in keys) or not integer(event.get('node_size_bytes'), True)
                or (integer(event.get('node_len')) and integer(event.get('node_capacity'))
                    and event['node_capacity'] < event['node_len'])
                or (integer(event.get('name_len')) and integer(event.get('name_capacity'))
                    and event['name_capacity'] < event['name_len'])
                or type(event.get('node_buffer_changed')) is not bool
                or type(event.get('name_buffer_changed')) is not bool):
            issues.append(f'{phase}: invalid allocation fields')
        if previous is None and (event.get('node_buffer_changed') is not False
                                 or event.get('name_buffer_changed') is not False):
            issues.append(f'{phase}: first buffer change flags must be false')
        if previous is not None and integer(previous.get('node_capacity')) and integer(event.get('node_capacity')):
            if previous['node_capacity'] != event['node_capacity']:
                descriptions.append(dict(previous_phase=previous['phase'], phase=phase,
                                         before=previous['node_capacity'], after=event['node_capacity'],
                                         buffer_changed=event.get('node_buffer_changed')))
        if previous is not None and integer(previous.get('name_capacity')) and integer(event.get('name_capacity')):
            if previous['name_capacity'] != event['name_capacity']:
                name_changes.append(dict(previous_phase=previous['phase'], phase=phase,
                                         before=previous['name_capacity'], after=event['name_capacity'],
                                         buffer_changed=event.get('name_buffer_changed')))
        previous = event
    if integer(expected_entries, True) and integer(subtree_entries) and type(removed) is bool:
        for event in allocation:
            expected = expected_entries
            if not removed and event['phase'] in ('refresh_after_nodes', 'refresh_after_graft', 'refresh_after_finalize'):
                expected += max(0, subtree_entries - 1)
            if event.get('node_len') != expected:
                issues.append(f"{event['phase']}: unexpected node length")
    return dict(valid_structure=not issues, issues=issues, hwm_anomalies=anomalies,
                original_hwm_monotonic_contract_pass=not anomalies and all(integer(e.get('hwm_kib')) for e in events),
                node_capacity_changes=descriptions, name_capacity_changes=name_changes, events=events, warning=WARNING)


def read_trace(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise ValueError('trace must be a regular file')
        data = stream.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError('trace exceeds 128 KiB')
    def reject_constant(value):
        raise ValueError(f'nonfinite JSON number: {value}')
    def finite_float(value):
        parsed = float(value)
        if not math.isfinite(parsed):
            raise ValueError('nonfinite JSON float')
        return parsed
    def unique_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f'duplicate JSON key: {key}')
            result[key] = value
        return result
    return [json.loads(line, parse_constant=reject_constant, parse_float=finite_float,
                       object_pairs_hook=unique_pairs) for line in data.splitlines()]


def analyze(root):
    root = Path(root)
    summary = summarize(root)
    issues = []
    manifest = read_json(root / 'manifest.json', issues)
    ids = manifest.get('expected_arms')
    if not summary.get('valid'):
        issues.append('strict paired summary invalid')
    if manifest.get('operation') != 'refresh':
        issues.append('manifest operation must be refresh')
    policy = manifest.get('diagnostic_policy')
    if (not isinstance(policy, dict) or policy.get('allocation_diagnostics') is not True
            or policy.get('profile_policy') != POLICY):
        issues.append('manifest profile-v2 diagnostics not enabled')
    if (not isinstance(ids, list) or not ids or any(not isinstance(x, str) or not ID.fullmatch(x) for x in ids)
            or len(set(ids)) != len(ids)):
        issues.append('invalid manifest arm IDs')
        ids = []
    arms = {}
    for ident in ids:
        directory = root / ident
        if root.is_symlink() or directory.is_symlink() or not directory.is_dir():
            issues.append(f'{ident}: unsafe directory')
            continue
        spec = read_json(directory / 'spec.json', issues)
        metric = read_json(directory / 'metrics.json', issues)
        report = metric.get('report') if isinstance(metric.get('report'), dict) else {}
        if spec.get('allocation_diagnostics') is not True:
            issues.append(f'{ident}: allocation diagnostics not enabled')
        try:
            events = read_trace(directory / 'allocation-profile.jsonl')
            result = analyze_events(events, spec.get('expected_entries'), spec.get('expected_subtree_entries'), report.get('removed'))
            if (not integer(report.get('subtree_entries'))
                    or report.get('subtree_entries') != spec.get('expected_subtree_entries')):
                result['valid_structure'] = False
                result['issues'].append('reported subtree count differs from spec')
            peak = metric.get('time', {}).get('peak_rss_kib') if isinstance(metric.get('time'), dict) else None
            result['time_peak_rss_kib'] = peak if integer(peak, True) else None
            if not integer(peak, True):
                result['valid_structure'] = False
                result['issues'].append('invalid time peak RSS')
            arms[ident] = result
            if not result['valid_structure']:
                issues.append(f'{ident}: invalid trace structure')
        except (OSError, ValueError, UnicodeError, RecursionError, OverflowError) as error:
            issues.append(f'{ident}: cannot read trace: {error}')
    hwm_pass = (bool(ids) and len(arms) == len(ids)
                and all(arm['valid_structure'] and arm['original_hwm_monotonic_contract_pass']
                        for arm in arms.values()))
    return dict(policy=POLICY, valid_structure=not issues, issues=issues, strict_summary=summary,
                original_hwm_monotonic_contract_pass=hwm_pass, arms=arms, warning=WARNING)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--json-out', type=Path)
    args = parser.parse_args(argv)
    result = analyze(args.directory)
    rendered = json.dumps(result, indent=2, allow_nan=False) + '\n'
    if args.json_out:
        with args.json_out.open('x') as stream:
            stream.write(rendered)
    else:
        print(rendered, end='')
    return 0 if result['valid_structure'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
