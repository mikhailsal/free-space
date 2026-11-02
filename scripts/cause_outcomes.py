"""Prospectively fixed exploratory mechanism rules; not confidence intervals."""
MIB = 2**20
POLICY = 'resource-cause-outcomes-v1'


def spread(values):
    return max(values) - min(values)


def evaluate(rows, stage):
    if stage != 'screen':
        return dict(policy=POLICY, status='smoke-ineligible', qualification=False)
    latin = {(r['round'], r['mode']): r for r in rows if r['group'] == 'latin'}
    if len(latin) != 16:
        return dict(policy=POLICY, status='incomplete-ram-group', qualification=False)
    key = 'slab_reclaimable'
    slab = lambda r: r['frozen_post_cli_memory_stat'][key]
    peak = lambda r: r['kernel_memory_peak_bytes']
    rounds = range(1, 5)
    delta_slab = [slab(latin[n, 'stat']) - slab(latin[n, 'names']) for n in rounds]
    delta_peak = [peak(latin[n, 'stat']) - peak(latin[n, 'names']) for n in rounds]
    noise_slab = max(spread([slab(latin[n, m]) for n in rounds]) for m in ('names', 'idle'))
    noise_peak = max(spread([peak(latin[n, m]) for n in rounds]) for m in ('names', 'idle'))
    floors = dict(slab=max(64*MIB, noise_slab), peak=max(64*MIB, noise_peak))
    if min(delta_slab) > floors['slab'] and min(delta_peak) > floors['peak']:
        status = 'supported'
    elif max(map(abs, delta_slab + delta_peak)) <= 16*MIB and max(noise_slab, noise_peak) <= 16*MIB:
        status = 'rejected'
    else:
        status = 'unresolved'
    collector = collector_rule(rows)
    ratios = [dict(round=n, slab=slab(latin[n, 'stat']) / max(1, slab(latin[n, 'scan'])),
                   peak=peak(latin[n, 'stat']) / peak(latin[n, 'scan'])) for n in rounds]
    comparable = (status == 'supported' and collector['status'] == 'within-exploratory-band'
                  and all(r['slab'] >= .75 and r['peak'] >= .75 for r in ratios))
    return dict(policy=POLICY, qualification=False, primary_metadata_effect=status,
                claim='Added explicit metadata lookups are sufficient to create a material extra charge in this probe/environment.',
                slab_delta_bytes=delta_slab, kernel_peak_delta_bytes=delta_peak,
                control_cross_round_ranges_bytes=dict(slab=noise_slab, peak=noise_peak),
                support_floors_bytes=floors, collector=collector,
                stat_to_scan_ratios=ratios,
                metadata_scan_comparability='supported' if comparable else 'unresolved',
                interpretation='supported requires all4 deltas in BOTH primary measures above64MiB and control ranges; rejected means all absolute deltas and control ranges<=16MiB, rejecting only the proposed material effect in this screen; otherwise unresolved. Comparable requires each stat/scan ratio>=0.75 and collector within band. No additive decomposition or population confidence claim.')


def collector_rule(rows):
    collector = {(r['round'], r['sampler']): r for r in rows if r['group'] == 'collector'}
    if len(collector) != 6:
        return dict(status='incomplete-collector-group')
    receipts = []
    for n in (1, 2, 3):
        full, reserve = collector[n, 'full'], collector[n, 'reserve']
        bounds = dict(kernel_memory_peak_bytes=max(16*MIB, reserve['kernel_memory_peak_bytes']*.02),
                      peak_rss_kib=1024, elapsed_seconds=max(.5, reserve['elapsed_seconds']*.05))
        deltas = {k: full[k] - reserve[k] for k in bounds}
        receipts.append(dict(round=n, deltas=deltas, absolute_bounds=bounds,
                             within=all(abs(deltas[k]) <= bounds[k] for k in bounds)))
    return dict(status='within-exploratory-band' if all(r['within'] for r in receipts) else 'unresolved',
                pairs=receipts, scope='Empirical3-pair descriptive collector bound at this scale/cap, not a confidence interval.')
