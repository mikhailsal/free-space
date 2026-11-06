#!/usr/bin/env python3
"""Whole-disk benchmark of freespace v1 against the coarse bucket probe, run as root.

Approximate and exploratory: one live host that other tasks keep busy, warm/cold cache
as it happens, commands run under nice/ionice, single runs. Every command is measured
with wait4 (wall time, CPU, max RSS), a 50 ms sampler of the snapshot directory and a
guard on free disk space and available memory. Phases (run as separate invocations):

  probe-scan [--n 2]   scan / with the live-tolerant probe build
  v1-scan              scan / with the installed freespace, v1 backend, installed flags (low memory, one walker)
  v1-default-scan      the same scan with defaults: min(CPUs, 8) walkers, entries kept in memory
  refresh              refresh the same folders with the probe and with v1, in place
"""
import argparse
import json
import os
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import local_host_bench as lhb  # noqa: E402

BASE = Path('/tmp/fsbench')
WHOLE = BASE / 'whole'
PROBE = BASE / 'probe-target/x86_64-unknown-linux-musl/release/cleanup_probe_live'
INSTALLED = Path('/usr/local/bin/freespace')
MEM_ABORT_KIB = 1024 * 1024  # MemAvailable below 1 GiB ...
BIG_RSS_KIB = 768 * 1024  # ... and this command holding more than 768 MiB: it is part of the problem, stop it
RSS_CAP_KIB = 3584 * 1024  # hard cap on the command's own resident set
STATUS_EVERY = 60.0
TAG = 'whole'


def mem_available_kib():
    for line in Path('/proc/meminfo').read_text().splitlines():
        if line.startswith('MemAvailable:'):
            return int(line.split()[1])
    return 0


def vm_rss_kib(pid):
    try:
        for line in Path(f'/proc/{pid}/status').read_text().splitlines():
            if line.startswith('VmRSS:'):
                return int(line.split()[1])
    except OSError:
        pass
    return 0


def execute(label, op, argv, env, snap, out_dir, timeout, make_tmp=False, target=None):
    """Run argv once and return its measurement record (the lhb.run logic, plus a memory guard)."""
    lhb.guard()
    snap.mkdir(parents=True, exist_ok=True)
    if make_tmp:
        (snap / 'tmp').mkdir(exist_ok=True)
    out_dir.mkdir(parents=True, exist_ok=True)
    start_files = lhb.dir_files(snap)
    state = dict(peak=lhb.alloc_total(start_files), min_free=lhb.free_bytes(), min_mem=mem_available_kib(), error=None, series=[], rss_sampled=0)
    stop = threading.Event()
    before = lhb.pressure()
    load_before = Path('/proc/loadavg').read_text().split()[:3]
    t0 = time.monotonic()
    with (out_dir / 'stdout').open('wb') as so, (out_dir / 'stderr').open('wb') as se:
        proc = subprocess.Popen(['nice', '-n', '10', 'ionice', '-c2', '-n7', *argv], stdout=so, stderr=se, env=env, start_new_session=True)

    def kill(reason):
        state['error'] = reason
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    def sample():
        next_status = time.monotonic() + STATUS_EVERY
        tick = 0
        while not stop.is_set():
            allocated = lhb.alloc_total(lhb.dir_files(snap))
            state['peak'] = max(state['peak'], allocated)
            free = lhb.free_bytes()
            state['min_free'] = min(state['min_free'], free)
            if tick % 5 == 0:
                avail = mem_available_kib()
                state['min_mem'] = min(state['min_mem'], avail)
                rss = vm_rss_kib(proc.pid)
                state['rss_sampled'] = max(state['rss_sampled'], rss)
                if (avail < MEM_ABORT_KIB and rss > BIG_RSS_KIB) or rss > RSS_CAP_KIB:
                    kill(f'memory guard: available {avail} KiB, own RSS {rss} KiB')
                    return
            if free < lhb.GUARD_ABORT:
                kill('free-space reserve exhausted')
                return
            if time.monotonic() >= next_status:
                next_status += STATUS_EVERY
                elapsed = time.monotonic() - t0
                state['series'].append((round(elapsed), allocated, free))
                print(f'  [{label} {elapsed:6.0f}s] snapshot={allocated / 1048576:8.1f}MiB free={free / 1073741824:5.2f}GiB '
                      f'rss={state["rss_sampled"] / 1024:7.0f}MiB memavail={mem_available_kib() / 1024:7.0f}MiB load={Path("/proc/loadavg").read_text().split()[0]}', flush=True)
            tick += 1
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
    after = lhb.pressure()
    final = lhb.dir_files(snap)
    kept = {os.path.relpath(k, snap): v for k, v in final.items() if not os.path.relpath(k, snap).startswith('tmp' + os.sep)}
    state['peak'] = max(state['peak'], lhb.alloc_total(final))
    text = (out_dir / 'stdout').read_text(errors='replace')
    report = None
    for candidate in (text, text.strip().splitlines()[-1] if text.strip() else ''):
        try:
            report = json.loads(candidate)
            break
        except json.JSONDecodeError:
            continue
    return dict(
        arm=label, op=op, target=target, argv=argv, exit_code=proc.returncode, error=state['error'], wall_s=wall,
        user_s=ru.ru_utime, sys_s=ru.ru_stime, max_rss_kib=ru.ru_maxrss, report=report,
        stderr_tail=(out_dir / 'stderr').read_text(errors='replace')[-600:],
        final_files=kept, final_logical=sum(v[0] for v in kept.values()), final_allocated=sum(v[1] for v in kept.values()),
        peak_allocated=state['peak'], min_free_bytes=state['min_free'], min_mem_available_kib=state['min_mem'], rss_sampled_peak_kib=state['rss_sampled'],
        series=state['series'], load_before=load_before, load_after=Path('/proc/loadavg').read_text().split()[:3],
        psi_stall_us={k: round(after[k]['total'] - before[k]['total']) for k in after if k in before},
        started_at=time.time() - wall, io_blocks=dict(inblock=ru.ru_inblock, oublock=ru.ru_oublock),
        ctx_switches=dict(voluntary=ru.ru_nvcsw, involuntary=ru.ru_nivcsw),
    )


