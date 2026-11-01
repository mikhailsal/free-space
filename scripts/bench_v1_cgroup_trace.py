"""Opt-in bounded cgroup trace; the caller owns scheduling and process outcome.

Call tick() from the existing sampler loop. No background thread is created;
stop() closes the trace and leaves no collector thread to join. Values from
separate pseudo-files are not atomic and overlapping memory.stat fields are not
an additive peak decomposition.
"""
import json
import os
from pathlib import Path
import stat
import time

INTERVAL_NS = 5_000_000_000
MAX_SAMPLES = 1500
MAX_OUTPUT = 4 * 1024 * 1024
MAX_READ = 64 * 1024
MAX_RECORD_BYTES = 64 * 1024
SUMMARY_RESERVE = 4096
FINAL_RESERVE = MAX_RECORD_BYTES + SUMMARY_RESERVE
LATE_TOLERANCE_NS = 20_000_000
FILES = ('memory.current', 'memory.peak', 'memory.stat', 'memory.events',
         'memory.events.local', 'memory.pressure', 'io.pressure', 'io.stat')
STAT_KEYS = ('anon', 'file', 'kernel', 'slab_reclaimable', 'slab_unreclaimable',
             'active_anon', 'inactive_anon', 'active_file', 'inactive_file',
             'file_dirty', 'file_writeback', 'workingset_refault_anon',
             'workingset_refault_file', 'pgscan', 'pgsteal')
EVENT_KEYS = ('low', 'high', 'max', 'oom', 'oom_kill', 'oom_group_kill')


def _integer(value):
    return value.isascii() and value.isdecimal()


def _pairs(raw):
    result = {}
    for line in raw.splitlines():
        parts = line.split()
        if len(parts) != 2 or parts[0] in result or not _integer(parts[1]):
            raise ValueError('invalid counter line')
        result[parts[0]] = int(parts[1])
    return result


def _psi(raw):
    rows = {}
    for line in raw.splitlines():
        parts = line.split()
        if not parts or parts[0] in rows:
            raise ValueError('invalid pressure line')
        values = {}
        for part in parts[1:]:
            key, value = part.split('=', 1)
            if key in values:
                raise ValueError('duplicate pressure field')
            values[key] = value
        if set(values) != {'avg10', 'avg60', 'avg300', 'total'} or not _integer(values['total']):
            raise ValueError('invalid pressure fields')
        for key in ('avg10', 'avg60', 'avg300'):
            value = float(values[key])
            if not 0 <= value <= 100:
                raise ValueError('invalid pressure average')
        rows[parts[0]] = values
    if set(rows) != {'some', 'full'}:
        raise ValueError('missing pressure rows')
    return rows


def _io(raw):
    rows = {}
    for line in raw.splitlines():
        parts = line.split()
        if not parts or parts[0] in rows:
            raise ValueError('invalid io device')
        device = parts[0].split(':')
        if len(device) != 2 or not all(_integer(x) for x in device):
            raise ValueError('invalid io device')
        values = {}
        for part in parts[1:]:
            key, value = part.split('=', 1)
            if not key or key in values or not value:
                raise ValueError('invalid io counter')
            values[key] = value
        required = {'rbytes', 'wbytes', 'rios', 'wios'}
        present = required & values.keys()
        if present and present != required:
            raise ValueError('missing io counter')
        if present and not all(_integer(values[key]) for key in required):
            raise ValueError('invalid required io counter')
        discard = {'dbytes', 'dios'} & values.keys()
        if discard and not present:
            raise ValueError('discard counters require primary io counters')
        if any(not _integer(values[key]) for key in discard):
            raise ValueError('invalid discard io counter')
        rows[parts[0]] = values
    return rows


def _io_unreported(raw):
    return [device for device, values in _io(raw).items() if 'rbytes' not in values]


