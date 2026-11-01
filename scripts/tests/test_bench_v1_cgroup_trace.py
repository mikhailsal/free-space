"""Mocked cgroup trace tests: no real cgroup or application is read."""
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bench_v1_cgroup_trace as trace


class Clock:
    def __init__(self):
        self.now = 0

    def __call__(self):
        return self.now


def values():
    result = {key: '0' for key in trace.FILES}
    result['memory.stat'] = '\n'.join(f'{key} {i}' for i, key in enumerate(trace.STAT_KEYS))
    event = '\n'.join(f'{key} 0' for key in trace.EVENT_KEYS)
    result['memory.events'] = event
    result['memory.events.local'] = event.replace('max 0', 'max 7')
    psi = 'some avg10=0.00 avg60=0.00 avg300=0.00 total=9\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=3'
    result['memory.pressure'] = psi
    result['io.pressure'] = psi
    result['io.stat'] = '7:0 rbytes=1 wbytes=2 rios=3 wios=4'
    return result


class TraceTests(unittest.TestCase):
    def make(self, root, data=None):
        clock = Clock()
        raw = values() if data is None else data
        def read(_, name):
            value = raw[name]
            if isinstance(value, Exception):
                raise value
            return value
        return trace.CgroupTrace('/not-a-cgroup', root, read=read, clock=clock, cpu_clock=lambda: 0), clock

    def test_default_construction_does_no_io_or_thread_work(self):
        with patch('os.open', side_effect=AssertionError('unexpected I/O')):
            obj = trace.CgroupTrace('/not-a-cgroup', '/not-a-directory')
            self.assertIsNone(obj.next_ns)
            self.assertFalse(obj.result()['valid'])

    def test_final_sample_covers_due_tick_only_within_tolerance(self):
        for offset, covered, missed in ((-1, False, 0), (trace.LATE_TOLERANCE_NS, True, 0),
                                        (trace.LATE_TOLERANCE_NS + 1, False, 1)):
            with self.subTest(offset=offset), tempfile.TemporaryDirectory() as tmp:
                obj, clock = self.make(tmp)
                obj.start()
                obj.tick()
                clock.now = trace.INTERVAL_NS + offset
                result = obj.stop()
                self.assertEqual(result['missed'], missed)
                self.assertIs(result['final_covers_due_tick'], covered)
                self.assertEqual(result['final_covered_deadline_ns'], trace.INTERVAL_NS if covered else None)
                sample = [json.loads(line) for line in (Path(tmp) / 'cgroup-trace.jsonl').read_text().splitlines()][-2]
                self.assertIs(sample['final_covers_due_tick'], covered)

    def test_lifecycle_before_start_and_repeated_start(self):
        with tempfile.TemporaryDirectory() as tmp:
            obj, _ = self.make(tmp)
            self.assertFalse(obj.stop()['valid'])
            with self.assertRaisesRegex(RuntimeError, 'already started or finished'):
                obj.start()
            other, _ = self.make(tmp)
            self.assertTrue(other.start())
            with self.assertRaisesRegex(RuntimeError, 'already started or finished'):
                other.start()
            other.stop()

    def test_cadence_misses_raw_fields_and_closes_without_thread(self):
        with tempfile.TemporaryDirectory() as tmp:
            obj, clock = self.make(tmp)
            self.assertTrue(obj.start())
            obj.tick()
            clock.now = trace.INTERVAL_NS - 1
            obj.tick()
            clock.now = trace.INTERVAL_NS * 3 + 1
            obj.tick()
            result = obj.stop()
            self.assertTrue(result['valid'])
            self.assertEqual((result['samples'], result['attempted'], result['missed'], result['late']), (3, 3, 2, 0))
            self.assertEqual(result['max_lateness_ns'], 1)
            self.assertIsNone(obj.fd)
            output = Path(tmp, 'cgroup-trace.jsonl')
            records = [json.loads(line) for line in output.read_text().splitlines()]
            self.assertEqual(result['bytes'], output.stat().st_size)
            self.assertEqual(records[0]['values']['memory.peak'], '0')
            self.assertIn('max 7', records[0]['values']['memory.events.local'])
            self.assertIn('max 0', records[0]['values']['memory.events'])
            self.assertEqual(records[0]['unavailable'], {})
            self.assertEqual(records[-2]['kind'], 'final')
            self.assertEqual(records[-1]['type'], 'summary')
            self.assertEqual(records[-1]['bytes_before_summary'], sum(len((json.dumps(r, separators=(',', ':')) + '\n').encode()) for r in records[:-1]))
            obj.tick()
            self.assertEqual(obj.stop(), result)

    def test_missing_and_malformed_are_explicit_and_not_zero(self):
        data = values()
        data['memory.events.local'] = FileNotFoundError('missing')
        data['memory.current'] = 'not-a-number'
        with tempfile.TemporaryDirectory() as tmp:
            obj, _ = self.make(tmp, data)
            self.assertTrue(obj.start())
            obj.tick()
            result = obj.stop()
            self.assertFalse(result['valid'])
            self.assertEqual((result['missing'], result['errors']), (2, 2))
            record = json.loads(Path(tmp, 'cgroup-trace.jsonl').read_text().splitlines()[0])
            self.assertIsNone(record['values']['memory.events.local'])
            self.assertEqual(record['values']['memory.current'], 'not-a-number')
            self.assertEqual(record['unavailable']['memory.events.local'], 'missing')
            self.assertEqual(record['unavailable']['memory.current'], 'invalid_content')

    def test_final_missed_ticks_and_incomplete_numeric_fields(self):
        data = values()
        data['memory.stat'] = 'anon 4\nfile 5'
        data['io.pressure'] = 'some avg10=0 avg60=0 avg300=0 total=3'
        with tempfile.TemporaryDirectory() as tmp:
            obj, clock = self.make(tmp, data)
            self.assertTrue(obj.start())
            obj.tick()
            clock.now = 2 * trace.INTERVAL_NS + 1
            result = obj.stop()
            self.assertEqual(result['missed'], 2)
            self.assertEqual(result['errors'], 4)
            self.assertEqual(result['ended_ns'], clock.now)
            self.assertFalse(result['valid'])
            record = json.loads(Path(tmp, 'cgroup-trace.jsonl').read_text().splitlines()[0])
            self.assertEqual(record['values']['memory.stat'], 'anon 4\nfile 5')
            self.assertEqual(record['unavailable']['memory.stat'], 'invalid_content')

    def test_final_sample_and_meaningful_lateness(self):
        data = values()
        with tempfile.TemporaryDirectory() as tmp:
            obj, clock = self.make(tmp, data)
            self.assertTrue(obj.start())
            obj.tick()
            clock.now = trace.INTERVAL_NS + trace.LATE_TOLERANCE_NS + 1
            obj.tick()
            data['memory.peak'] = '9000'
            data['memory.events.local'] = data['memory.events.local'].replace('max 7', 'max 8')
            result = obj.stop()
            self.assertTrue(result['valid'])
            self.assertEqual((result['late'], result['max_lateness_ns']), (1, trace.LATE_TOLERANCE_NS + 1))
            records = [json.loads(line) for line in Path(tmp, 'cgroup-trace.jsonl').read_text().splitlines()]
            self.assertEqual(records[1]['actual_ns'] - records[1]['requested_ns'], records[1]['lateness_ns'])
            self.assertEqual(records[-2]['kind'], 'final')
            self.assertEqual(records[-2]['values']['memory.peak'], '9000')
            self.assertIn('max 8', records[-2]['values']['memory.events.local'])

    def test_summary_identifies_preclose_status(self):
        with tempfile.TemporaryDirectory() as tmp:
            obj, _ = self.make(tmp)
            self.assertTrue(obj.start())
            obj.tick()
            fd = obj.fd
            close = os.close
            def failing_close(actual):
                close(actual)
                if actual == fd:
                    raise OSError('close')
            with patch('bench_v1_cgroup_trace.os.close', side_effect=failing_close):
                result = obj.stop()
            self.assertFalse(result['valid'])
            self.assertEqual(result['errors'], 1)
            summary = json.loads(Path(tmp, 'cgroup-trace.jsonl').read_text().splitlines()[-1])
            self.assertTrue(summary['valid_before_close'])
            self.assertEqual(summary['errors_before_close'], 0)

    def test_observed_bare_and_extension_only_io_rows_then_full_counters(self):
        observed = '7:0 \n8:0 rbytes=17690624 wbytes=0 rios=600 wios=0 dbytes=0 dios=0'
        raw = values()
        raw['io.stat'] = observed
        with tempfile.TemporaryDirectory() as tmp:
            obj, clock = self.make(tmp, raw)
            self.assertTrue(obj.start())
            obj.tick()
            raw['io.stat'] = '7:0 depth=max cost.vrate=12.50\n8:0 rbytes=1 wbytes=2 rios=3 wios=4'
            clock.now = trace.INTERVAL_NS
            obj.tick()
            raw['io.stat'] = '7:0 rbytes=1 wbytes=2 rios=3 wios=4'
            clock.now += 1
            self.assertTrue(obj.stop()['valid'])
            records = [json.loads(line) for line in Path(tmp, 'cgroup-trace.jsonl').read_text().splitlines()]
            self.assertEqual(records[0]['values']['io.stat'], observed)
            self.assertEqual(records[0]['io_stat_unreported_devices'], ['7:0'])
            self.assertEqual(records[1]['io_stat_unreported_devices'], ['7:0'])
            self.assertEqual(records[2]['io_stat_unreported_devices'], [])
            self.assertEqual(trace._io(observed)['7:0'], {})

    def test_malformed_counters_are_preserved_and_invalid(self):
        raw = values()
        cases = [('memory.pressure', raw['memory.pressure'].replace('avg10=0.00', 'avg10=0.00 avg10=0.01', 1)),
                 ('io.stat', '7:0 rbytes=1 wbytes=2'),
                 ('io.stat', '7:0 rbytes=1 rbytes=2 wbytes=2 rios=3 wios=4'),
                 ('io.stat', '7:0 rbytes=-1 wbytes=2 rios=3 wios=4'),
                 ('io.stat', '7:0 =invalid'),
                 ('io.stat', '7:0 dbytes=5 dios=1'),
                 ('io.stat', '7:0 rbytes=1 wbytes=2 rios=3 wios=4 dbytes=-1'),
                 ('io.stat', '7:0 depth=max\n7:0'),
                 ('memory.current', '-1'),
                 ('memory.peak', 'nan'),
                 ('memory.stat', raw['memory.stat'] + '\nanon 5'),
                 ('memory.stat', raw['memory.stat'].replace('anon 0', 'anon -1', 1))]
        for name, bad in cases:
            with self.subTest(name=name, bad=bad), tempfile.TemporaryDirectory() as tmp:
                data = values()
                data[name] = bad
                obj, _ = self.make(tmp, data)
                self.assertTrue(obj.start())
                obj.tick()
                self.assertFalse(obj.stop()['valid'])
                record = json.loads(Path(tmp, 'cgroup-trace.jsonl').read_text().splitlines()[0])
                self.assertEqual(record['values'][name], bad)
                self.assertEqual(record['unavailable'][name], 'invalid_content')
        trace._validate('io.stat', '')
        trace._validate('io.stat', '7:0 rbytes=1 wbytes=2 rios=3 wios=4')
        trace._validate('io.stat', '7:0 rbytes=1 wbytes=2 rios=3 wios=4 dbytes=9')
        trace._validate('io.stat', '7:0 rbytes=1 wbytes=2 rios=3 wios=4 depth=max cost.vrate=12.50')

    def test_caps_report_truncation_and_stay_bounded(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(trace, 'MAX_SAMPLES', 1):
            obj, clock = self.make(tmp)
            self.assertTrue(obj.start())
            obj.tick()
            clock.now = trace.INTERVAL_NS
            obj.tick()
            result = obj.stop()
            self.assertTrue(result['truncated'])
            self.assertFalse(result['valid'])
            self.assertEqual(result['samples'], 1)
        with tempfile.TemporaryDirectory() as tmp, patch.object(trace, 'MAX_OUTPUT', trace.FINAL_RESERVE + 10):
            obj, _ = self.make(tmp)
            self.assertTrue(obj.start())
            obj.tick()
            result = obj.stop()
            self.assertTrue(result['truncated'])
            self.assertLessEqual(Path(tmp, 'cgroup-trace.jsonl').stat().st_size, trace.FINAL_RESERVE + 10)
        with tempfile.TemporaryDirectory() as tmp, patch.object(trace, 'MAX_RECORD_BYTES', 512):
            obj, _ = self.make(tmp)
            self.assertTrue(obj.start())
            obj.tick()
            result = obj.stop()
            self.assertTrue(result['truncated'])
            self.assertFalse(result['valid'])
            self.assertLessEqual(result['bytes'], trace.MAX_OUTPUT)

    def test_existing_and_symlink_output_never_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp, 'cgroup-trace.jsonl')
            output.write_bytes(b'foreign')
            obj, _ = self.make(tmp)
            self.assertFalse(obj.start())
            self.assertEqual(output.read_bytes(), b'foreign')
            output.unlink()
            target = Path(tmp, 'foreign')
            target.write_bytes(b'foreign')
            output.symlink_to(target)
            obj, _ = self.make(tmp)
            self.assertFalse(obj.start())
            self.assertEqual(target.read_bytes(), b'foreign')

    def test_write_error_invalidates_diagnostic_without_throwing(self):
        with tempfile.TemporaryDirectory() as tmp:
            obj, _ = self.make(tmp)
            self.assertTrue(obj.start())
            with patch('bench_v1_cgroup_trace.os.write', side_effect=OSError('disk')):
                obj.tick()
                result = obj.stop()
            self.assertFalse(result['valid'])
            self.assertGreaterEqual(result['errors'], 1)
            self.assertIsNone(obj.fd)

    def test_open_and_partial_write_failures_are_diagnostic(self):
        with tempfile.TemporaryDirectory() as tmp:
            obj, _ = self.make(tmp)
            with patch('bench_v1_cgroup_trace.os.open', side_effect=OSError('open')):
                self.assertFalse(obj.start())
            self.assertFalse(obj.stop()['valid'])
        with tempfile.TemporaryDirectory() as tmp:
            obj, _ = self.make(tmp)
            self.assertTrue(obj.start())
            real_write = os.write
            calls = []
            def partial(fd, data):
                calls.append(1)
                if len(calls) == 1:
                    return real_write(fd, data[:2])
                raise OSError('disk')
            with patch('bench_v1_cgroup_trace.os.write', side_effect=partial):
                obj.tick()
            self.assertFalse(obj.stop()['valid'])
            self.assertEqual(obj.result()['errors'], 1)
            self.assertTrue(obj.result()['truncated'])
            for line in Path(tmp, 'cgroup-trace.jsonl').read_text().splitlines():
                json.loads(line)

    def test_failed_write_rollback_never_appends_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            obj, _ = self.make(tmp)
            self.assertTrue(obj.start())
            real_write = os.write
            calls = []
            def partial(fd, data):
                calls.append(1)
                if len(calls) == 1:
                    return real_write(fd, data[:2])
                raise OSError('disk')
            with patch('bench_v1_cgroup_trace.os.write', side_effect=partial), \
                 patch('bench_v1_cgroup_trace.os.ftruncate', side_effect=OSError('rollback')):
                obj.tick()
            result = obj.stop()
            self.assertFalse(result['valid'])
            self.assertTrue(obj.write_broken)
            self.assertEqual(Path(tmp, 'cgroup-trace.jsonl').read_bytes(), b'{"')

    def test_read_limit_and_no_follow_use_only_fake_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'memory.current').write_text('9' * (trace.MAX_READ + 1))
            with self.assertRaisesRegex(ValueError, 'exceeds limit'):
                trace._read(root, 'memory.current')
            (root / 'memory.current').unlink()
            (root / 'other').write_text('1')
            (root / 'memory.current').symlink_to(root / 'other')
            with self.assertRaises(OSError):
                trace._read(root, 'memory.current')


if __name__ == '__main__':
    unittest.main()
