#!/usr/bin/env python3
"""Print the memory tables from memory.jsonl (mem_ladder.py) and memory-refresh.jsonl (mem_refresh.py).

  summarize_memory.py results/memory.jsonl [results/memory-refresh.jsonl]

Values of max RSS at or below FLOOR_MIB are shown as "<=": the harness is a Python process of about 12-13 MiB, and
Linux keeps the larger of the pre-exec parent's peak and the new program's, so a small program cannot read lower.
"""
import json
import sys
from pathlib import Path

MIB = 1 << 20
FLOOR_MIB = 13.2
CLEAN_PROBE_BASE_MIB = 8.62  # results/memory/clean-rss-usr.json: probe scan of /usr under /usr/bin/time, no Python parent


def load(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def fixture(rec):
    return rec['tree'].rstrip('/').rsplit('/', 1)[-1]


def rss(rec):
    mib = rec['max_rss_kib'] / 1024
    return f'<= {mib:.1f} MiB (floor)' if mib <= FLOOR_MIB else f'{mib:.1f} MiB'


def label(cap_bytes):
    cap = cap_bytes // MIB
    return f'{cap // 1024} GiB' if cap % 1024 == 0 else f'{cap} MiB'


def reference(rows):
    print('Uncapped reference (cap 6 GiB), cold caches')
    print('| fixture | arm | visits | max RSS | cgroup peak | anon | file cache | slab (reclaimable) | wall | snapshot |')
    print('| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |')
    for r in rows:
        if r['cap_bytes'] != 6 << 30 or not r['ok']:
            continue
        p = r['at_peak']
        print(f"| {fixture(r)} | {r['arm']} | {r['entries']:,} | {rss(r)} | {r['cgroup_peak_sampled'] / MIB:,.0f} MiB | "
              f"{p['anon'] / MIB:.0f} | {p['file'] / MIB:,.0f} | {p['slab_reclaimable'] / MIB:,.0f} | {r['wall_s']:.1f} s | {r['snapshot_allocated'] / MIB:.1f} MiB |")


def ladders(rows):
    print('\nCap ladder (memory.max, swap off, cold caches before every run); the cap counts the process, its page cache and its kernel slab')
    print('| fixture | arm | cap | result | wall | max RSS | times the limit was hit | pages reclaimed (4 KiB) |')
    print('| --- | --- | --- | --- | --- | --- | --- | --- |')
    for r in rows:
        if r['cap_bytes'] == 6 << 30:
            continue
        result = 'completed' if r['ok'] else f"killed (exit {r['exit_code']})"
        wall = f"{r['wall_s']:.1f} s" if r['ok'] else f"{r['wall_s']:.1f} s (to the kill)"
        print(f"| {fixture(r)} | {r['arm']} | {label(r['cap_bytes'])} | {result} | {wall} | {rss(r)} | {r['events'].get('max'):,} | {r['reclaim'].get('pgsteal'):,} |")


def refresh(rows):
    print('\nRefresh of one folder of the 1,004,501-entry snapshot, cap ladder, cold caches')
    print('| arm | cap | result | wall | max RSS | anon at the peak sample | cgroup peak (sampled) |')
    print('| --- | --- | --- | --- | --- | --- | --- |')
    for r in rows:
        result = 'completed' if r['ok'] else f"killed by the OOM killer (exit {r['exit_code']})"
        print(f"| {r['arm']} | {label(r['cap_bytes'])} | {result} | {r['wall_s']:.1f} s | {rss(r)} | {r['at_peak']['anon'] / MIB:.0f} MiB | "
              f"{r['cgroup_peak_sampled'] / MIB:,.0f} MiB |")


def owners(rows):
    a = [r for r in rows if fixture(r) == 'A' and r['arm'] == 'probe' and r['ok']]
    b = [r for r in rows if fixture(r) == 'B' and r['arm'] == 'probe' and r['ok']]
    if not (a and b):
        return
    with_links = min(r['max_rss_kib'] for r in b) / 1024
    floor_clamped = min(r['max_rss_kib'] for r in a) / 1024
    print(f'\nHardlink owners (300,000 distinct hardlinked inodes, paths of about 94 bytes): probe max RSS {with_links:.1f} MiB on B.')
    print(f'  Per inode: {(with_links - CLEAN_PROBE_BASE_MIB) * MIB / 300000:.0f} bytes against the clean {CLEAN_PROBE_BASE_MIB} MiB baseline, '
          f'{(with_links - floor_clamped) * MIB / 300000:.0f} bytes against the floor-clamped {floor_clamped:.1f} MiB reading on A (a lower bound).')


def main():
    rows = [r for r in load(sys.argv[1]) if 'loop-mnt' in r['tree']]
    reference(rows)
    ladders(rows)
    if len(sys.argv) > 2:
        refresh(load(sys.argv[2]))
    owners(rows)


main()
