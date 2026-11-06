#!/usr/bin/env python3
"""Local-host benchmark: freespace v1 / v2 against the coarse bucket probe on real trees.

Exploratory, single host, warm caches, no I/O throttling. Every timed command runs on a
fresh copy of an arm's two-scan seed snapshot (the seed scans are recorded but excluded
from the comparison). Arm order rotates between repeats. Results go to a JSONL file, one
record per command, so an aborted run keeps what it measured.

Phases: seeds, scan, refresh.  Example:
  local_host_bench.py --tree /usr --tag usr --phases seeds,scan,refresh --repeats 3
"""
import argparse
import json
import os
import shutil
import signal
import sqlite3
import subprocess
import sys
import threading
import time
from pathlib import Path

HOSTED = Path('/tmp/cleanup-screen-artifact/provenance')
BIN = {'hosted': HOSTED / 'freespace', 'probe': HOSTED / 'cleanup_probe', 'installed': Path('/usr/local/bin/freespace')}
BASE = Path('/tmp/fsbench')
WORK = BASE / 'work'
RESULTS = BASE / 'results'
GUARD_START = 3 * 1024**3  # refuse to start a step below this much free space on /tmp
GUARD_ABORT = 2560 * 1024**2  # kill a running command below this (2.5 GiB)
LOW = ['--workers', '1', '--low-memory']  # the installed config: low_memory = true, one walker
ARMS = {
    'v1': dict(kind='fs', binary='hosted', backend='v1', extra=LOW, seed='v1'),
    'v2': dict(kind='fs', binary='hosted', backend='v2', extra=LOW, seed='v2'),
    'probe': dict(kind='probe', seed='probe'),
    'v1-8w': dict(kind='fs', binary='hosted', backend='v1', extra=[], seed='v1'),
    'v2-8w': dict(kind='fs', binary='hosted', backend='v2', extra=[], seed='v2'),
    'v2-installed': dict(kind='fs', binary='installed', backend='v2', extra=LOW, seed='v2-installed'),
}
SEED_ARMS = ['v1', 'v2', 'probe', 'v2-installed']
MAIN = ['v1', 'v2', 'probe']
MASK = (1 << 64) - 1


class Abort(Exception):
    pass


def free_bytes():
    return shutil.disk_usage('/tmp').free


def guard():
    if free_bytes() < GUARD_START:
        raise Abort(f'free space {free_bytes()} below the start guard {GUARD_START}')


def dir_files(path):
    """File name -> (logical, allocated) for every regular file below path."""
    out = {}
    stack = [Path(path)]
    while stack:
        try:
            with os.scandir(stack.pop()) as it:
                for e in it:
                    try:
                        if e.is_dir(follow_symlinks=False):
                            stack.append(Path(e.path))
                        elif e.is_file(follow_symlinks=False):
                            st = e.stat(follow_symlinks=False)
                            out[e.path] = (st.st_size, st.st_blocks * 512)
                    except FileNotFoundError:
                        pass
        except FileNotFoundError:
            pass
    return out


def alloc_total(files):
    return sum(v[1] for v in files.values())


def pressure():
    out = {}
    for name in ('cpu', 'io', 'memory'):
        try:
            line = Path(f'/proc/pressure/{name}').read_text().splitlines()[0].split()
            out[name] = {k: float(v) for k, v in (x.split('=') for x in line[1:])}
        except OSError:
            pass
    return out


def env_for(arm, snap):
    env = dict(os.environ, LC_ALL='C')
    if ARMS[arm]['kind'] == 'fs':
        env.update(FREESPACE_CONFIG=str(BASE / 'config.toml'), FREESPACE_SNAPSHOT_DIR=str(snap), SQLITE_TMPDIR=str(snap / 'tmp'))
    return env


def command(arm, op, tree, snap, target=None):
    a = ARMS[arm]
    if a['kind'] == 'probe':
        base = [str(BIN['probe']), op, '--root', tree, '--snapshot', str(snap)]
        return base + (['--ppm', '1000'] if op == 'scan' else ['--target', target])
    base = [str(BIN[a['binary']]), '--backend', a['backend']]
    if op == 'scan':
        return base + ['scan', tree] + a['extra'] + ['--json']
    return base + ['refresh', target, '--workers', '1', '--json']


