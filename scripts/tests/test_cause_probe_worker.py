"""Private tiny fixtures and mocked workers only; never launch subprocesses."""
import ctypes
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import cause_probe as probe
import cause_worker as worker


def spec(mode='names', sampler='reserve'):
    return dict(protocol=worker.PROTOCOL, id='test-arm', nonce='a'*32, mode=mode,
                sampler=sampler, expected_entries=0 if mode == 'idle' else 6,
                memory_mib=4096, cpus=2, workers=1, timeout_seconds=1,
                io_limits={}, initial_snapshot_allocated_bytes=0,
                minimum_free_bytes=2**31)


def counters():
    return {'memory.current': '100', 'memory.peak': '200', 'memory.max': str(4096*2**20),
            'memory.swap.current': '0', 'memory.swap.peak': '0', 'memory.swap.max': '0',
            'memory.stat': 'anon 1\nfile 2\nkernel 3\nslab_reclaimable 2\nslab_unreclaimable 1',
            'memory.events': 'max 0\noom 0\noom_kill 0', 'cpu.max': '200000 100000',
            'cpu.stat': 'usage_usec 1', 'io.max': '', 'io.stat': '7:0\n8:0 rbytes=1',
            'memory.pressure': 'some avg10=0 avg60=0 avg300=0 total=0\nfull avg10=0 avg60=0 avg300=0 total=0',
            'io.pressure': 'some avg10=0 avg60=0 avg300=0 total=1\nfull avg10=0 avg60=0 avg300=0 total=1'}


def report(mode='names'):
    return dict(protocol=worker.PROTOCOL, mode=mode, entries=6, regular_files=3,
                directories=3, directory_opens=3, root_lstat_calls=1,
                entry_stat_calls=5 if mode == 'stat' else 0,
                path_kind_digest='b'*64, validated=True, traversal=True)


def timing():
    return dict(elapsed_seconds=.1, peak_rss_kib=100, inputs=0, outputs=0, exit_code=0)


class ProbeTests(unittest.TestCase):
    def test_streaming_raw_names_hardlinks_and_stat_counts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = os.fsencode(tmp)
            os.mkdir(root + b'/one')
            os.mkdir(root + b'/one/two')
            fd = os.open(root + b'/one/raw\xff', os.O_CREAT | os.O_WRONLY, 0o600)
            os.close(fd)
            os.link(root + b'/one/raw\xff', root + b'/alias')
            fd = os.open(root + b'/ordinary', os.O_CREAT | os.O_WRONLY, 0o600)
            os.close(fd)
            real_stat = os.stat
            with patch('cause_probe.os.stat', wraps=real_stat) as calls:
                names = probe.probe(root, 'names')
                calls.assert_not_called()
                metadata = probe.probe(root, 'stat')
                self.assertEqual(calls.call_count, 5)
                self.assertTrue(all(c.kwargs == {'follow_symlinks': False} for c in calls.call_args_list))
            self.assertEqual(names['path_kind_digest'], metadata['path_kind_digest'])
            for result in (names, metadata):
                self.assertEqual((result['entries'], result['regular_files'], result['directories']), (6, 3, 3))
                self.assertEqual(result['root_lstat_calls'], 1)
                self.assertEqual(result['directory_opens'], 3)
            self.assertEqual(names['entry_stat_calls'], 0)
            self.assertEqual(metadata['entry_stat_calls'], 5)

    def test_idle_never_opens_or_stats_root(self):
        with patch('cause_probe.time.sleep') as sleep, patch('cause_probe.os.lstat') as stat_call, patch('cause_probe.Directory') as directory:
            result = probe.probe('/nonexistent', 'idle')
        sleep.assert_called_once_with(1)
        stat_call.assert_not_called()
        directory.assert_not_called()
        self.assertEqual(result['entries'], 0)
        self.assertFalse(result['traversal'])

    def fake_directory(self, kind=0, reclen=24, name=b'x'):
        record = probe.Dirent()
        record.ino, record.type, record.reclen, record.name = 12, kind, reclen, name
        directory = probe.Directory.__new__(probe.Directory)
        directory.handle = 1
        directory.lib = SimpleNamespace(readdir=Mock(return_value=ctypes.pointer(record)), closedir=Mock(return_value=0))
        return directory

    def test_unknown_type_has_no_stat_fallback(self):
        with patch('cause_probe.os.stat') as stat_call:
            with self.assertRaisesRegex(ValueError, 'DT_UNKNOWN'):
                next(self.fake_directory())
            stat_call.assert_not_called()

    def test_rejects_bad_record_and_unexpected_type(self):
        for directory in (self.fake_directory(8, 22), self.fake_directory(8, 288), self.fake_directory(10)):
            with self.assertRaises(ValueError):
                next(directory)
        directory = self.fake_directory(8, 280, b'x'*255)
        ctypes.memset(ctypes.addressof(directory.lib.readdir.return_value.contents) + 19, 120, 261)
        with self.assertRaisesRegex(ValueError, 'unterminated'):
            next(directory)

    def test_inode_mismatch_fails_and_closes_all_streams(self):
        iterator = Mock()
        iterator.__iter__ = Mock(return_value=iterator)
        iterator.__next__ = Mock(side_effect=[(b'x', 8, 12)])
        factory = Mock(return_value=iterator)
        with patch('cause_probe.os.lstat', return_value=SimpleNamespace(st_mode=0o40755)), patch('cause_probe.os.stat', return_value=SimpleNamespace(st_mode=0o100600, st_ino=13)):
            with self.assertRaisesRegex(ValueError, 'inode or type'):
                probe.probe('/private', 'stat', directory=factory)
        iterator.close.assert_called_once()

    def test_digest_is_independent_of_directory_order(self):
        class Stream:
            def __init__(self, records):
                self.records = iter(records)
            def __next__(self):
                return next(self.records)
            def close(self):
                pass
        values = [(b'a', 8, 1), (b'raw\xff', 8, 2)]
        with patch('cause_probe.os.lstat', return_value=SimpleNamespace(st_mode=0o40755)):
            first = probe.probe('/private', 'names', lambda _: Stream(values))
            second = probe.probe('/private', 'names', lambda _: Stream(list(reversed(values))))
        self.assertEqual(first['path_kind_digest'], second['path_kind_digest'])


