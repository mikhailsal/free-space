#!/usr/bin/env python3
"""GH-only resource-cause-v1 worker. Diagnostic receipts never qualify products."""
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import threading
import time
import uuid

from bench_v1_runtime import legacy

PROTOCOL = 'resource-cause-v1'
INTERVAL = .02
RESERVE_MAX_GAP_NS = 250_000_000
REQUIRED = ('memory.current', 'memory.peak', 'memory.max', 'memory.stat',
            'memory.events', 'memory.events.local', 'memory.swap.current',
            'memory.swap.peak', 'memory.swap.max', 'memory.pressure',
            'io.pressure', 'io.stat', 'io.max', 'cpu.max', 'cpu.stat')


def integer(value, minimum=0):
    return type(value) is int and value >= minimum


def validate_spec(spec):
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        raise ValueError('dedicated ephemeral GitHub-hosted runner required')
    if spec.get('protocol') != PROTOCOL:
        raise ValueError('wrong protocol')
    if not re.fullmatch(r'[A-Za-z0-9_-]{1,100}', spec.get('id', '')):
        raise ValueError('invalid id')
    if not re.fullmatch(r'[A-Za-z0-9_-]{16,128}', spec.get('nonce', '')):
        raise ValueError('invalid nonce')
    if spec.get('mode') not in ('idle', 'names', 'stat', 'scan') or spec.get('sampler') not in ('full', 'reserve'):
        raise ValueError('invalid mode/sampler')
    for key, value in (('memory_mib', 4096), ('cpus', 2), ('workers', 1)):
        if type(spec.get(key)) is not int or spec[key] != value:
            raise ValueError('invalid ' + key)
    if not integer(spec.get('timeout_seconds'), 1) or spec['timeout_seconds'] > 1200:
        raise ValueError('invalid timeout')
    if not integer(spec.get('expected_entries')) or (spec['expected_entries'] == 0) != (spec['mode'] == 'idle'):
        raise ValueError('invalid expected entries')
    for key in ('initial_snapshot_allocated_bytes', 'minimum_free_bytes'):
        if not integer(spec.get(key)):
            raise ValueError('invalid ' + key)
    if spec['minimum_free_bytes'] < 2**31:
        raise ValueError('reserve must be at least 2 GiB')
    limits = spec.get('io_limits')
    if not isinstance(limits, dict) or any(k not in ('rbps', 'wbps', 'riops', 'wiops') or not integer(v, 1) for k, v in limits.items()):
        raise ValueError('invalid I/O limits')
    if limits and not re.fullmatch(r'\d+:\d+', spec.get('io_device_number', '')):
        raise ValueError('invalid I/O device')


def publish(helper, path, value):
    """Publish complete JSON exclusively; never replace another receipt."""
    temp = path.with_name('.' + path.name + '-' + uuid.uuid4().hex)
    try:
        with temp.open('x'):
            pass
        helper.dump(temp, value)
        os.link(temp, path)
    finally:
        temp.unlink(missing_ok=True)


def pairs(raw):
    result = {}
    for line in raw.splitlines():
        key, value = line.split()
        if key in result or not value.isdecimal():
            raise ValueError('invalid counter row')
        result[key] = int(value)
    return result


def snapshot(helper, group, spec):
    started = time.monotonic_ns()
    values = helper.cg_read(group)
    # Legacy cg_read omits events.local; preserve it without changing that helper.
    values['memory.events.local'] = (group / 'memory.events.local').read_text().strip()
    missing = set(REQUIRED) - values.keys()
    if missing:
        raise ValueError('missing cgroup counters: ' + ','.join(sorted(missing)))
    for name in ('memory.current', 'memory.peak', 'memory.max', 'memory.swap.current', 'memory.swap.peak', 'memory.swap.max'):
        if not values[name].isdecimal():
            raise ValueError('invalid ' + name)
    if values['memory.max'] != str(4096 * 2**20) or values['memory.swap.max'] != '0':
        raise ValueError('RAM/swap cap mismatch')
    quota, period = values['cpu.max'].split()
    if not quota.isdecimal() or not period.isdecimal() or int(period) <= 0 or int(quota) != 2 * int(period):
        raise ValueError('CPU cap mismatch')
    for name, required in (('memory.stat', ('anon', 'file', 'kernel', 'slab_reclaimable', 'slab_unreclaimable')),
                           ('memory.events', ('max', 'oom', 'oom_kill')),
                           ('memory.events.local', ('max', 'oom', 'oom_kill')),
                           ('cpu.stat', ('usage_usec',))):
        if not set(required) <= pairs(values[name]).keys():
            raise ValueError('incomplete ' + name)
    for name in ('memory.pressure', 'io.pressure'):
        rows = {line.split()[0]: line.split()[1:] for line in values[name].splitlines()}
        if not {'some', 'full'} <= rows.keys():
            raise ValueError('incomplete PSI')
        for fields in rows.values():
            parsed = dict(field.split('=', 1) for field in fields)
            if not parsed.get('total', '').isdecimal():
                raise ValueError('invalid PSI total')
    # Bare device rows are legal and remain raw/unreported, never zero-filled.
    for line in values['io.stat'].splitlines():
        device, *fields = line.split()
        if not re.fullmatch(r'\d+:\d+', device) or any(not re.fullmatch(r'[A-Za-z_]+=[0-9]+', field) for field in fields):
            raise ValueError('invalid io.stat')
    helper.verify_io(values, spec)
    return dict(start_ns=started, end_ns=time.monotonic_ns(), values=values)