def run(arm, op, tree, snap, out_dir, target=None, timeout=3600):
    """Run one command; return its measurement record."""
    guard()
    argv = command(arm, op, tree, snap, target)
    if ARMS[arm]['kind'] == 'fs':
        (snap / 'tmp').mkdir(parents=True, exist_ok=True)
    out_dir.mkdir(parents=True, exist_ok=True)
    start_files = dir_files(snap)
    state = dict(peak=alloc_total(start_files), min_free=free_bytes(), error=None)
    stop = threading.Event()
    before = pressure()
    load_before = Path('/proc/loadavg').read_text().split()[:3]
    wrapped = ['nice', '-n', '10', 'ionice', '-c2', '-n7', *argv]
    t0 = time.monotonic()
    with (out_dir / 'stdout').open('wb') as so, (out_dir / 'stderr').open('wb') as se:
        proc = subprocess.Popen(wrapped, stdout=so, stderr=se, env=env_for(arm, snap), start_new_session=True)

    def kill(reason):
        state['error'] = reason
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    def sample():
        while not stop.is_set():
            state['peak'] = max(state['peak'], alloc_total(dir_files(snap)))
            free = free_bytes()
            state['min_free'] = min(state['min_free'], free)
            if free < GUARD_ABORT:
                kill('free-space reserve exhausted')
                return
            stop.wait(0.05)

    sampler = threading.Thread(target=sample)
    sampler.start()
    timer = threading.Timer(timeout, kill, args=('timeout',))
    timer.start()
    _, status, ru = os.wait4(proc.pid, 0)
    wall = time.monotonic() - t0
    proc.returncode = os.waitstatus_to_exitcode(status)
    timer.cancel()
    stop.set()
    sampler.join()
    after = pressure()
    final = dir_files(snap)
    kept = {os.path.relpath(k, snap): v for k, v in final.items() if not os.path.relpath(k, snap).startswith('tmp' + os.sep)}
    state['peak'] = max(state['peak'], alloc_total(final))
    text = (out_dir / 'stdout').read_text(errors='replace')
    report = None
    for candidate in (text, text.strip().splitlines()[-1] if text.strip() else ''):
        try:
            report = json.loads(candidate)
            break
        except json.JSONDecodeError:
            continue
    stall = {k: round(after[k]['total'] - before[k]['total']) for k in after if k in before}
    return dict(
        arm=arm, op=op, tree=tree, target=target, argv=argv, exit_code=proc.returncode, error=state['error'],
        wall_s=wall, user_s=ru.ru_utime, sys_s=ru.ru_stime, max_rss_kib=ru.ru_maxrss,
        report=report, stderr_tail=(out_dir / 'stderr').read_text(errors='replace')[-400:],
        final_files=kept, final_logical=sum(v[0] for v in kept.values()), final_allocated=sum(v[1] for v in kept.values()),
        peak_allocated=state['peak'], min_free_bytes=state['min_free'], load_before=load_before,
        load_after=Path('/proc/loadavg').read_text().split()[:3], psi_stall_us=stall, psi_avg10_before=before,
        started_at=time.time() - wall,
    )


def ok(rec):
    r = rec['report']
    return (rec['exit_code'] == 0 and not rec['error'] and isinstance(r, dict) and r.get('success', True) is True
            and not r.get('errors') and not r.get('incomplete'))


def emit(tag, **fields):
    RESULTS.mkdir(parents=True, exist_ok=True)
    with (RESULTS / f'{tag}.jsonl').open('a') as f:
        f.write(json.dumps(fields, ensure_ascii=False) + '\n')


def summary(rec):
    r = rec['report'] or {}
    return (f"{rec['arm']:>12} {rec['op']:<7} wall={rec['wall_s']:8.2f}s rss={rec['max_rss_kib'] / 1024:7.1f}MiB "
            f"final={rec['final_allocated'] / 1048576:8.2f}MiB peak={rec['peak_allocated'] / 1048576:8.2f}MiB "
            f"exit={rec['exit_code']} ok={ok(rec)} entries={r.get('entries', r.get('visited'))}")


def seed_dir(tag, arm):
    return WORK / tag / f'seed-{arm}' / 'snapshot'


def build_seed(tag, tree, arm):
    snap = seed_dir(tag, arm)
    if snap.parent.exists():
        shutil.rmtree(snap.parent)
    snap.mkdir(parents=True)
    for n in (1, 2):
        rec = run(arm, 'scan', tree, snap, WORK / tag / f'seed-{arm}' / f'out{n}')
        emit(tag, phase='seed', seed_scan=n, **rec)
        print('seed', n, summary(rec), flush=True)
        if not ok(rec):
            raise Abort(f'seed scan failed for {arm}: {rec["stderr_tail"]!r} {rec["report"]!r}')
        time.sleep(1.05)  # distinct second-resolution scan stamps, as in the hosted harness


def rotate(items, k):
    k %= len(items)
    return items[k:] + items[:k]


def timed(tag, phase, arm, op, tree, rep, position, scenario=None, target=None):
    case = WORK / tag / 'case'
    if case.exists():
        shutil.rmtree(case)
    snap = case / 'snapshot'
    seed = seed_dir(tag, ARMS[arm]['seed'])
    need = alloc_total(dir_files(seed))
    if free_bytes() - need < GUARD_START:
        raise Abort(f'copying the {arm} seed ({need} B) would leave less than the start guard')
    shutil.copytree(seed, snap, symlinks=True)
    try:
        rec = run(arm, op, tree, snap, case / 'out', target)
    finally:
        shutil.rmtree(case, ignore_errors=True)
    emit(tag, phase=phase, repeat=rep, position=position, scenario=scenario, **rec)
    print(f'[{phase} r{rep} {scenario or ""}]', summary(rec), flush=True)
    return rec


def slot_of(name):
    h = 0xcbf29ce484222325
    for b in name:
        h = ((h ^ b) * 0x100000001B3) & MASK
    return ((h * 0x9E3779B97F4A7C15) & MASK) >> 52