def good(rec):
    r = rec['report']
    return rec['exit_code'] == 0 and not rec['error'] and isinstance(r, dict) and r.get('success', True) is True


def show(rec):
    r = rec['report'] or {}
    keys = ('entries', 'visited', 'retained', 'buckets', 'tolerated_errors', 'digest_changed', 'fallback_full_root', 'scope')
    extra = {k: r[k] for k in keys if k in r}
    return (f"{rec['arm']} {rec['op']} wall={rec['wall_s']:.1f}s user={rec['user_s']:.0f}s sys={rec['sys_s']:.0f}s "
            f"rss={rec['max_rss_kib'] / 1024:.1f}MiB final={rec['final_allocated'] / 1048576:.2f}MiB "
            f"peak={rec['peak_allocated'] / 1048576:.2f}MiB exit={rec['exit_code']} ok={good(rec)} {extra}")


def probe_env():
    return dict(os.environ, LC_ALL='C')


def v1_env(snap, low_memory=True):
    config = WHOLE / ('v1.config.toml' if low_memory else 'v1-default.config.toml')
    config.write_text('roots = ["/", "/boot/efi"]\nkeep_previous = true\nlow_memory = %s\nbackend = "v1"\n' % str(low_memory).lower())
    return dict(os.environ, LC_ALL='C', FREESPACE_CONFIG=str(config), FREESPACE_SNAPSHOT_DIR=str(snap), SQLITE_TMPDIR=str(snap / 'tmp'))


def probe_cmd(op, snap, target=None):
    base = [str(PROBE), op, '--root', '/', '--snapshot', str(snap)]
    return base + (['--ppm', '1000'] if op == 'scan' else ['--target', target])


def v1_cmd(op, target=None, default=False):
    if op == 'scan' and default:
        return [str(INSTALLED), '--backend', 'v1', 'scan', '--json']
    if op == 'scan':
        return [str(INSTALLED), '--backend', 'v1', 'scan', '--low-memory', '--workers', '1', '--json']  # the Oct 31 calibration command
    return [str(INSTALLED), '--backend', 'v1', 'refresh', target, '--workers', '1', '--json']


def phase_probe_scan(n):
    snap = WHOLE / 'probe' / 'snapshot'
    for i in range(1, n + 1):
        rec = execute('probe-live', 'scan', probe_cmd('scan', snap), probe_env(), snap, WHOLE / 'probe' / f'out-scan{i}', 4 * 3600)
        lhb.emit(TAG, phase='scan', repeat=i, **rec)
        print(show(rec), flush=True)
        if not good(rec):
            print('FAILED:', rec['stderr_tail'], rec['report'], flush=True)
            return 2
        time.sleep(1.05)
    return 0