class Sampler:
    def __init__(self, helper, group, snapshot_dir, spec, kill):
        self.helper, self.group, self.directory, self.spec = helper, group, snapshot_dir, spec
        self.kill = kill
        self.stop_event = threading.Event()
        self.errors = []
        self.ticks = self.missed = self.late = 0
        self.max_lateness_ns = 0
        self.coverage_start_ns = time.monotonic_ns()
        self.last_tick_ns = self.coverage_start_ns
        self.max_tick_gap_ns = 0
        self.disk = self.files = self.memory = self.filesystem_peak = None
        self.thread = threading.Thread(target=self.run, name='cause-reserve-sampler', daemon=True)

    def tick(self):
        started = time.monotonic_ns()
        self.max_tick_gap_ns = max(self.max_tick_gap_ns, started - self.last_tick_ns)
        self.last_tick_ns = started
        usage = os.statvfs(self.directory)
        if usage.f_bavail * usage.f_frsize < self.spec['minimum_free_bytes']:
            raise RuntimeError('free-space reserve exhausted')
        if self.spec['sampler'] == 'full':
            used = self.helper.filesystem_used(self.directory)
            self.filesystem_peak = max(self.filesystem_peak or used, used)
            current = self.helper.occupancy(self.directory)
            if self.disk is None:
                self.disk = current
            else:
                for kind, values in current.items():
                    for key, value in values.items():
                        self.disk[kind][key] = max(self.disk[kind][key], value)
            if self.files is None:
                self.files = {}
            for name, values in self.helper.file_occupancy(self.directory).items():
                prior = self.files.setdefault(name, dict(logical_bytes=0, allocated_bytes=0))
                for key, value in values.items():
                    prior[key] = max(prior[key], value)
            current_memory = int((self.group / 'memory.current').read_text())
            self.memory = max(self.memory or current_memory, current_memory)
        self.ticks += 1

    def run(self):
        deadline = time.monotonic()
        try:
            while not self.stop_event.is_set():
                now = time.monotonic()
                lateness = max(0, now - deadline)
                self.max_lateness_ns = max(self.max_lateness_ns, int(lateness * 1e9))
                self.late += int(lateness > INTERVAL)
                skipped = int(lateness / INTERVAL)
                self.missed += skipped
                deadline += (skipped + 1) * INTERVAL
                self.tick()
                self.stop_event.wait(max(0, deadline - time.monotonic()))
        except Exception as error:
            self.errors.append(repr(error))
            self.kill()

    def finish(self):
        self.stop_event.set()
        if self.thread.ident is not None:
            self.thread.join(timeout=5)
            if self.thread.is_alive():
                self.errors.append('sampler join timeout')
        ended = time.monotonic_ns()
        # Include both boundary gaps, including a blocked final reserve check.
        self.max_tick_gap_ns = max(self.max_tick_gap_ns, ended - self.last_tick_ns)
        return dict(interval_seconds=INTERVAL, ticks=self.ticks, missed_ticks=self.missed,
                    late_ticks=self.late, max_lateness_ns=self.max_lateness_ns,
                    max_tick_gap_ns=self.max_tick_gap_ns,
                    reserve_max_gap_ns=RESERVE_MAX_GAP_NS,
                    coverage_start_ns=self.coverage_start_ns, coverage_end_ns=ended,
                    coverage_valid=bool(self.ticks and not self.errors
                                        and self.max_tick_gap_ns <= RESERVE_MAX_GAP_NS),
                    coverage_scope='check-start gaps plus initialization/finish boundaries; missed/late ticks descriptive',
                    errors=self.errors, capture_errors=len(self.errors),
                    availability='sampled-lower-bound' if self.spec['sampler'] == 'full' else 'unavailable-reserve-only',
                    sampled_disk_peak=self.disk, sampled_file_peaks=self.files,
                    sampled_memory_peak=self.memory, filesystem_used_peak=self.filesystem_peak)


