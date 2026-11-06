#!/usr/bin/env python3
"""What "scan memory" is, and the smallest memory cap a scan still completes under. Run as root.

Every command runs in its own transient systemd scope with MemoryMax=<cap> and MemorySwapMax=0, so an
overflow is an OOM kill inside that scope and nothing else on the host is touched. Three things are recorded
for each run:
  * max RSS of the command itself (wait4 rusage: the number the earlier tables call "Max RSS"),
  * the highest sample (every 20 ms) of the scope's memory.current, which also counts page cache and kernel
    slab charged to the scope, and the memory.stat split at that sample,
  * whether the run completed (exit status, memory.events oom_kill).
The kernel here is 5.15, which has no memory.peak, so the cgroup peak is a sampled lower bound.

Reading max_rss_kib: this harness is a Python process of about 10.5-13 MiB and Linux keeps the larger of the parent's
peak and the exec'd program's, so a value at or below about 13 MiB is a floor, not a measurement (see
results/memory/clean-rss-usr.json for a clean reading of the probe). Whether a run completed or was killed, and
every value above the floor, are not affected.

  mem_ladder.py --tree /usr --arms probe,v1-low --caps 4G,512M,128M
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

BASE = Path('/tmp/fsbench')
WORK = BASE / 'memtest'
RESULTS = BASE / 'results' / 'memory.jsonl'
PROBE = BASE / 'probe-target/x86_64-unknown-linux-musl/release/cleanup_probe_live'
INSTALLED = '/usr/local/bin/freespace'
GUARD_START = 3 << 30  # refuse to start a run below this much free space
GUARD_ABORT = 2560 << 20  # kill a running command below this
UNITS = {'K': 1 << 10, 'M': 1 << 20, 'G': 1 << 30}
COUNTER = [0]
STAT_KEYS = ('anon', 'file', 'file_mapped', 'slab_reclaimable', 'slab_unreclaimable', 'kernel_stack', 'pagetables', 'sock', 'shmem')
RECLAIM_KEYS = ('pgscan', 'pgsteal', 'pgmajfault', 'workingset_refault_anon', 'workingset_refault_file')
COLD = {}  # {'image': ..., 'mount': ...}: remount before each run, which drops that filesystem's dentries and inodes


def parse_cap(text):
    return int(text[:-1]) * UNITS[text[-1].upper()]


def free_bytes():
    st = os.statvfs('/tmp')
    return st.f_bavail * st.f_frsize


def read_int(path):
    try:
        return int(Path(path).read_text().strip())
    except (OSError, ValueError):
        return None


def read_kv(path):
    out = {}
    try:
        for line in Path(path).read_text().splitlines():
            key, value = line.split()
            out[key] = int(value)
    except (OSError, ValueError):
        pass
    return out


def arm_cmd(arm, tree, snap):
    env = dict(os.environ, LC_ALL='C')
    if arm == 'probe':
        return [str(PROBE), 'scan', '--root', tree, '--snapshot', str(snap), '--ppm', '1000'], env
    backend, mode = arm.split('-')
    argv = [INSTALLED, '--backend', backend, 'scan', tree, '--json']
    if mode == 'low':
        argv += ['--workers', '1', '--low-memory']  # what the installed configuration implies
    env.update(FREESPACE_CONFIG=str(BASE / 'config.toml'), FREESPACE_SNAPSHOT_DIR=str(snap), SQLITE_TMPDIR=str(snap / 'tmp'))
    return argv, env


def sampler(pid, unit, state, stop, proc_kill):
    cgroup = None
    deadline = time.monotonic() + 10
    while cgroup is None and time.monotonic() < deadline and not stop.is_set():
        try:
            path = Path(f'/proc/{pid}/cgroup').read_text().strip().splitlines()[-1].split('::', 1)[1]
            if unit in path:
                cgroup = Path('/sys/fs/cgroup') / path.lstrip('/')
        except (OSError, IndexError):
            pass
        if cgroup is None:
            time.sleep(0.005)
    if cgroup is None:
        state['sampler_error'] = 'scope cgroup not found'
        return
    state['memory_max'] = (cgroup / 'memory.max').read_text().strip() if (cgroup / 'memory.max').exists() else None
    tick = 0
    while not stop.is_set():
        current = read_int(cgroup / 'memory.current')
        if current is not None and current > state['peak']:
            state['peak'] = current
            stat = read_kv(cgroup / 'memory.stat')
            state['at_peak'] = {k: stat.get(k) for k in STAT_KEYS}
        if tick % 5 == 0:
            events = read_kv(cgroup / 'memory.events')
            if events:
                state['events'] = events
            stat = read_kv(cgroup / 'memory.stat')
            if stat:
                state['reclaim'] = {k: stat.get(k) for k in RECLAIM_KEYS}
            if free_bytes() < GUARD_ABORT:
                state['error'] = 'free-space reserve exhausted'
                proc_kill()
                return
        tick += 1
        stop.wait(0.02)


def snapshot_bytes(snap):
    total = 0
    for p in snap.rglob('*'):
        if p.is_file() and 'tmp' not in p.relative_to(snap).parts[:1]:
            total += p.stat().st_blocks * 512
    return total


def run_one(arm, tree, cap):
    COUNTER[0] += 1
    if free_bytes() < GUARD_START:
        raise SystemExit(f'free space {free_bytes()} below the start guard')
    name = f'{arm}-{tree.strip("/").replace("/", "_")}-{cap >> 20}M'
    root = WORK / name
    shutil.rmtree(root, ignore_errors=True)
    snap = root / 'snapshot'
    (snap / 'tmp').mkdir(parents=True)
    argv, env = arm_cmd(arm, tree, snap)
    if COLD:
        subprocess.run(['umount', COLD['mount']], check=True)
        subprocess.run(['mount', '-o', 'loop,noatime', COLD['image'], COLD['mount']], check=True)
    unit = f'fsbench-mem-{os.getpid()}-{COUNTER[0]}'
    cmd = ['systemd-run', '--scope', '--quiet', '--collect', '--slice=fsbench.slice', f'--unit={unit}', '-p', 'MemoryAccounting=yes',
           '-p', 'MemorySwapMax=0', '-p', f'MemoryMax={cap}', '--', 'nice', '-n', '10', 'ionice', '-c2', '-n7', *argv]
    state = dict(peak=0, at_peak={}, events={}, reclaim={}, error=None, memory_max=None)
    stop = threading.Event()
    t0 = time.monotonic()
    with (root / 'stdout').open('wb') as so, (root / 'stderr').open('wb') as se:
        proc = subprocess.Popen(cmd, stdout=so, stderr=se, env=env, start_new_session=True)
    kill = lambda: os.killpg(proc.pid, 9)  # noqa: E731
    thread = threading.Thread(target=sampler, args=(proc.pid, unit, state, stop, kill))
    thread.start()
    _, status, usage = os.wait4(proc.pid, 0)
    wall = time.monotonic() - t0
    stop.set()
    thread.join()
    code = os.waitstatus_to_exitcode(status)
    text = (root / 'stdout').read_text(errors='replace')
    report = None
    for candidate in (text, text.strip().splitlines()[-1] if text.strip() else ''):
        try:
            report = json.loads(candidate)
            break
        except json.JSONDecodeError:
            continue
    oom = state['events'].get('oom_kill', 0)
    rec = dict(arm=arm, tree=tree, cap_bytes=cap, exit_code=code, oom_kill=oom, error=state['error'] or state.get('sampler_error'),
               ok=(code == 0 and oom == 0 and state['error'] is None), wall_s=wall, user_s=usage.ru_utime, sys_s=usage.ru_stime,
               max_rss_kib=usage.ru_maxrss, cgroup_peak_sampled=state['peak'], at_peak=state['at_peak'], events=state['events'],
               memory_max=state['memory_max'], reclaim=state['reclaim'], cold=bool(COLD), entries=(report or {}).get('entries') or (report or {}).get('visited'),
               snapshot_allocated=snapshot_bytes(snap), stderr_tail=(root / 'stderr').read_text(errors='replace')[-300:],
               io_blocks=dict(inblock=usage.ru_inblock, oublock=usage.ru_oublock), at=time.strftime('%F %T%z'))
    shutil.rmtree(root, ignore_errors=True)
    return rec


def show(rec):
    mib = 1048576
    peak = rec['at_peak']
    return (f"{rec['arm']:11} {rec['tree']:6} cap={rec['cap_bytes'] // mib:5}M ok={rec['ok']!s:5} exit={rec['exit_code']:3} oom={rec['oom_kill']} "
            f"wall={rec['wall_s']:7.1f}s rss={rec['max_rss_kib'] / 1024:7.1f}MiB cgroup_peak={rec['cgroup_peak_sampled'] / mib:7.1f}MiB "
            f"(anon={(peak.get('anon') or 0) / mib:.0f} file={(peak.get('file') or 0) / mib:.0f} slab_recl={(peak.get('slab_reclaimable') or 0) / mib:.0f}) "
            f"limit_hits={rec['events'].get('max')} pgsteal={rec['reclaim'].get('pgsteal')} "
            f"entries={rec['entries']} snap={rec['snapshot_allocated'] / mib:.1f}MiB")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--tree', required=True)
    ap.add_argument('--arms', required=True)
    ap.add_argument('--caps', required=True, help='comma list, descending, e.g. 4G,512M,128M')
    ap.add_argument('--no-stop', action='store_true', help='keep descending after a failure')
    ap.add_argument('--image', help='loop image to remount before every run (cold dentry/inode caches)')
    ap.add_argument('--mount', help='mount point of --image')
    args = ap.parse_args()
    if os.geteuid() != 0:
        sys.exit('run as root (sudo -n)')
    if args.image:
        COLD.update(image=args.image, mount=args.mount)
    WORK.mkdir(exist_ok=True)
    RESULTS.parent.mkdir(exist_ok=True)
    for arm in args.arms.split(','):
        for cap in map(parse_cap, args.caps.split(',')):
            rec = run_one(arm, args.tree, cap)
            with RESULTS.open('a') as f:
                f.write(json.dumps(rec) + '\n')
            print(show(rec), flush=True)
            if not rec['ok']:
                print('   FAILED:', rec['stderr_tail'].strip()[-200:], flush=True)
                if not args.no_stop:
                    break
    return 0


if __name__ == '__main__':
    sys.exit(main())
