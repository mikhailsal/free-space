#!/usr/bin/env python3
"""Summarise the local-host benchmark JSONL files into the tables used by the report.

Usage: summarize.py cargo|whole [jsonl]
Junk records (the duplicate launch of the probe scan: exit_code != 0 and wall < 1 s with no error text of a
real run) are dropped by the rule `exit_code != 0` for scans, and reported separately.
"""
import json
import statistics
import sys
from collections import defaultdict

MIB = 1048576


def load(path):
    out = []
    for line in open(path, encoding='utf-8'):
        line = line.strip()
        if line:
            out.append(json.loads(line))
    return out


def med(xs):
    return statistics.median(xs) if xs else float('nan')


def row(label, recs):
    ok = [r for r in recs if r.get('exit_code') == 0 and not r.get('error')]
    bad = len(recs) - len(ok)
    if not ok:
        return f'{label:28} no successful run ({bad} failed)'
    rep = ok[0].get('report') or {}
    entries = rep.get('entries') or rep.get('visited')
    walls = [r['wall_s'] for r in ok]
    cpu = [r['user_s'] + r['sys_s'] for r in ok]
    return (f'{label:28} n={len(ok)}{"+%d bad" % bad if bad else "":7} wall med={med(walls):9.2f}s '
            f'[{min(walls):8.2f}..{max(walls):8.2f}] cpu={med(cpu):8.1f}s rss={med([r["max_rss_kib"] for r in ok]) / 1024:8.1f}MiB '
            f'db={med([r["final_allocated"] for r in ok]) / MIB:10.2f}MiB peak={med([r["peak_allocated"] for r in ok]) / MIB:10.2f}MiB '
            f'entries={entries}')


def main():
    kind = sys.argv[1]
    path = sys.argv[2] if len(sys.argv) > 2 else f'/tmp/fsbench/results/{kind}.jsonl'
    recs = [r for r in load(path) if r.get('wall_s') is not None]
    groups = defaultdict(list)
    for r in recs:
        groups[(r.get('phase'), r.get('op'), r.get('scenario'), r.get('arm'))].append(r)
    for key in sorted(groups, key=lambda k: tuple(str(x) for x in k)):
        phase, op, scenario, arm = key
        if phase == 'seed':
            continue
        print(row(f'{phase}/{scenario or "-"}/{arm}', groups[key]))


if __name__ == '__main__':
    main()
