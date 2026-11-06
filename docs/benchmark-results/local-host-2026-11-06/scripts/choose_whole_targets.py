#!/usr/bin/env python3
"""Pick refresh targets on the whole-disk coarse.db and predict each plan (region and fallback)."""
import json
import os
import sqlite3
import sys
from pathlib import Path

sys.path.insert(0, '/tmp/fsbench')
import local_host_bench as lhb  # noqa: E402

SNAP = Path('/tmp/fsbench/whole/probe/snapshot')
con = sqlite3.connect(f'file:{SNAP / "coarse.db"}?mode=ro&immutable=1', uri=True)
nodes = {bytes(p): dict(kind=k, entries=f + d + o, links=ln, allocated=a) for p, k, f, d, o, ln, a in
         con.execute('select path,kind,files,dirs,others,links,allocated from nodes')}
buckets = {}
for par, lo, hi, f, d, o, ln in con.execute('select parent,lo,hi,files,dirs,others,links from buckets order by parent,lo'):
    buckets.setdefault(bytes(par), []).append(dict(lo=lo, hi=hi, entries=f + d + o, links=ln))


def plan_of(target):
    """The region the probe would rescan for an existing target, and whether it falls back."""
    raw = os.fsencode(os.path.realpath(target))
    if raw in nodes:
        n = nodes[raw]
        return dict(plan='retained', region_entries=n['entries'], links=n['links'], fallback=n['links'] > 0 or raw == b'/')
    parts = raw.split(b'/')
    for cut in range(len(parts) - 1, 0, -1):
        anc = b'/'.join(parts[:cut]) or b'/'
        if anc in nodes:
            child = parts[cut]
            slot = lhb.slot_of(child)
            b = next(x for x in buckets[anc] if x['lo'] <= slot < x['hi'])
            return dict(plan='range', ancestor=anc.decode(errors='replace'), region_entries=b['entries'], links=b['links'], fallback=b['links'] > 0)
    return dict(plan='unknown', fallback=True)


picks = lhb.pick_targets('/', SNAP)
picks.pop('sizes', None)
# Host-specific inputs come from the environment (the committed copy carries no private paths):
#   FS_BENCH_CALIBRATION_DIR  a small source folder, the one the Oct 31 v1 refresh calibration used
#   FS_BENCH_JOBS_DIR         a folder of many similar checkouts; its median-sized retained, hardlink-free child is picked
calibration = os.environ.get('FS_BENCH_CALIBRATION_DIR', '')
if calibration and Path(calibration).is_dir():
    picks = {'calibration-dir': calibration, **picks}
# a real cleanup candidate: one checkout of the many (retained, hardlink-free)
jobs = Path(os.environ.get('FS_BENCH_JOBS_DIR', '/nonexistent'))
if jobs.is_dir():
    cands = sorted(((nodes[os.fsencode(str(p))]['entries'], str(p)) for p in jobs.iterdir() if os.fsencode(str(p)) in nodes and nodes[os.fsencode(str(p))]['links'] == 0))
    if cands:
        picks['job-checkout'] = cands[len(cands) // 2][1]
report = {}
for name, target in picks.items():
    if isinstance(target, str) and Path(target).exists():
        report[name] = dict(target=target, **plan_of(target))
    else:
        report[name] = dict(target=target)
Path('/tmp/fsbench/whole-targets.json').write_text(json.dumps({k: v.get('target') for k, v in report.items()}, indent=1))
Path('/tmp/fsbench/results/whole-targets-plans.json').write_text(json.dumps(report, indent=1))
for k, v in report.items():
    print(f'{k:18} {v}')
