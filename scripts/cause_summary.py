#!/usr/bin/env python3
"""Validate causal diagnostic evidence; emit descriptive comparisons, never promotion."""
import argparse
import json
from pathlib import Path
import statistics
from cause_outcomes import evaluate

from cause_validate import (PROTOCOL, cold_and_io_ok, disk_ok, docker_ok, global_ok, lifetime_ok, load, manifest_ok,
                            memory_ok, need, report_ok, sampler_ok, spec_ok)


def case_result(root, manifest, row):
    ident = row['id']
    need(row['spec_path'] == ident + '.json', 'spec path')
    spec = load(root, row['spec_path'])
    spec_ok(spec, row, manifest)
    receipts = {name: load(root, ident + '/' + name + '.json') for name in
                ('metrics', 'host', 'worker-ready', 'worker-terminal', 'host-ack')}
    for data in receipts.values():
        need(all(data[k] == spec[k] for k in ('protocol', 'id', 'nonce')), 'receipt identity')
    metrics, host, ready, terminal = [receipts[k] for k in ('metrics', 'host', 'worker-ready', 'worker-terminal')]
    need(metrics['validated'] is True and metrics['cli_validated'] is True and not metrics['errors']
         and metrics['capture_errors'] == 0 and metrics['qualification'] is False
         and metrics['exit_code'] == 0 and metrics['timed_out'] is False, 'worker measurement failure')
    need(ready['status'] == 'ready' and ready['cli_validated'] is True and ready['kernel_peak_frozen_in'] == 'metrics.json'
         and terminal['validated'] is True and terminal['acknowledged'] is True and terminal['ack_status'] == 'matched'
         and terminal['success'] is True and not terminal['errors'], 'acknowledgement/terminal failure')
    need(host['validated'] is True and host['container_cleanup'] is True and not host['errors']
         and host['pin'] is row['pin'], 'host/cleanup failure')
    need(metrics['mode'] == spec['mode'] and metrics['sampler'] == spec['sampler'], 'metrics mode')
    stats = memory_ok(metrics)
    sampler_ok(metrics, spec)
    report_ok(metrics, spec, manifest['fixture_receipt'])
    cold_and_io_ok(root, ident, metrics, spec)
    need(load(root, ident + '/stdout.json') == metrics['stdout'] and load(root, ident + '/time.log') == metrics['gnu_time'], 'raw CLI report differs')
    docker_ok(host, spec)
    for phase, count in (('before', 0), ('after', 1)):
        other = load(root, f'{ident}/foreign-containers-{phase}.json')
        need(other['exclusive'] is True and not other['foreign'] and other['running_count'] == count,
             'unexpected running Docker container')
    globals_ = []
    for phase in ('before', 'after'):
        relative = f'{ident}/global-{phase}/global.json'
        need(host['global_' + phase] == relative, 'global receipt path')
        data = load(root, relative)
        global_ok(data)
        globals_.append(data)
    need(globals_[0]['after']['monotonic_ns'] <= metrics['cli_start_ns']
         and metrics['cli_end_ns'] <= ready['ready_ns'] <= globals_[1]['before']['monotonic_ns']
         <= globals_[1]['after']['monotonic_ns'] <= terminal['terminal_ns'], 'host/CLI time boundaries')
    available = globals_[0]['sources']['meminfo']['parsed']['fields']['MemAvailable']
    need(available['unit'] == 'kB' and available['value'] * 1024 >= 5 * 2**30
         and globals_[0]['background']['sources']['memory']['parsed']['some']['avg10'] <= 1.0, 'host pre-case pressure')
    if spec['mode'] == 'scan':
        observer = load(root, ident + '/observer.json')
        need(observer['validated'] is True and not observer['errors'] and observer['logical_content_unchanged'] is True
             and observer['seed_sha256'] == spec['seed_sha256']
             and observer['current_entries'] == observer['previous_entries'] == spec['expected_entries'], 'semantic observer failure')
    else:
        observer = None
        seed = load(root, ident + '/seed-integrity.json')
        need(seed['unchanged'] is True and seed['expected_seed_sha256'] == seed['observed_seed_sha256']
             == spec['seed_sha256'], 'probe modified snapshot seed')
    disk = lifetime = None
    if row['pin']:
        need(host['disk_path'] == ident + '/disk.json', 'disk receipt path')
        disk = load(root, host['disk_path'])
        disk_ok(disk)
        need(host['lifetime_path'] == ident + '/lifetime.json', 'lifetime receipt path')
        lifetime = load(root, host['lifetime_path'])
        lifetime_ok(lifetime, host)
    return dict(id=ident, mode=row['mode'], sampler=row['sampler'], group=row['group'], round=row.get('round'),
                diagnostic_pin=row['pin'], elapsed_seconds=metrics['elapsed_seconds'],
                kernel_memory_peak_bytes=metrics['kernel_memory_peak_bytes'],
                peak_scope=metrics['kernel_peak_scope'], peak_rss_kib=metrics['gnu_time']['peak_rss_kib'],
                frozen_post_cli_memory_stat=stats, sampling=metrics['sampling'],
                global_slab_delta=slab_delta(*globals_),
                path_kind_digest=metrics['stdout'].get('path_kind_digest'),
                semantic_sha256=observer['semantic_sha256'] if observer else None, disk=disk, lifetime=lifetime)


