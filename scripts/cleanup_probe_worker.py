#!/usr/bin/env python3
"""Measure exactly one writer in a fresh private Docker cgroup."""
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time

p = Path(__file__).with_name('bench-v2-container.py')
spec = importlib.util.spec_from_file_location('legacy_bench', p)
legacy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(legacy)


def scope_errors(report, expected):
    errors=[]
    for key, wanted in expected.items():
        actual=report.get(key)
        if type(actual) is not type(wanted) or actual != wanted:
            errors.append(f'{key}: expected {wanted!r}, got {actual!r}')
    return errors


def run(config):
    output = Path('/results') / config['id']
    output.mkdir()
    snapshot = Path('/scratch/snapshot')
    cg = legacy.cgroup()
    before = legacy.cg_read(cg)
    if before.get('memory.max') != str(4096 * 1024 * 1024) or before.get('memory.swap.max') != '0':
        raise ValueError('unexpected memory/swap limit')
    quota, period = before['cpu.max'].split()
    if quota == 'max' or int(quota) != 2 * int(period):
        raise ValueError('unexpected CPU quota')
    legacy.verify_io(before, config)
    command = config['command']
    env = dict(os.environ, LC_ALL='C', FREESPACE_CONFIG='/scratch/config.toml',
               FREESPACE_SNAPSHOT_DIR=str(snapshot), SQLITE_TMPDIR=str(snapshot / 'tmp'))
    start_occupancy = legacy.occupancy(snapshot)
    start_used = legacy.filesystem_used(snapshot)
    state = dict(peak_used=start_used, file_peaks={}, errors=[], available_min={}, backing_allocated_peak=0)
    stop = threading.Event()
    process = None
    def sample():
        try:
            while not stop.is_set():
                state['peak_used'] = max(state['peak_used'], legacy.filesystem_used(snapshot))
                for name, values in legacy.file_occupancy(snapshot).items():
                    peak = state['file_peaks'].setdefault(name, {'logical_bytes': 0, 'allocated_bytes': 0})
                    for key, value in values.items(): peak[key] = max(peak[key], value)
                for label, path in (('inner', snapshot), ('outer', '/outer'), ('results', '/results')):
                    fs = os.statvfs(path)
                    available = fs.f_bavail * fs.f_frsize
                    state['available_min'][label] = min(state['available_min'].get(label, available), available)
                state['backing_allocated_peak'] = max(state['backing_allocated_peak'], os.stat('/outer/disk.img').st_blocks * 512)
                if any(value < 2 * 1024**3 for value in state['available_min'].values()):
                    state['errors'].append('inner or outer writer free-space reserve exhausted')
                    if process and process.poll() is None: os.killpg(process.pid, signal.SIGKILL)
                    break
                stop.wait(0.05)
        except Exception as error:
            state['errors'].append(repr(error))
    started = time.monotonic()
    with (output/'stdout.json').open('w') as out, (output/'stderr.log').open('w') as err:
        process = subprocess.Popen(['/usr/bin/time', '-f', '{"elapsed_seconds":%e,"peak_rss_kib":%M,"inputs":%I,"outputs":%O,"exit_code":%x}', '-o', str(output/'time.json'), *command], stdout=out, stderr=err, env=env, start_new_session=True)
        thread = threading.Thread(target=sample)
        thread.start()
        try:
            process.wait(timeout=config['timeout_seconds'])
        except subprocess.TimeoutExpired:
            state['errors'].append('command timeout')
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        finally:
            stop.set(); thread.join()
    elapsed = time.monotonic() - started
    after = legacy.cg_read(cg)
    legacy.verify_io(after, config)
    end = legacy.occupancy(snapshot)
    state['peak_used'] = max(state['peak_used'], legacy.filesystem_used(snapshot))
    def counters(metrics, name):
        return {parts[0]: int(parts[1]) for line in metrics.get(name, '').splitlines()
                if len(parts := line.split()) == 2 and parts[1].isdigit()}
    old_events, new_events = counters(before, 'memory.events'), counters(after, 'memory.events')
    old_stat, new_stat = counters(before, 'memory.stat'), counters(after, 'memory.stat')
    event_max = new_events.get('max', 0) - old_events.get('max', 0)
    reclaim = {key: new_stat.get(key, 0) - old_stat.get(key, 0) for key in ('pgscan', 'pgsteal', 'pgscan_direct', 'pgsteal_direct')}
    pressure = event_max > 0 or any(value > 0 for value in reclaim.values())
    report = json.loads((output/'stdout.json').read_text())
    work_errors = scope_errors(report, config['expected_scope'])
    valid = not work_errors and process.returncode == 0 and not state['errors'] and report.get('success', True) is True and not report.get('errors') and not report.get('incomplete')
    record = dict(id=config['id'], command=command, exit_code=process.returncode, validated=valid,
                  expected_scope=config['expected_scope'], scope_errors=work_errors,
                  elapsed_seconds=elapsed, report=report, time=json.loads((output/'time.json').read_text().splitlines()[-1]),
                  cgroup_before=before, cgroup_after=after, memory_events_max_delta=event_max,
                  reclaim_counter_deltas=reclaim, memory_comparison_inconclusive=pressure, initial_output=start_occupancy,
                  final_output=end, sampled_file_peaks=state['file_peaks'], sample_errors=state['errors'],
                  fs_used_start=start_used, fs_used_peak=state['peak_used'],
                  output_peak_estimate_bytes=start_occupancy['total']['allocated_bytes'] + state['peak_used'] - start_used,
                  sampled_minimum_available_bytes=state['available_min'],
                  sampled_outer_backing_allocated_peak_bytes=state['backing_allocated_peak'],
                  output_peak_scope='initial allocated snapshot plus sampled total filesystem growth; excludes initial fixture bytes; includes filesystem metadata and any open unlinked output; sampled lower bound',
                  memory_peak_scope='fresh writer container including measurement wrapper and sampler; excludes fixture setup and observers')
    (output/'metrics.json').write_text(json.dumps(record, indent=2)+'\n')
    return 0 if valid else 1


if __name__ == '__main__':
    raise SystemExit(run(json.loads(Path(sys.argv[1]).read_text())))