def _validate(name, raw):
    if name in ('memory.current', 'memory.peak'):
        if not _integer(raw.strip()):
            raise ValueError('invalid memory scalar')
    elif name == 'memory.stat':
        values = _pairs(raw)
        if any(key not in values for key in STAT_KEYS):
            raise ValueError('missing memory.stat field')
    elif name in ('memory.events', 'memory.events.local'):
        values = _pairs(raw)
        if any(key not in values for key in EVENT_KEYS):
            raise ValueError('missing memory.events field')
    elif name in ('memory.pressure', 'io.pressure'):
        _psi(raw)
    else:
        _io(raw)


def _read(group, name):
    fd = os.open(group / name, os.O_RDONLY | os.O_CLOEXEC | os.O_NONBLOCK | os.O_NOFOLLOW)
    try:
        chunks, size = [], 0
        while size <= MAX_READ:
            chunk = os.read(fd, min(4096, MAX_READ + 1 - size))
            if not chunk:
                break
            chunks.append(chunk)
            size += len(chunk)
        if size > MAX_READ:
            raise ValueError('cgroup file exceeds limit')
        return b''.join(chunks).decode('utf-8').strip()
    finally:
        os.close(fd)


class CgroupTrace:
    """Caller-driven collector; stop is terminal, including before start.

    Repeated start raises RuntimeError. Diagnostic file failures return invalid
    status; the caller decides how to report that without changing CLI outcome.
    """

    def __init__(self, group, output_dir, *, read=_read, clock=time.monotonic_ns,
                 cpu_clock=time.thread_time_ns):
        self.group, self.output_dir = Path(group), Path(output_dir)
        self.read, self.clock, self.cpu_clock = read, clock, cpu_clock
        self.fd = None
        self.next_ns = None
        self.started_ns = self.ended_ns = None
        self.count = self.attempts = self.missed = self.late = self.errors = self.missing = 0
        self.bytes = self.wall_ns = self.cpu_ns = 0
        self.max_lateness_ns = 0
        self.final_covers_due_tick = False
        self.final_covered_deadline_ns = None
        self.truncated = False
        self.write_broken = False
        self.valid = True
        self.finished = False

    def start(self):
        if self.next_ns is not None or self.finished:
            raise RuntimeError('trace already started or finished')
        try:
            parent = os.open(self.output_dir, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
            try:
                self.fd = os.open('cgroup-trace.jsonl', os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                                  os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=parent)
                if not stat.S_ISREG(os.fstat(self.fd).st_mode):
                    raise ValueError('trace output is not regular')
            finally:
                os.close(parent)
            self.started_ns = self.next_ns = self.clock()
            return True
        except (OSError, ValueError):
            self.valid = False
            self.errors += 1
            if self.fd is not None:
                try:
                    os.close(self.fd)
                except OSError:
                    self.errors += 1
                self.fd = None
            self.finished = True
            return False

    def _write(self, record, limit):
        data = (json.dumps(record, separators=(',', ':'), allow_nan=False) + '\n').encode()
        if len(data) > MAX_RECORD_BYTES or self.bytes + len(data) > limit:
            self.truncated = True
            self.valid = False
            return False
        before = self.bytes
        view = memoryview(data)
        try:
            while view:
                size = os.write(self.fd, view)
                if size <= 0:
                    raise OSError('short trace write')
                self.bytes += size
                view = view[size:]
        except OSError:
            try:
                os.ftruncate(self.fd, before)
                os.lseek(self.fd, before, os.SEEK_SET)
                self.bytes = before
            except OSError:
                self.write_broken = True
            raise
        return True

    def tick(self):
        if self.finished or self.next_ns is None or self.truncated:
            return
        now = self.clock()
        if now < self.next_ns:
            return
        skipped = (now - self.next_ns) // INTERVAL_NS
        self.missed += skipped
        requested = self.next_ns + skipped * INTERVAL_NS
        lateness = now - requested
        self.max_lateness_ns = max(self.max_lateness_ns, lateness)
        if lateness > LATE_TOLERANCE_NS:
            self.late += 1
        self.next_ns += (skipped + 1) * INTERVAL_NS
        if self.count >= MAX_SAMPLES - 1:
            self.truncated = True
            self.valid = False
            return
        self._sample('periodic', requested, lateness, now)

    def _sample(self, kind, requested=None, lateness=None, actual=None):
        start, cpu_start = self.clock(), self.cpu_clock()
        self.attempts += 1
        if kind == 'final' and start >= self.next_ns:
            overdue = start - self.next_ns
            if overdue <= LATE_TOLERANCE_NS:
                self.final_covers_due_tick = True
                self.final_covered_deadline_ns = self.next_ns
            else:
                self.missed += 1 + overdue // INTERVAL_NS
        values, unavailable = {}, {}
        io_stat_unreported_devices = None
        for name in FILES:
            try:
                raw = self.read(self.group, name)
                if not isinstance(raw, str) or len(raw.encode()) > MAX_READ:
                    raise ValueError('invalid bounded cgroup read')
            except FileNotFoundError:
                values[name] = None
                unavailable[name] = 'missing'
                self.missing += 1
                self.valid = False
                continue
            except (OSError, UnicodeError, ValueError, OverflowError):
                values[name] = None
                unavailable[name] = 'read_or_parse_error'
                self.errors += 1
                self.valid = False
                continue
            values[name] = raw
            try:
                _validate(name, raw)
                if name == 'io.stat':
                    io_stat_unreported_devices = _io_unreported(raw)
            except (ValueError, OverflowError):
                unavailable[name] = 'invalid_content'
                self.errors += 1
                self.valid = False
        end = self.clock()
        try:
            limit = MAX_OUTPUT - (FINAL_RESERVE if kind == 'periodic' else SUMMARY_RESERVE)
            if self._write(dict(type='sample', kind=kind, requested_ns=requested, actual_ns=start if actual is None else actual,
                                lateness_ns=lateness,
                                final_covers_due_tick=self.final_covers_due_tick if kind == 'final' else None,
                                final_covered_deadline_ns=self.final_covered_deadline_ns if kind == 'final' else None,
                                io_stat_unreported_devices=io_stat_unreported_devices,
                                start_ns=start, end_ns=end, values=values,
                                unavailable=unavailable),
                           limit):
                self.count += 1
        except OSError:
            self.errors += 1
            self.valid = False
            self.truncated = True
        finally:
            self.wall_ns += self.clock() - start
            self.cpu_ns += self.cpu_clock() - cpu_start

    def stop(self):
        if self.finished:
            return self.result()
        self.finished = True
        if self.next_ns is not None:
            self.ended_ns = self.clock()
        if self.fd is not None:
            try:
                if not self.write_broken and self.count < MAX_SAMPLES:
                    self._sample('final')
                    self.ended_ns = self.clock()
                summary = self.result()
                summary['bytes_before_summary'] = summary.pop('bytes')
                summary['valid_before_close'] = summary.pop('valid')
                summary['errors_before_close'] = summary.pop('errors')
                if not self.write_broken:
                    self._write(dict(type='summary', **summary), MAX_OUTPUT)
            except OSError:
                self.errors += 1
                self.valid = False
            finally:
                try:
                    os.close(self.fd)
                except OSError:
                    self.errors += 1
                    self.valid = False
                self.fd = None
        return self.result()

    def result(self):
        return dict(valid=self.valid and self.finished and self.count > 0 and not self.truncated,
                    started_ns=self.started_ns, ended_ns=self.ended_ns,
                    interval_ns=INTERVAL_NS, samples=self.count, attempted=self.attempts, missed=self.missed,
                    late=self.late, max_lateness_ns=self.max_lateness_ns,
                    final_covers_due_tick=self.final_covers_due_tick,
                    final_covered_deadline_ns=self.final_covered_deadline_ns,
                    late_tolerance_ns=LATE_TOLERANCE_NS, errors=self.errors, missing=self.missing,
                    truncated=self.truncated, bytes=self.bytes, wall_ns=self.wall_ns,
                    thread_cpu_ns=self.cpu_ns)