def phase_v1_scan():
    snap = WHOLE / 'v1' / 'snapshot'
    rec = execute('v1', 'scan', v1_cmd('scan'), v1_env(snap), snap, WHOLE / 'v1' / 'out-scan', 4 * 3600, make_tmp=True)
    lhb.emit(TAG, phase='scan', repeat=1, **rec)
    print(show(rec), flush=True)
    if not good(rec):
        print('FAILED:', rec['stderr_tail'], rec['report'], flush=True)
        return 2
    return 0


def phase_v1_default_scan():
    snap = WHOLE / 'v1w' / 'snapshot'
    rec = execute('v1-default', 'scan', v1_cmd('scan', default=True), v1_env(snap, low_memory=False), snap, WHOLE / 'v1w' / 'out-scan', 4 * 3600, make_tmp=True)
    lhb.emit(TAG, phase='scan', repeat=1, **rec)
    print(show(rec), flush=True)
    if not good(rec):
        print('FAILED:', rec['stderr_tail'], rec['report'], flush=True)
        return 2
    return 0


def phase_refresh(targets_file, which, repeats, with_fallback):
    picks = json.loads(Path(targets_file).read_text())
    psnap, vsnap = WHOLE / 'probe' / 'snapshot', WHOLE / 'v1' / 'snapshot'
    for scenario, target in picks.items():
        if not isinstance(target, str) or not Path(target).exists():
            continue
        if scenario == 'hardlinked-region' and not with_fallback:
            continue  # the probe would rescan the whole root: that costs a full scan
        arms = which if scenario in ('calibration-dir', 'retained-large') else [a for a in which if a == 'probe-live']
        for arm in arms:
            for rep in range(repeats if arm == 'probe-live' else 1):
                run_refresh(arm, scenario, target, psnap, vsnap, rep)
    return 0


def run_refresh(arm, scenario, target, psnap, vsnap, rep):
    if arm == 'probe-live':
        rec = execute(arm, 'refresh', probe_cmd('refresh', psnap, target), probe_env(), psnap,
                      WHOLE / 'probe' / f'out-ref-{scenario}', 3600, target=target)
    else:
        rec = execute(arm, 'refresh', v1_cmd('refresh', target), v1_env(vsnap), vsnap,
                      WHOLE / 'v1' / f'out-ref-{scenario}', 3600, make_tmp=True, target=target)
    lhb.emit(TAG, phase='refresh', scenario=scenario, repeat=rep, **rec)
    print(f'[{scenario} r{rep}]', show(rec), flush=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('phase', choices=['probe-scan', 'v1-scan', 'v1-default-scan', 'refresh'])
    ap.add_argument('--n', type=int, default=2)
    ap.add_argument('--targets', default=str(BASE / 'whole-targets.json'))
    ap.add_argument('--arms', default='probe-live,v1')
    ap.add_argument('--repeats', type=int, default=3)
    ap.add_argument('--with-fallback', action='store_true')
    args = ap.parse_args()
    if os.geteuid() != 0:
        sys.exit('run as root (sudo -n)')
    WHOLE.mkdir(exist_ok=True)
    lhb.emit(TAG, phase='phase-start', name=args.phase, started=time.strftime('%F %T%z'), free=lhb.free_bytes(),
             mem_available_kib=mem_available_kib(), load=Path('/proc/loadavg').read_text().split()[:3])
    try:
        if args.phase == 'probe-scan':
            return phase_probe_scan(args.n)
        if args.phase == 'v1-scan':
            return phase_v1_scan()
        if args.phase == 'v1-default-scan':
            return phase_v1_default_scan()
        return phase_refresh(args.targets, args.arms.split(','), args.repeats, args.with_fallback)
    except lhb.Abort as e:
        print('ABORT:', e, flush=True)
        lhb.emit(TAG, phase='abort', reason=str(e))
        return 2
    finally:
        lhb.emit(TAG, phase='phase-end', name=args.phase, finished=time.strftime('%F %T%z'), free=lhb.free_bytes())


if __name__ == '__main__':
    sys.exit(main())