class WorkerTests(unittest.TestCase):
    def setUp(self):
        self.no_process = patch('cause_worker.subprocess.Popen', side_effect=AssertionError('unmocked process forbidden'))
        self.no_process.start()
        self.addCleanup(self.no_process.stop)

    def helper(self, group):
        occupancy = lambda _: {'total': {'logical_bytes': 1, 'allocated_bytes': 1}}
        return SimpleNamespace(cgroup=lambda: group, cg_read=lambda _: counters(),
                               verify_io=lambda values, config: None, occupancy=occupancy,
                               file_occupancy=lambda _: {'snapshot.db': {'logical_bytes': 1, 'allocated_bytes': 1}},
                               filesystem_used=lambda _: 1,
                               dump=lambda path, data: path.write_text(json.dumps(data)))

    def test_spec_github_caps_and_types(self):
        env = dict(GITHUB_ACTIONS='true', RUNNER_ENVIRONMENT='github-hosted')
        with patch.dict(os.environ, env):
            worker.validate_spec(spec())
            for key, value in (('memory_mib', 2048), ('cpus', True), ('workers', 2), ('expected_entries', True), ('timeout_seconds', 1201), ('minimum_free_bytes', 1)):
                invalid = dict(spec(), **{key: value})
                with self.assertRaises(ValueError):
                    worker.validate_spec(invalid)
        with patch.dict(os.environ, {'GITHUB_ACTIONS': 'false'}):
            with self.assertRaisesRegex(ValueError, 'GitHub-hosted'):
                worker.validate_spec(spec())

    def test_capture_raw_bare_io_rows_and_rejects_bad_caps_missing_data(self):
        with tempfile.TemporaryDirectory() as tmp:
            group = Path(tmp)
            (group / 'memory.events.local').write_text('max 0\noom 0\noom_kill 0')
            helper = self.helper(group)
            capture = worker.snapshot(helper, group, spec())
            self.assertEqual(capture['values']['io.stat'], '7:0\n8:0 rbytes=1')
            self.assertLessEqual(capture['start_ns'], capture['end_ns'])
            for key, value in (('memory.max', '1'), ('memory.swap.max', '1'), ('cpu.max', '100000 100000'), ('memory.stat', 'anon 1')):
                helper.cg_read = lambda _, k=key, v=value: dict(counters(), **{k: v})
                with self.assertRaises(ValueError):
                    worker.snapshot(helper, group, spec())
            helper.cg_read = lambda _: {key: value for key, value in counters().items() if key != 'memory.current'}
            with self.assertRaisesRegex(ValueError, 'missing'):
                worker.snapshot(helper, group, spec())
            helper.cg_read = lambda _: counters()
            helper.verify_io = Mock(side_effect=RuntimeError('I/O mismatch'))
            with self.assertRaisesRegex(RuntimeError, 'I/O mismatch'):
                worker.snapshot(helper, group, spec())

    def test_report_counts_and_status_are_typed(self):
        worker.validate_report(report(), timing(), spec())
        worker.validate_report(report('stat'), timing(), spec('stat'))
        for invalid in (dict(report(), entries=True), dict(report(), directories=4), dict(report(), entry_stat_calls=1), dict(report(), validated=1)):
            with self.assertRaises(ValueError):
                worker.validate_report(invalid, timing(), spec())
        for invalid in (dict(timing(), exit_code=False), dict(timing(), peak_rss_kib=True), dict(timing(), elapsed_seconds=float('nan'))):
            with self.assertRaises(ValueError):
                worker.validate_report(report(), invalid, spec())
        scan = dict(entries=6, errors=0, incomplete=False, workers=1)
        worker.validate_report(scan, timing(), spec('scan'))
        with self.assertRaisesRegex(ValueError, 'scan status'):
            worker.validate_report(dict(scan, errors=False), timing(), spec('scan'))

    def test_sampler_modes_keep_reserve_safety_with_unavailable_fields(self):
        with tempfile.TemporaryDirectory() as tmp:
            group = Path(tmp)
            (group / 'memory.current').write_text('100')
            usage = SimpleNamespace(f_bavail=2**31, f_frsize=1)
            helper = self.helper(group)
            samples = []
            with patch('cause_worker.os.statvfs', return_value=usage) as check:
                for mode in ('full', 'reserve'):
                    sampler = worker.Sampler(helper, group, group, spec(sampler=mode), Mock())
                    sampler.tick()
                    samples.append(sampler)
            self.assertEqual(check.call_count, 2)
            self.assertEqual(samples[0].memory, 100)
            self.assertEqual(samples[0].filesystem_peak, 1)
            self.assertIsNone(samples[1].memory)
            self.assertIsNone(samples[1].disk)
            self.assertIsNone(samples[1].files)
            self.assertIsNone(samples[1].filesystem_peak)
            for sampler in samples:
                with patch('cause_worker.os.statvfs', return_value=SimpleNamespace(f_bavail=0, f_frsize=1)):
                    with self.assertRaisesRegex(RuntimeError, 'reserve'):
                        sampler.tick()

    def test_ack_timeout_mismatch_and_exclusive_publication(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            ack = directory / 'host-ack.json'
            with patch('cause_worker.time.monotonic', side_effect=[0, 91]):
                with self.assertRaises(TimeoutError):
                    worker.await_ack(ack, spec())
            ack.write_text(json.dumps(dict(protocol=worker.PROTOCOL, id='test-arm', nonce='wrong')))
            with self.assertRaisesRegex(ValueError, 'mismatch'):
                worker.await_ack(ack, spec())
            ack.write_text(json.dumps(dict(protocol=worker.PROTOCOL, id='test-arm', nonce='a'*32)))
            self.assertTrue(worker.await_ack(ack, spec()))
            helper = self.helper(directory)
            output = directory / 'receipt.json'
            worker.publish(helper, output, {'ok': True})
            with self.assertRaises(FileExistsError):
                worker.publish(helper, output, {'ok': False})
            self.assertEqual(json.loads(output.read_text()), {'ok': True})

    def test_sampler_errors_kill_and_missed_ticks_are_recorded(self):
        helper = self.helper(Path('/private'))
        kill = Mock()
        sampler = worker.Sampler(helper, Path('/private'), Path('/private'), spec(), kill)
        with patch.object(sampler, 'tick', side_effect=OSError('read failed')):
            sampler.run()
        kill.assert_called_once()
        self.assertEqual(sampler.finish()['capture_errors'], 1)
        sampler = worker.Sampler(helper, Path('/private'), Path('/private'), spec(), Mock())
        with patch('cause_worker.time.monotonic', side_effect=[0, .061, .061]), patch.object(sampler, 'tick', side_effect=sampler.stop_event.set):
            sampler.run()
        self.assertEqual(sampler.missed, 3)
        self.assertEqual(sampler.late, 1)

    def test_reserve_coverage_accepts_missed_ticks_but_rejects_250ms_gaps(self):
        with tempfile.TemporaryDirectory() as tmp:
            group = Path(tmp)
            (group / 'memory.current').write_text('100')
            usage = SimpleNamespace(f_bavail=2**31, f_frsize=1)
            # 25-ms scheduling delay misses a nominal tick; the check still covers
            # the required 250-ms bound. Test tick and final boundary failures.
            for mode in ('full', 'reserve'):
                for tick_ns, end_ns, valid in ((25_000_000, 25_000_000, True),
                                               (250_000_000, 250_000_000, True),
                                               (275_000_000, 275_000_000, False),
                                               (25_000_000, 300_000_000, False)):
                    with patch('cause_worker.time.monotonic_ns', side_effect=[0, 0, tick_ns, end_ns]), patch('cause_worker.os.statvfs', return_value=usage):
                        sampler = worker.Sampler(self.helper(group), group, group, spec(sampler=mode), Mock())
                        sampler.tick()
                        real_tick = sampler.tick
                        def final_tick():
                            real_tick()
                            sampler.stop_event.set()
                        elapsed = tick_ns / 1e9
                        with patch('cause_worker.time.monotonic', side_effect=[0, elapsed, elapsed]), patch.object(sampler, 'tick', side_effect=final_tick):
                            sampler.run()
                        result = sampler.finish()
                    self.assertGreater(result['missed_ticks'], 0)
                    self.assertGreater(result['late_ticks'], 0)
                    self.assertEqual(result['coverage_valid'], valid)
                    self.assertEqual(result['max_tick_gap_ns'], max(tick_ns, end_ns-tick_ns))
                    self.assertEqual(result['reserve_max_gap_ns'], 250_000_000)

    def test_reserve_failure_prevents_process_launch_in_both_modes(self):
        with tempfile.TemporaryDirectory() as tmp:
            group = Path(tmp)
            (group / 'memory.events.local').write_text('max 0\noom 0\noom_kill 0')
            for sampler in ('full', 'reserve'):
                with patch('cause_worker.os.statvfs', return_value=SimpleNamespace(f_bavail=0, f_frsize=1)), patch('cause_worker.subprocess.Popen') as launch:
                    with self.assertRaisesRegex(RuntimeError, 'reserve'):
                        worker.measure(spec(sampler=sampler), group, self.helper(group))
                    launch.assert_not_called()

    def measure_fixture(self, mode='names', sampler='reserve', timeout=False, kill_failure=False):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        directory = Path(temporary.name)
        group = directory / 'group'
        group.mkdir()
        (group / 'memory.events.local').write_text('max 0\noom 0\noom_kill 0')
        (group / 'memory.current').write_text('100')
        helper = self.helper(group)
        fake = Mock(pid=12345, returncode=0)
        fake.poll.return_value = None if timeout else 0

        def launch(command, **kwargs):
            kwargs['stdout'].write(json.dumps(report(mode)))
            (directory / 'time.log').write_text(json.dumps(timing()))
            return fake

        class Timer:
            def __init__(self, _, callback):
                self.callback = callback
            def start(self):
                if timeout:
                    self.callback()
            def cancel(self):
                pass
            def join(self, timeout):
                pass
            def is_alive(self):
                return False

        usage = SimpleNamespace(f_bavail=2**31, f_frsize=1)
        with patch('cause_worker.subprocess.Popen', side_effect=launch), patch('cause_worker.threading.Timer', Timer), patch('cause_worker.os.statvfs', return_value=usage), patch('cause_worker.os.killpg', side_effect=PermissionError('denied') if kill_failure else None) as kill:
            metrics = worker.measure(spec(mode, sampler), directory, helper)
        return metrics, kill, fake

    def test_common_measurement_full_and_reserve(self):
        for sampler in ('full', 'reserve'):
            metrics, kill, fake = self.measure_fixture(sampler=sampler)
            self.assertTrue(metrics['validated'], metrics['errors'])
            self.assertFalse(metrics['semantic_validated'])
            self.assertFalse(metrics['qualification'])
            self.assertEqual(metrics['kernel_memory_peak_bytes'], 200)
            self.assertEqual(metrics['sampling']['sampled_memory_peak'], 100 if sampler == 'full' else None)
            self.assertEqual(set(metrics['endpoint_before']), set(metrics['endpoint_after']))
            kill.assert_not_called()
            fake.wait.assert_called_once_with(timeout=6)

    def test_watchdog_timeout_and_kill_failure_invalidate(self):
        metrics, kill, _ = self.measure_fixture(timeout=True, kill_failure=True)
        self.assertFalse(metrics['validated'])
        self.assertTrue(metrics['timed_out'])
        self.assertTrue(any('watchdog kill' in item for item in metrics['errors']))
        kill.assert_called_once_with(12345, worker.signal.SIGKILL)

    def test_run_failure_receipts_and_ack_outside_measurement(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            path = directory / 'spec.json'
            path.write_text(json.dumps(spec()))
            helper = self.helper(directory)
            env = dict(GITHUB_ACTIONS='true', RUNNER_ENVIRONMENT='github-hosted')
            def acknowledge(ack, _):
                self.assertTrue((ack.parent / 'metrics.json').exists())
                self.assertTrue((ack.parent / 'worker-ready.json').exists())
                raise TimeoutError('ack timeout')
            with patch.dict(os.environ, env), patch('cause_worker.legacy', return_value=helper), patch('cause_worker.measure', return_value=dict(cli_validated=True, validated=True)), patch('cause_worker.await_ack', side_effect=acknowledge):
                self.assertEqual(worker.run(path), 1)
            terminal = json.loads((directory / 'test-arm/worker-terminal.json').read_text())
            self.assertFalse(terminal['success'])
            self.assertFalse(terminal['acknowledged'])
            self.assertIn('ack timeout', terminal['errors'][0])


if __name__ == '__main__':
    unittest.main()
