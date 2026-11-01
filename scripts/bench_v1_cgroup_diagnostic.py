"""Opt-in worker trace session and read-only contrast validation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import time

from bench_v1_cgroup_trace import CgroupTrace, FILES, LATE_TOLERANCE_NS, MAX_OUTPUT, MAX_RECORD_BYTES, MAX_SAMPLES, _validate


class TraceSession:
    """Collector failures are diagnostic only; caller owns CLI success."""
    def __init__(self, group, output, snapshot):
        wall_start, cpu_start = time.monotonic_ns(), time.thread_time_ns()
        self.trace = CgroupTrace(group, output)
        self.output = output
        self.sha256 = None
        self.error = None
        self.devices = None
        try:
            self.devices = {'output_st_dev': output.stat().st_dev,
                            'snapshot_st_dev': snapshot.stat().st_dev}
            if not self.trace.start():
                self.error = 'trace start failed'
        except Exception as error:
            self.error = repr(error)
        self.setup_wall_ns = time.monotonic_ns() - wall_start
        self.setup_thread_cpu_ns = time.thread_time_ns() - cpu_start
        self.finalize_wall_ns = self.finalize_thread_cpu_ns = None

    def tick(self):
        if self.error is None:
            try:
                self.trace.tick()
            except Exception as error:
                self.error = repr(error)

    def stop(self):
        wall_start, cpu_start = time.monotonic_ns(), time.thread_time_ns()
        try:
            self.trace.stop()
            if self.error is None:
                self.sha256 = hashlib.sha256(_bounded_data(self.output / 'cgroup-trace.jsonl')).hexdigest()
        except Exception as error:
            self.error = repr(error)
        self.finalize_wall_ns = time.monotonic_ns() - wall_start
        self.finalize_thread_cpu_ns = time.thread_time_ns() - cpu_start

    def receipt(self, started, finished):
        try:
            result = self.trace.result()
            result.update(cli_start_ns=int(started * 1_000_000_000),
                          cli_end_ns=int(finished * 1_000_000_000), devices=self.devices,
                          error=self.error, sha256=self.sha256,
                          setup_wall_ns=self.setup_wall_ns, setup_thread_cpu_ns=self.setup_thread_cpu_ns,
                          finalize_wall_ns=self.finalize_wall_ns, finalize_thread_cpu_ns=self.finalize_thread_cpu_ns,
                          scope='worker cgroup includes CLI, helper, sampler and diagnostic; output may share storage')
            result['valid'] = result['valid'] and self.error is None and self.devices is not None
            return result
        except Exception as error:
            return {'valid': False, 'error': repr(error)}


def _read_json(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('missing or unsafe JSON: ' + str(path))
    return json.loads(path.read_text())


def _bounded_data(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise ValueError('trace is not a regular file')
        data = bytearray()
        while len(data) <= MAX_OUTPUT:
            chunk = os.read(fd, min(65536, MAX_OUTPUT + 1 - len(data)))
            if not chunk:
                break
            data.extend(chunk)
        if len(data) > MAX_OUTPUT:
            raise ValueError('trace exceeds output limit')
    finally:
        os.close(fd)
    return bytes(data)


def _trace(path, receipt):
    data = _bounded_data(path)
    if (len(data) != receipt.get('bytes') or not data.endswith(b'\n')
            or hashlib.sha256(data).hexdigest() != receipt.get('sha256')):
        raise ValueError('trace byte count or terminator mismatch')
    lines = data.splitlines(keepends=True)
    if not 3 <= len(lines) <= MAX_SAMPLES + 1:
        raise ValueError('trace sample count invalid')
    if any(len(line) > MAX_RECORD_BYTES for line in lines):
        raise ValueError('trace record exceeds limit')
    records = [json.loads(line) for line in lines]
    summary = records[-1]
    if summary.get('type') != 'summary' or summary.get('bytes_before_summary') != sum(map(len, lines[:-1])):
        raise ValueError('trace summary mismatch')
    if summary.get('valid_before_close') is not True or summary.get('errors_before_close') != 0:
        raise ValueError('invalid pre-close trace summary')
    for key in ('started_ns', 'ended_ns', 'interval_ns', 'samples', 'attempted', 'missed',
                'late', 'max_lateness_ns', 'late_tolerance_ns', 'missing', 'truncated', 'wall_ns', 'thread_cpu_ns',
                'final_covers_due_tick', 'final_covered_deadline_ns'):
        if type(summary.get(key)) is not type(receipt.get(key)) or summary.get(key) != receipt.get(key):
            raise ValueError('trace receipt mismatch: ' + key)
    for key in ('started_ns', 'ended_ns', 'interval_ns', 'samples', 'attempted', 'missed',
                'late', 'max_lateness_ns', 'late_tolerance_ns', 'missing', 'wall_ns', 'thread_cpu_ns'):
        if type(summary.get(key)) is not int or summary[key] < 0:
            raise ValueError('invalid numeric trace summary: ' + key)
    if summary['interval_ns'] != 5_000_000_000 or summary['late_tolerance_ns'] != LATE_TOLERANCE_NS:
        raise ValueError('invalid sampling interval')
    if (summary.get('samples') != len(records) - 1
            or summary.get('attempted') != summary.get('samples') or summary.get('missed') != 0
            or receipt.get('errors') != 0 or summary.get('missing') != 0 or summary.get('truncated') is not False):
        raise ValueError('trace incomplete')
    prior = summary['started_ns']
    latenesses = []
    sampled_currents = []
    for index, sample in enumerate(records[:-1]):
        begin, end = sample.get('start_ns'), sample.get('end_ns')
        if (sample.get('type') != 'sample' or type(begin) is not int or type(end) is not int
                or not prior <= begin <= end <= summary['ended_ns'] or sample.get('unavailable') != {}):
            raise ValueError('invalid sample ordering or unavailable data')
        final = index == len(records) - 2
        if sample.get('kind') != ('final' if final else 'periodic'):
            raise ValueError('missing or misplaced final sample')
        if final:
            deadline = summary['started_ns'] + index * summary['interval_ns']
            covered = deadline <= begin <= deadline + LATE_TOLERANCE_NS
            expected = deadline if covered else None
            if (type(sample.get('final_covers_due_tick')) is not bool
                    or sample['final_covers_due_tick'] != covered
                    or sample.get('final_covered_deadline_ns') != expected
                    or (covered and type(sample.get('final_covered_deadline_ns')) is not int)
                    or type(summary.get('final_covers_due_tick')) is not bool
                    or summary['final_covers_due_tick'] != covered
                    or summary.get('final_covered_deadline_ns') != expected
                    or (covered and type(summary.get('final_covered_deadline_ns')) is not int)
                    or begin >= deadline and not covered):
                raise ValueError('invalid final deadline coverage')
            if (sample.get('requested_ns') is not None or sample.get('lateness_ns') is not None
                    or sample.get('actual_ns') != begin):
                raise ValueError('invalid final timing')
        elif (type(sample.get('requested_ns')) is not int or type(sample.get('actual_ns')) is not int
              or type(sample.get('lateness_ns')) is not int
              or sample['requested_ns'] != summary['started_ns'] + index * summary['interval_ns']
              or sample['actual_ns'] - sample['requested_ns'] != sample['lateness_ns']
              or sample['lateness_ns'] < 0 or sample['actual_ns'] > begin):
            raise ValueError('invalid periodic timing')
        else:
            latenesses.append(sample['lateness_ns'])
        fields = sample.get('values')
        if not isinstance(fields, dict) or set(fields) != set(FILES):
            raise ValueError('missing cgroup files')
        for name, raw in fields.items():
            if not isinstance(raw, str) or len(raw.encode()) > 65536:
                raise ValueError('invalid cgroup value')
            _validate(name, raw)
        sampled_currents.append(int(fields['memory.current']))
        prior = end
    if summary['late'] != sum(v > LATE_TOLERANCE_NS for v in latenesses) or summary['max_lateness_ns'] != max(latenesses):
        raise ValueError('lateness summary mismatch')
    if (any(type(receipt.get(k)) is not int or receipt[k] < 0 for k in ('cli_start_ns', 'cli_end_ns'))
            or not (summary['started_ns'] <= receipt['cli_start_ns'] <= receipt['cli_end_ns'] <= records[-2]['start_ns'] <= summary['ended_ns'])):
        raise ValueError('CLI window outside trace window')
    return {'samples': summary['samples'], 'bytes': len(data), 'missed': summary['missed'],
            'max_record_bytes': max(map(len, lines[:-1])),
            'coverage_ns': summary['ended_ns'] - summary['started_ns'],
            'sampled_current_max': max(sampled_currents),
            'final_memory_peak': int(records[-2]['values']['memory.peak'])}


def analyze(root):
    from bench_v1_summary import summarize
    issues, details = [], []
    disclosure = ('Instrumentation contrast, not a noise band or product optimization. '
                  'Cgroup reads, JSON encoding and cache writes are cocharged to the worker. '
                  'The shared 20ms sampler can delay disk sampling and free-space reserve checks. '
                  'Output and snapshot devices can share a physical disk. Five-second points are '
                  'non-atomic; they provide no phase attribution or peak decomposition.')
    try:
        ordinary = summarize(root)
        if ordinary.get('valid') is not True or ordinary.get('issues'):
            raise ValueError('ordinary strict summary invalid')
        if _read_json(root / 'summary.json') != ordinary:
            raise ValueError('ordinary summary does not match raw receipts')
        manifest = _read_json(root / 'manifest.json')
        if (manifest.get('operation') != 'scan' or manifest.get('cgroup_trace_policy') !=
                {'mode': 'contrast', 'baseline-v1': False, 'candidate-v1': True}):
            raise ValueError('invalid diagnostic policy')
        provenance = manifest['provenance']
        if provenance['baseline-v1']['sha256'] != provenance['candidate-v1']['sha256']:
            raise ValueError('binary hashes differ')
        setup = root.with_name(root.name + '-setup')
        for ordinal in (1, 2):
            seed = _read_json(setup / f'seed-{ordinal}.json')
            if seed.get('cgroup_trace') is not None or seed.get('cgroup_trace_enabled') is not None:
                raise ValueError('seed tracing is not disabled')
        peaks = {}
        for ident in manifest['expected_arms']:
            directory = root / ident
            spec = _read_json(directory / 'spec.json')
            metric = _read_json(directory / 'metrics.json')
            if spec.get('cgroup_trace') != 'contrast':
                raise ValueError('arm diagnostic policy missing')
            enabled = spec.get('arm') == 'candidate-v1'
            if metric.get('cgroup_trace_enabled') is not enabled:
                raise ValueError('worker trace flag mismatch')
            peak = metric['cgroup_after']['memory.peak']
            if not isinstance(peak, str) or not peak.isascii() or not peak.isdecimal():
                raise ValueError('invalid kernel cgroup peak')
            peaks.setdefault(spec['pair'], {})[spec['arm']] = int(peak)
            path = directory / 'cgroup-trace.jsonl'
            if enabled:
                receipt = metric.get('cgroup_trace_receipt')
                if not isinstance(receipt, dict) or receipt.get('valid') is not True or receipt.get('error') is not None:
                    raise ValueError('invalid diagnostic receipt')
                devices = receipt.get('devices')
                if not isinstance(devices, dict) or any(type(devices.get(k)) is not int or devices[k] < 0
                                                        for k in ('output_st_dev', 'snapshot_st_dev')):
                    raise ValueError('missing output and snapshot device metadata')
                if any(type(receipt.get(k)) is not int or receipt[k] < 0 for k in
                       ('setup_wall_ns', 'setup_thread_cpu_ns', 'finalize_wall_ns', 'finalize_thread_cpu_ns')):
                    raise ValueError('missing diagnostic setup/finalize costs')
                if not isinstance(receipt.get('scope'), str) or not receipt['scope']:
                    raise ValueError('missing diagnostic scope')
                details.append(dict(id=ident, devices=devices, scope=receipt['scope']))
                detail = _trace(path, receipt)
                if detail['final_memory_peak'] > int(peak):
                    raise ValueError('final trace peak exceeds kernel endpoint peak')
                details[-1].update(dict(**detail,
                                    sampling_wall_ns=receipt['wall_ns'],
                                    sampling_thread_cpu_ns=receipt['thread_cpu_ns'],
                                    setup_wall_ns=receipt['setup_wall_ns'],
                                    setup_thread_cpu_ns=receipt['setup_thread_cpu_ns'],
                                    finalize_wall_ns=receipt['finalize_wall_ns'],
                                    finalize_thread_cpu_ns=receipt['finalize_thread_cpu_ns']))
            elif path.exists() or path.is_symlink() or metric.get('cgroup_trace_receipt') is not None:
                raise ValueError('unexpected baseline trace')
        peak_pairs = []
        for pair, row in sorted(peaks.items()):
            if set(row) != {'baseline-v1', 'candidate-v1'}:
                raise ValueError('incomplete cgroup peak pair')
            peak_pairs.append({'pair': pair, 'off': row['baseline-v1'], 'on': row['candidate-v1'],
                               'delta': row['candidate-v1'] - row['baseline-v1']})
        return {'valid': True, 'issues': [], 'policy': 'contrast', 'diagnostic_arms': details,
                'cgroup_peak_bytes': peak_pairs,
                'ordinary_summary': ordinary, 'interpretation': disclosure}
    except (OSError, ValueError, KeyError, TypeError, UnicodeError, OverflowError, RecursionError, AttributeError) as error:
        issues.append(str(error))
    return {'valid': False, 'issues': issues, 'policy': 'contrast',
            'diagnostic_arms': details, 'interpretation': disclosure}


def markdown(result):
    state = 'VALID' if result['valid'] else 'INVALID'
    lines = [f"## Cgroup instrumentation contrast — {state}", '', result['interpretation'], '']
    for arm in result['diagnostic_arms']:
        devices = arm['devices']
        lines.append(f"- {arm['id']}: receipt.scope={arm['scope']}; output st_dev={devices['output_st_dev']}; "
                     f"snapshot st_dev={devices['snapshot_st_dev']}")
    if not result['diagnostic_arms']:
        lines.append('- Receipt scope and device numbers unavailable.')
    if result['issues']:
        lines.append('- Diagnostic issues: ' + '; '.join(result['issues']))
    return '\n'.join(lines) + '\n'


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args(argv)
    result = analyze(args.directory)
    path = args.directory / 'diagnostic-summary.json'
    with path.open('x') as out:
        json.dump(result, out, indent=2, allow_nan=False)
        out.write('\n')
    with (args.directory / 'diagnostic-summary.md').open('x') as out:
        out.write(markdown(result))
    print(markdown(result), end='')
    return 0 if result['valid'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