def count_entries(path, cap=6000):
    n = 0
    for _, dirs, files in os.walk(path, followlinks=False):
        n += len(dirs) + len(files)
        if n > cap:
            return n
    return n


def pick_targets(tree, probe_seed):
    """Choose refresh targets from the probe's own snapshot: retained dirs and folded dirs."""
    con = sqlite3.connect(f'file:{probe_seed / "coarse.db"}?mode=ro&immutable=1', uri=True)
    nodes = {bytes(p): dict(parent=par, entries=f + d + o, links=ln) for p, par, f, d, o, ln in
             con.execute('select path,parent,files,dirs,others,links from nodes')}
    buckets = {}
    for par, lo, hi, f, d, o, ln in con.execute('select parent,lo,hi,files,dirs,others,links from buckets order by parent,lo'):
        buckets.setdefault(bytes(par), []).append((lo, hi, f + d + o, ln))
    root = tree.encode()
    picks = {}

    def closest(want, lo, hi):
        c = [(abs(v['entries'] - want), p) for p, v in nodes.items() if p != root and v['links'] == 0 and lo <= v['entries'] <= hi]
        return min(c)[1].decode() if c else None

    picks['retained-small'] = closest(8000, 4097, 30000)
    picks['retained-large'] = closest(100000, 40000, 250000)
    small, wide = [], []
    for parent in sorted(nodes, key=lambda p: nodes[p]['entries']):
        if len(small) >= 40 and len(wide) >= 20:
            break
        if nodes[parent]['entries'] > 60000 or parent not in buckets:
            continue
        try:
            children = [e for e in os.scandir(os.fsdecode(parent)) if e.is_dir(follow_symlinks=False)]
        except OSError:
            continue
        for e in children[:400]:
            raw = os.fsencode(e.name)
            if os.fsencode(e.path) in nodes:
                continue
            bucket = next((b for b in buckets[parent] if b[0] <= slot_of(raw) < b[1]), None)
            if not bucket or bucket[3] != 0 or bucket[2] > 4200:
                continue
            n = count_entries(e.path)
            if 40 <= n <= 300:
                small.append((abs(n - 100), e.path, n))
            elif 1200 <= n <= 4096:
                wide.append((abs(n - 2000), e.path, n))
    picks['omitted'] = min(small)[1] if small else None
    picks['wide'] = min(wide)[1] if wide else None
    picks['sizes'] = {k: (count_entries(v, 400000) if v else None) for k, v in picks.items()}
    found = subprocess.run(['find', tree, '-xdev', '-type', 'f', '-links', '+1', '-print', '-quit'], capture_output=True, text=True)
    first = found.stdout.strip()
    picks['hardlinked-region'] = str(Path(first).parent) if first else None
    return picks


def phase_scan(tag, tree, repeats, arms):
    for rep in range(repeats):
        for pos, arm in enumerate(rotate(arms, 2 * rep)):
            timed(tag, 'scan', arm, 'scan', tree, rep, pos)


def phase_refresh(tag, tree, repeats):
    picks = pick_targets(tree, seed_dir(tag, 'probe'))
    emit(tag, phase='refresh-targets', picks=picks)
    print('targets', json.dumps(picks), flush=True)
    for scenario in ('retained-small', 'retained-large', 'omitted', 'wide', 'hardlinked-region'):
        target = picks.get(scenario)
        if not target:
            print('no target for', scenario, flush=True)
            continue
        for rep in range(repeats):
            for pos, arm in enumerate(rotate(MAIN, rep)):
                timed(tag, 'refresh', arm, 'refresh', tree, rep, pos, scenario, target)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--tree', required=True)
    ap.add_argument('--tag', required=True)
    ap.add_argument('--phases', default='seeds,scan,refresh')
    ap.add_argument('--repeats', type=int, default=3)
    ap.add_argument('--arms', default=','.join(ARMS))
    ap.add_argument('--seed-arms', default=','.join(SEED_ARMS))
    args = ap.parse_args()
    BASE.mkdir(exist_ok=True)
    (BASE / 'config.toml').write_text('keep_previous = true\n')
    phases = args.phases.split(',')
    try:
        emit(args.tag, phase='start', tree=args.tree, started=time.strftime('%F %T%z'), phases=phases, free=free_bytes(),
             hashes={k: subprocess.run(['sha256sum', str(v)], capture_output=True, text=True).stdout.split()[0] for k, v in BIN.items()})
        if 'seeds' in phases:
            for arm in args.seed_arms.split(','):
                build_seed(args.tag, args.tree, arm)
        if 'scan' in phases:
            phase_scan(args.tag, args.tree, args.repeats, args.arms.split(','))
        if 'refresh' in phases:
            phase_refresh(args.tag, args.tree, args.repeats)
    except Abort as e:
        print('ABORT:', e, flush=True)
        emit(args.tag, phase='abort', reason=str(e))
        return 2
    finally:
        emit(args.tag, phase='end', finished=time.strftime('%F %T%z'), free=free_bytes())
    return 0


if __name__ == '__main__':
    sys.exit(main())
