#!/usr/bin/env python3
"""Check on /usr that the hardlink fallback of a probe refresh is predicted by the stored coarse.db.

For a set of target directories the stored snapshot predicts whether the planned region holds a hardlinked
entry (then the probe rescans the whole root). Every target is refreshed on a fresh copy of the same
snapshot and the report's `fallback_full_root` and `visited` are compared with the prediction.
"""
import json
import os
import random
import shutil
import sqlite3
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, '/tmp/fsbench')
import local_host_bench as lhb  # noqa: E402

PROBE = '/tmp/fsbench/probe-target/x86_64-unknown-linux-musl/release/cleanup_probe_live'
SEED = Path('/tmp/fsbench/diff/usr-live')
WORK = Path('/tmp/fsbench/work/usr-fallback')
OUT = Path('/tmp/fsbench/results/usr-fallback-check.json')

con = sqlite3.connect(f'file:{SEED / "coarse.db"}?mode=ro&immutable=1', uri=True)
nodes = {bytes(p): dict(entries=f + d + o, links=ln) for p, f, d, o, ln in con.execute('select path,files,dirs,others,links from nodes')}
buckets = {}
for par, lo, hi, f, d, o, ln in con.execute('select parent,lo,hi,files,dirs,others,links from buckets order by parent,lo'):
    buckets.setdefault(bytes(par), []).append(dict(lo=lo, hi=hi, entries=f + d + o, links=ln))


def plan_of(target):
    raw = os.fsencode(os.path.realpath(target))
    if raw in nodes:
        n = nodes[raw]
        return dict(plan='retained', region_entries=n['entries'], links=n['links'], fallback=n['links'] > 0 or raw == b'/usr')
    parts = raw.split(b'/')
    for cut in range(len(parts) - 1, 0, -1):
        anc = b'/'.join(parts[:cut]) or b'/'
        if anc in nodes:
            slot = lhb.slot_of(parts[cut])
            b = next(x for x in buckets[anc] if x['lo'] <= slot < x['hi'])
            return dict(plan='range', region_entries=b['entries'], links=b['links'], fallback=b['links'] > 0)
    return dict(plan='unknown', fallback=True)


def candidates():
    linked, plain = set(), []
    out = subprocess.run(['find', '/usr', '-xdev', '-type', 'f', '-links', '+1', '-printf', '%h\\n'], capture_output=True, text=True).stdout
    linked.update(out.split('\n'))
    linked.discard('')
    dirs = subprocess.run(['find', '/usr', '-xdev', '-mindepth', '2', '-type', 'd', '-printf', '%p\\n'], capture_output=True, text=True).stdout.split('\n')
    for d in dirs:
        if d and d not in linked:
            plain.append(d)
    return sorted(linked), plain


def refresh(target, tag):
    snap = WORK / tag
    if snap.exists():
        shutil.rmtree(snap)
    shutil.copytree(SEED, snap)
    p = subprocess.run([PROBE, 'refresh', '--root', '/usr', '--snapshot', str(snap), '--target', target], capture_output=True, text=True)
    try:
        rep = json.loads(p.stdout)
    except json.JSONDecodeError:
        rep = None
    return p.returncode, rep, p.stderr[-300:]


def main():
    random.seed(20261106)
    WORK.mkdir(parents=True, exist_ok=True)
    linked, plain = candidates()
    want_fb = [t for t in random.sample(linked, min(len(linked), 400)) if plan_of(t)['fallback']][:3]
    want_ok = [t for t in random.sample(plain, min(len(plain), 400)) if not plan_of(t)['fallback']][:3]
    results = []
    for i, t in enumerate(want_fb + want_ok):
        pred = plan_of(t)
        code, rep, err = refresh(t, f't{i}')
        actual = (rep or {}).get('fallback_full_root')
        row = dict(target=t, predicted=pred, exit_code=code, actual_fallback=actual, visited=(rep or {}).get('visited'),
                   scope=(rep or {}).get('scope'), elapsed_seconds=(rep or {}).get('elapsed_seconds'), match=(actual == pred['fallback']), stderr=err)
        results.append(row)
        print(json.dumps({k: v for k, v in row.items() if k != 'stderr'}), flush=True)
    summary = dict(targets=len(results), all_match=all(r['match'] for r in results), hardlinked_candidates=len(linked), plain_candidates=len(plain), results=results)
    OUT.write_text(json.dumps(summary, indent=1))
    print('all predictions match:', summary['all_match'])
    shutil.rmtree(WORK, ignore_errors=True)


if __name__ == '__main__':
    main()
