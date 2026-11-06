#!/usr/bin/env python3
"""Refresh memory under cgroup caps: v1 (loads and rewrites the whole database) against the probe. Run as root.

Builds one snapshot per arm on the 1M-entry fixture (no cap), then refreshes one folder of it under each cap
from a cold remount, on a fresh copy of the snapshot every time so that a killed run cannot damage the next one.
Reuses the sampler of mem_ladder.py.
"""
import json
import os
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

import mem_ladder as ml

MNT = '/tmp/fsbench/loop-mnt'
IMAGE = '/tmp/fsbench/loop.img'
TREE = f'{MNT}/A'
TARGET = f'{MNT}/A/d0042'
RESULTS = ml.BASE / 'results' / 'memory-refresh.jsonl'
STAGE = ml.WORK / 'refresh'


def remount():
    subprocess.run(['umount', MNT], check=True)
    subprocess.run(['mount', '-o', 'loop,noatime', IMAGE, MNT], check=True)


def v1_env(snap):
    return dict(os.environ, LC_ALL='C', FREESPACE_CONFIG=str(ml.BASE / 'config.toml'), FREESPACE_SNAPSHOT_DIR=str(snap), SQLITE_TMPDIR=str(snap / 'tmp'))


def commands(arm, snap):
    if arm == 'probe':
        scan = [str(ml.PROBE), 'scan', '--root', TREE, '--snapshot', str(snap), '--ppm', '1000']
        refresh = [str(ml.PROBE), 'refresh', '--root', TREE, '--snapshot', str(snap), '--target', TARGET]
        return scan, refresh, dict(os.environ, LC_ALL='C')
    scan = [ml.INSTALLED, '--backend', 'v1', 'scan', TREE, '--low-memory', '--workers', '1', '--json']
    refresh = [ml.INSTALLED, '--backend', 'v1', 'refresh', TARGET, '--workers', '1', '--json']
    return scan, refresh, v1_env(snap)


def run_scoped(argv, env, cap, tag):
    ml.COUNTER[0] += 1
    unit = f'fsbench-mem-{os.getpid()}-{ml.COUNTER[0]}'
    cmd = ['systemd-run', '--scope', '--quiet', '--collect', '--slice=fsbench.slice', f'--unit={unit}', '-p', 'MemoryAccounting=yes',
           '-p', 'MemorySwapMax=0', '-p', f'MemoryMax={cap}', '--', 'nice', '-n', '10', 'ionice', '-c2', '-n7', *argv]
    state = dict(peak=0, at_peak={}, events={}, reclaim={}, error=None, memory_max=None)
    stop = threading.Event()
    out = STAGE / f'{tag}.out'
    t0 = time.monotonic()
    with out.open('wb') as so, (STAGE / f'{tag}.err').open('wb') as se:
        proc = subprocess.Popen(cmd, stdout=so, stderr=se, env=env, start_new_session=True)
    thread = threading.Thread(target=ml.sampler, args=(proc.pid, unit, state, stop, lambda: os.killpg(proc.pid, 9)))
    thread.start()
    _, status, usage = os.wait4(proc.pid, 0)
    wall = time.monotonic() - t0
    stop.set()
    thread.join()
    code = os.waitstatus_to_exitcode(status)
    return dict(exit_code=code, ok=(code == 0 and state['error'] is None), wall_s=wall, max_rss_kib=usage.ru_maxrss,
                cgroup_peak_sampled=state['peak'], at_peak=state['at_peak'], events=state['events'], reclaim=state['reclaim'],
                stderr_tail=(STAGE / f'{tag}.err').read_text(errors='replace')[-300:], at=time.strftime('%F %T%z'))


def main():
    caps = {'v1': ['6G', '512M', '256M', '192M', '128M', '96M'], 'probe': ['6G', '256M', '128M', '64M', '48M']}
    shutil.rmtree(STAGE, ignore_errors=True)
    STAGE.mkdir(parents=True)
    for arm in ('v1', 'probe'):
        base = STAGE / f'{arm}-base'
        (base / 'tmp').mkdir(parents=True)
        scan, _, env = commands(arm, base)
        remount()
        rec = run_scoped(scan, env, ml.parse_cap('6G'), f'{arm}-scan')
        print(arm, 'scan', 'ok' if rec['ok'] else 'FAILED', f"rss={rec['max_rss_kib'] / 1024:.1f}MiB wall={rec['wall_s']:.1f}s", flush=True)
        if not rec['ok']:
            return 2
        for cap_text in caps[arm]:
            cap = ml.parse_cap(cap_text)
            run_dir = STAGE / f'{arm}-run'
            shutil.rmtree(run_dir, ignore_errors=True)
            shutil.copytree(base, run_dir, symlinks=True)
            _, refresh, env = commands(arm, run_dir)
            remount()
            rec = run_scoped(refresh, env, cap, f'{arm}-refresh-{cap_text}')
            rec.update(arm=arm, op='refresh', target=TARGET, cap_bytes=cap, cold=True, entries_in_snapshot=1004501)
            with RESULTS.open('a') as f:
                f.write(json.dumps(rec) + '\n')
            print(f"{arm:6} refresh cap={cap_text:>4} ok={rec['ok']!s:5} exit={rec['exit_code']:3} wall={rec['wall_s']:6.1f}s rss={rec['max_rss_kib'] / 1024:7.1f}MiB "
                  f"cgroup_peak={rec['cgroup_peak_sampled'] / 1048576:7.1f}MiB limit_hits={rec['events'].get('max')}", flush=True)
            shutil.rmtree(run_dir, ignore_errors=True)
            if not rec['ok']:
                print('   FAILED:', rec['stderr_tail'].strip()[-160:], flush=True)
                break
        shutil.rmtree(base, ignore_errors=True)
    shutil.rmtree(STAGE, ignore_errors=True)
    return 0


if __name__ == '__main__':
    if os.geteuid() != 0:
        sys.exit('run as root (sudo -n)')
    sys.exit(main())