def slab_delta(before, after):
    def rows(data):
        return {r['name']: r for r in data['sources']['slabinfo']['parsed']['objects']}
    a, b = rows(before), rows(after)
    result = {}
    for name in sorted(a.keys() & b.keys()):
        if name == 'dentry' or 'inode' in name:
            result[name] = {key: b[name][key] - a[name][key] for key in
                            ('active_objects', 'total_objects', 'total_slab_pages') if key in a[name] and key in b[name]}
    return dict(scope='GLOBAL host context, not cgroup object attribution', caches=result)


def differences(a, b):
    result = {key: b[key] - a[key] for key in ('elapsed_seconds', 'kernel_memory_peak_bytes', 'peak_rss_kib')}
    result['post_cli_category_deltas'] = {key: b['frozen_post_cli_memory_stat'][key] - a['frozen_post_cli_memory_stat'][key]
                                        for key in ('anon', 'file', 'kernel', 'slab_reclaimable', 'slab_unreclaimable')}
    return result


def comparisons(rows, stage):
    result = dict(ram=[], collector=[], limitations=[
        'Exploratory counterbalanced screen; no population confidence interval or product qualification.',
        'stat minus names tests added metadata calls; scan minus stat is not an additive SQLite/heap decomposition.',
        'Post-CLI categories are sequential endpoint observations, not a decomposition of the true kernel peak.',
        'Pinned disk pass deliberately extends inode lifetimes and cannot qualify ordinary RAM/time/peak disk.',
        'Missed/late 20-ms ticks are descriptive; reserve coverage requires maximum 250-ms check gaps.'])
    if stage == 'screen':
        for n in range(1, 5):
            group = {r['mode']: r for r in rows if r['group'] == 'latin' and r['round'] == n}
            result['ram'].append(dict(round=n, stat_minus_names=differences(group['names'], group['stat']),
                                      scan_minus_stat=differences(group['stat'], group['scan'])))
        for n in (1, 2, 3):
            group = {r['sampler']: r for r in rows if r['group'] == 'collector' and r['round'] == n}
            result['collector'].append(dict(round=n, full_minus_reserve=differences(group['reserve'], group['full'])))
    else:
        group = {r['mode']: r for r in rows if r['group'] == 'smoke'}
        result['ram'].append(dict(stat_minus_names=differences(group['names'], group['stat'])))
    result['by_mode_medians'] = {}
    for mode in ('idle', 'names', 'stat', 'scan'):
        group = [r for r in rows if r['mode'] == mode and not r['diagnostic_pin'] and r['group'] in ('latin', 'smoke')]
        if group:
            result['by_mode_medians'][mode] = {key: statistics.median(r[key] for r in group) for key in
                                              ('elapsed_seconds', 'kernel_memory_peak_bytes', 'peak_rss_kib')}
    return result


def summarize(root, require_cleanup=False):
    root = Path(root)
    issues, rows, manifest, source_valid = [], [], None, False
    try:
        manifest = load(root, 'manifest.json')
        manifest_ok(root, manifest)
        source_valid = True
        status = load(root, 'status.json')
        need(status['success'] is True and status['completed'] is True and status['cleanup_success'] is True
             and not status['failures'], 'host terminal status')
        if require_cleanup:
            cleanup = load(root, 'runner-cleanup.json')
            need(cleanup['success'] is True and cleanup['exit_code'] == 0, 'runner cleanup failure')
    except (OSError, ValueError, KeyError, TypeError) as error:
        issues.append('campaign: ' + str(error))
    if source_valid:
        for row in manifest['cases']:
            try:
                rows.append(case_result(root, manifest, row))
            except (OSError, ValueError, KeyError, TypeError) as error:
                issues.append(row.get('id', '?') + ': ' + str(error))
        probes = {r['path_kind_digest'] for r in rows if r['mode'] in ('names', 'stat')}
        semantics = {r['semantic_sha256'] for r in rows if r['mode'] == 'scan'}
        if len(probes) != 1:
            issues.append('probe path/type multiset digests differ or missing')
        if len(semantics) != 1:
            issues.append('scan semantic digests differ or missing')
    valid = not issues and require_cleanup
    digest_valid = bool(rows) and len({r['path_kind_digest'] for r in rows if r['mode'] in ('names', 'stat')}) == 1
    digest_valid &= len({r['semantic_sha256'] for r in rows if r['mode'] == 'scan'}) == 1
    groups = {}
    if source_valid:
        for group, accepted in (('ram', ('latin', 'smoke')), ('collector', ('collector',)), ('disk', ('disk',))):
            expected = [r['id'] for r in manifest['cases'] if r['group'] in accepted]
            found = [r['id'] for r in rows if r['group'] in accepted]
            groups[group] = dict(valid=bool(expected) and found == expected and digest_valid, expected=expected, observed=found,
                                 scope='Receipt-valid diagnostic group; overall campaign/cleanup remains separate.')
    return dict(protocol=PROTOCOL, valid=valid, qualification=False,
                stage=manifest.get('stage') if manifest else None,
                status='valid-diagnostic' if valid else 'invalid' if issues else 'pending-runner-cleanup',
                issues=issues, rows=rows, groups=groups,
                comparisons=comparisons(rows, manifest['stage']) if valid else None,
                outcomes=evaluate(rows, manifest['stage']) if valid else None)


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('root', type=Path)
    parser.add_argument('--require-cleanup', action='store_true')
    args = parser.parse_args(argv)
    result = summarize(args.root, args.require_cleanup)
    args.root.mkdir(parents=True, exist_ok=True)
    (args.root / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k: v for k, v in result.items() if k not in ('rows', 'comparisons')}))
    return 0 if result['valid'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