def validate_report(report, timing, spec):
    if not isinstance(report, dict) or not integer(report.get('entries')) or report['entries'] != spec['expected_entries'] or report.get('errors') or report.get('incomplete'):
        raise ValueError('invalid CLI report/count')
    if (not isinstance(timing, dict) or not integer(timing.get('peak_rss_kib'))
            or not integer(timing.get('exit_code')) or timing['exit_code'] != 0
            or any(not integer(timing.get(key)) for key in ('inputs', 'outputs'))):
        raise ValueError('invalid GNU time report')
    elapsed = timing.get('elapsed_seconds')
    if type(elapsed) not in (int, float) or not math.isfinite(elapsed) or elapsed < 0:
        raise ValueError('invalid GNU elapsed')
    if spec['mode'] == 'scan':
        if (type(report.get('errors')) is not int or report['errors'] != 0
                or report.get('incomplete') is not False
                or type(report.get('workers')) is not int or report['workers'] != 1):
            raise ValueError('invalid scan status')
        return
    if report.get('protocol') != PROTOCOL or report.get('mode') != spec['mode'] or report.get('validated') is not True:
        raise ValueError('invalid probe report')
    keys = ('regular_files', 'directories', 'directory_opens', 'root_lstat_calls', 'entry_stat_calls')
    if any(not integer(report.get(key)) for key in keys):
        raise ValueError('invalid probe counters')
    if spec['mode'] == 'idle':
        if any(report[key] for key in keys) or report.get('traversal') is not False or report.get('path_kind_digest') is not None:
            raise ValueError('idle traversed fixture')
    elif (report['directories'] < 1 or report['regular_files'] + report['directories'] != report['entries']
          or report['directory_opens'] != report['directories'] or report['root_lstat_calls'] != 1
          or report['entry_stat_calls'] != (report['entries'] - 1 if spec['mode'] == 'stat' else 0)
          or report.get('traversal') is not True or not re.fullmatch(r'[0-9a-f]{64}', report.get('path_kind_digest', ''))):
        raise ValueError('probe invariant failed')
    for key in ('regular_files', 'directories'):
        if 'expected_' + key in spec and report[key] != spec['expected_' + key]:
            raise ValueError('unexpected ' + key)


def read_json(path):
    if path.is_symlink() or path.stat().st_size > 65536:
        raise ValueError('invalid bounded JSON file')
    return json.loads(path.read_text())


def measure(spec, directory, helper):
    group = helper.cgroup()
    target = Path('/scratch/snapshot')
    errors, proc, timed_out = [], None, threading.Event()

    def kill():
        if proc is not None and proc.poll() is None:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except Exception as error:
                errors.append('watchdog kill: ' + repr(error))

    before = snapshot(helper, group, spec)
    endpoint_before = dict(occupancy=helper.occupancy(target), files=helper.file_occupancy(target),
                           filesystem_used_bytes=helper.filesystem_used(target))
    sampler = Sampler(helper, group, target, spec, kill)
    # Refuse launch synchronously if the common reserve check cannot run.
    sampler.tick()
    common = ['--backend', 'v1', 'scan', '/scratch/tree', '--workers', '1', '--json', '--low-memory']
    command = ['/binary', *common] if spec['mode'] == 'scan' else [sys.executable, '/harness/cause_probe.py', *common]
    env = dict(os.environ, LC_ALL='C', FREESPACE_CONFIG='/scratch/config.toml',
               FREESPACE_SNAPSHOT_DIR='/scratch/snapshot', SQLITE_TMPDIR='/scratch/snapshot/tmp',
               FREESPACE_CAUSE_PROBE_MODE=spec['mode'])
    env.pop('FREESPACE_MEMORY_PROFILE', None)
    wrapper = ['/usr/bin/time', '-f', '{"elapsed_seconds":%e,"peak_rss_kib":%M,"inputs":%I,"outputs":%O,"exit_code":%x}', '-o', str(directory / 'time.log')]
    started = finished = None
    timer = None
    try:
        with (directory / 'stdout.json').open('x') as out, (directory / 'stderr.log').open('x') as err:
            started = time.monotonic_ns()
            proc = subprocess.Popen(wrapper + command, stdout=out, stderr=err, env=env, start_new_session=True)
            sampler.thread.start()

            def expire():
                if proc.poll() is None:
                    timed_out.set()
                    kill()

            timer = threading.Timer(spec['timeout_seconds'], expire)
            timer.start()
            try:
                proc.wait(timeout=spec['timeout_seconds'] + 5)
            except subprocess.TimeoutExpired:
                timed_out.set()
                errors.append('bounded wait expired')
                kill()
                try:
                    proc.kill()
                    proc.wait(timeout=5)
                except Exception as error:
                    errors.append('process cleanup: ' + repr(error))
            finished = time.monotonic_ns()
    except Exception as error:
        errors.append('CLI: ' + repr(error))
        kill()
        if proc is not None:
            try:
                proc.wait(timeout=5)
            except Exception as cleanup_error:
                errors.append('process cleanup: ' + repr(cleanup_error))
    finally:
        if timer is not None:
            timer.cancel()
            timer.join(timeout=5)
            if timer.is_alive():
                errors.append('watchdog join timeout')
        sampled = sampler.finish()
    after = snapshot(helper, group, spec)
    endpoint_after = dict(occupancy=helper.occupancy(target), files=helper.file_occupancy(target),
                          filesystem_used_bytes=helper.filesystem_used(target))
    if proc is None or proc.returncode != 0 or timed_out.is_set():
        errors.append('CLI exit/timeout invalid')
    if not sampled['coverage_valid']:
        errors.append('sampler invalid')
    if spec['sampler'] == 'full' and any(sampled[key] is None for key in ('sampled_disk_peak', 'sampled_file_peaks', 'sampled_memory_peak', 'filesystem_used_peak')):
        errors.append('full sampler incomplete')
    for name in ('memory.events', 'memory.events.local'):
        if any(pairs(after['values'][name])[key] > pairs(before['values'][name])[key] for key in ('oom', 'oom_kill')):
            errors.append('OOM event')
    if any(int(after['values'][name]) != 0 for name in ('memory.swap.current', 'memory.swap.peak')):
        errors.append('swap observed')
    report = timing = None
    try:
        report, timing = read_json(directory / 'stdout.json'), read_json(directory / 'time.log')
        validate_report(report, timing, spec)
    except Exception as error:
        errors.append('report: ' + repr(error))
    return dict(protocol=PROTOCOL, id=spec['id'], nonce=spec['nonce'], mode=spec['mode'], sampler=spec['sampler'],
                command=command, validated=not errors, cli_validated=not errors,
                semantic_validated=False, qualification=False, errors=errors,
                capture_errors=sampled['capture_errors'],
                cli_start_ns=started, cli_end_ns=finished,
                elapsed_seconds=(finished - started) / 1e9 if started is not None and finished is not None else None,
                timed_out=timed_out.is_set(), exit_code=proc.returncode if proc else None,
                cgroup_path=str(group), cgroup_before=before, cgroup_after=after,
                kernel_memory_peak_bytes=int(after['values']['memory.peak']),
                kernel_peak_scope='worker lifetime through frozen post-CLI capture; excludes later host acknowledgement hold',
                endpoint_before=endpoint_before, endpoint_after=endpoint_after,
                sampling=sampled, stdout=report, gnu_time=timing)


def await_ack(path, spec, timeout=90):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            data = read_json(path)
            if not isinstance(data, dict) or data.get('nonce') != spec['nonce'] or data.get('id') != spec['id'] or data.get('protocol') != PROTOCOL:
                raise ValueError('host acknowledgement mismatch')
            return True
        time.sleep(.05)
    raise TimeoutError('host acknowledgement timeout')


def run(spec_path):
    spec = read_json(Path(spec_path))
    validate_spec(spec)
    helper = legacy()
    directory = Path(spec_path).parent / spec['id']
    directory.mkdir(exist_ok=True)
    if directory.is_symlink():
        raise ValueError('symlink worker directory')
    if any((directory / name).exists() for name in ('metrics.json', 'worker-ready.json', 'worker-terminal.json', 'stdout.json', 'time.log', 'host-ack.json')):
        raise ValueError('worker output already exists')
    try:
        metrics = measure(spec, directory, helper)
    except Exception as error:
        metrics = dict(protocol=PROTOCOL, id=spec['id'], nonce=spec['nonce'],
                       validated=False, cli_validated=False, semantic_validated=False,
                       qualification=False, capture_errors=1,
                       errors=['measurement: ' + repr(error)])
    publish(helper, directory / 'metrics.json', metrics)
    publish(helper, directory / 'worker-ready.json', dict(protocol=PROTOCOL, id=spec['id'], nonce=spec['nonce'],
            status='ready', cli_validated=metrics['cli_validated'], kernel_peak_frozen_in='metrics.json',
            ready_ns=time.monotonic_ns(), hold_timeout_seconds=90))
    acknowledged, errors = False, []
    try:
        acknowledged = await_ack(directory / 'host-ack.json', spec)
    except Exception as error:
        errors.append(repr(error))
    valid = metrics['cli_validated'] and acknowledged and not errors
    publish(helper, directory / 'worker-terminal.json', dict(protocol=PROTOCOL, id=spec['id'], nonce=spec['nonce'],
            validated=valid, success=valid, acknowledged=acknowledged, errors=errors,
            status='complete' if valid else 'failed', cli_validated=metrics['cli_validated'],
            ack_status='matched' if acknowledged else 'invalid-or-timeout',
            qualification=False, terminal_ns=time.monotonic_ns()))
    return 0 if valid else 1


if __name__ == '__main__':
    raise SystemExit(run(sys.argv[1]))
