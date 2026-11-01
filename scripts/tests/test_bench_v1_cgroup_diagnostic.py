"""Mocked tests for opt-in cgroup contrast; no app, Docker or cgroup access."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from bench_v1_cgroup_diagnostic import TraceSession, analyze, main
from bench_v1_pair import arguments, host
from bench_v1_runtime import inside
from bench_v1_summary import summarize
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_bench_v1_cgroup_trace import Clock, values
from test_bench_v1_summary import fixture, put


class DiagnosticTests(unittest.TestCase):
    def test_diagnostic_failure_fails_step_without_invalidating_ordinary_cleanup(self):
        script = (Path(__file__).resolve().parents[1] / 'bench-representative.sh').read_text()
        cleanup = script[script.index('finish_cleanup() {'):script.index('\ncleanup() {')]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'measurements').mkdir()
            shell = r'''
output=$TEST_OUTPUT
status=0
python3() {
  case "$1" in
    scripts/bench_v1_summary.py) echo ordinary-valid; return 0 ;;
    scripts/bench_v1_cgroup_diagnostic.py) printf '%s' "$TEST_BLOCK"; return "$TEST_DIAG_STATUS" ;;
    *) command python3 "$@" ;;
  esac
}
''' + cleanup + '\nfinish_cleanup\nexit "$status"\n'
            env = dict(os.environ, TEST_OUTPUT=tmp, BENCH_PROTOCOL='v1-pairs',
                       BENCH_CGROUP_TRACE='contrast', GITHUB_STEP_SUMMARY=str(root / 'summary'),
                       TEST_BLOCK='', TEST_DIAG_STATUS='1')
            result = subprocess.run(['bash', '-c', shell], env=env, capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 1, result.stderr)
            receipt = json.loads((root / 'measurements/runner-cleanup.json').read_text())
            self.assertEqual(receipt, {'success': True, 'exit_code': 0})
            self.assertIn('instrumentation contrast — INVALID', (root / 'summary').read_text())
            env.update(TEST_BLOCK='## Cgroup instrumentation contrast — VALID', TEST_DIAG_STATUS='0')
            result = subprocess.run(['bash', '-c', shell], env=env, capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('instrumentation contrast — VALID', (root / 'summary').read_text())

    def test_success_disclosure_is_visible_and_artifact_matches(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'measurements'
            self.make_case(root)
            self.assertEqual(main([str(root)]), 0)
            block = (root / 'diagnostic-summary.md').read_text()
            self.assertIn('instrumentation contrast — VALID', block)
            self.assertIn('not a noise band', block)
            self.assertIn('output st_dev=1; snapshot st_dev=1', block)
            self.assertIn('receipt.scope=', block)
            result = json.loads((root / 'diagnostic-summary.json').read_text())
            self.assertIn('cocharged', result['interpretation'])

    def test_workflow_binary_gate_actual_inline_code(self):
        workflow = (Path(__file__).resolve().parents[2] / '.github/workflows/representative-bench.yml').read_text()
        step = workflow.split('      - name: Verify identical binaries for cgroup trace contrast', 1)[1].split('\n  paired:', 1)[0]
        self.assertIn("if: ${{ inputs.cgroup_trace == 'contrast' }}", step)
        code = step.split("          python3 - <<'PY'\n", 1)[1].split('\n          PY', 1)[0]
        code = '\n'.join(line.removeprefix('          ') for line in code.splitlines())
        with tempfile.TemporaryDirectory() as tmp:
            env = dict(os.environ, CGROUP_TRACE='off')
            run = lambda: subprocess.run([sys.executable, '-c', code], cwd=tmp, env=env, capture_output=True, text=True)
            self.assertEqual(run().returncode, 0)
            binaries = Path(tmp) / 'binaries'
            binaries.mkdir()
            (binaries / 'baseline').write_bytes(b'same')
            (binaries / 'candidate').write_bytes(b'same')
            env['CGROUP_TRACE'] = 'contrast'
            self.assertEqual(run().returncode, 0)
            (binaries / 'candidate').write_bytes(b'different')
            self.assertNotEqual(run().returncode, 0)

    def test_default_worker_and_diagnostic_failure_preserve_cli_result(self):
        from types import SimpleNamespace
        spec = importlib.util.spec_from_file_location('legacy_test', Path(__file__).resolve().parents[1] / 'bench-v2-container.py')
        legacy = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(legacy)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'results').mkdir()
            (root / 'group').mkdir()
            (root / 'snapshot').mkdir()
            (root / 'group/memory.current').write_text('1')
            def local_path(value):
                return {'/results': root / 'results', '/scratch/snapshot': root / 'snapshot'}.get(str(value), Path(value))
            def popen(command, stdout, **_):
                stdout.write('{"entries": 3, "errors": 0}')
                Path(command[command.index('-o') + 1]).write_text('{"peak_rss_kib":1}\n')
                return SimpleNamespace(returncode=0, wait=lambda: 0, poll=lambda: 0)
            class Thread:
                def __init__(self, *args, **kwargs): pass
                def start(self): pass
                def join(self): pass
                def cancel(self): pass
            cgroup = {'memory.max': str(1024**3), 'memory.swap.max': '0', 'cpu.max': '200000 100000'}
            base = dict(id='off', single_operation=True, memory_mib=1024, cpus=2, backend='v1',
                        operation='scan', target='/x', timeout_seconds=10, expected_entries=3,
                        initial_snapshot_allocated_bytes=0)
            reader = Mock(return_value=cgroup)
            fake_trace = Mock()
            fake_trace.return_value.receipt.return_value = {'valid': False, 'error': 'unavailable'}
            with patch.object(legacy, 'Path', side_effect=local_path), \
                 patch.object(legacy, 'cgroup', return_value=root / 'group'), patch.object(legacy, 'cg_read', reader), \
                 patch.object(legacy, 'verify_io'), patch.object(legacy, 'occupancy', return_value={}), \
                 patch.object(legacy, 'file_occupancy', return_value={}), patch.object(legacy, 'filesystem_used', return_value=0), \
                 patch.object(legacy.subprocess, 'Popen', side_effect=popen), \
                 patch.object(legacy.threading, 'Thread', Thread), patch.object(legacy.threading, 'Timer', Thread), \
                 patch('bench_v1_cgroup_diagnostic.TraceSession', fake_trace):
                self.assertEqual(legacy.measured_command(base), 0)
                self.assertEqual(reader.call_count, 2)
                fake_trace.assert_not_called()
                row = json.loads((root / 'results/off/metrics.json').read_text())
                self.assertTrue(row['validated'])
                self.assertNotIn('cgroup_trace_receipt', row)
                enabled = dict(base, id='on', cgroup_trace_enabled=True)
                self.assertEqual(legacy.measured_command(enabled), 0)
                row = json.loads((root / 'results/on/metrics.json').read_text())
                self.assertTrue(row['validated'])
                self.assertFalse(row['cgroup_trace_receipt']['valid'])
                fake_trace.return_value.stop.assert_called_once()

    def test_scan_only_and_default_off(self):
        base = ['--baseline', '/fake', '--candidate', '/fake', '--output', '/unused']
        self.assertEqual(arguments(base).cgroup_trace, 'off')
        self.assertEqual(arguments(base + ['--operation', 'scan', '--cgroup-trace', 'contrast']).cgroup_trace, 'contrast')
        with patch('sys.stderr'):
            with self.assertRaises(SystemExit):
                arguments(base + ['--operation', 'refresh', '--cgroup-trace', 'contrast'])

    def test_nonidentical_binaries_fail_before_docker_or_fixture(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as tmp:
            args = arguments(['--baseline', '/b', '--candidate', '/c', '--output', tmp + '/out',
                              '--cgroup-trace', 'contrast'])
            sources = [{'sha256': 'a'*64}, {'sha256': 'b'*64}]
            with patch.dict('os.environ', {'GITHUB_ACTIONS': 'true', 'RUNNER_ENVIRONMENT': 'github-hosted'}), \
                 patch('bench_v1_pair.legacy', return_value=SimpleNamespace()), \
                 patch('bench_v1_pair.provenance', side_effect=sources), \
                 patch('bench_v1_pair.prepare') as prepare, patch('bench_v1_pair.subprocess.run') as run:
                with self.assertRaisesRegex(ValueError, 'identical actual binary'):
                    host(args)
                prepare.assert_not_called()
                run.assert_not_called()

    def test_worker_flag_only_in_contrast_and_original_spec_unchanged(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'spec.json'
            measured = Mock(return_value=0)
            helper = SimpleNamespace(measured_command=measured)
            for arm, expected in [('baseline-v1', False), ('candidate-v1', True)]:
                spec = {'arm': arm, 'cgroup_trace': 'contrast'}
                path.write_text(json.dumps(spec))
                with patch('bench_v1_runtime.legacy', return_value=helper):
                    self.assertEqual(inside('worker', path), 0)
                self.assertIs(measured.call_args.args[0]['cgroup_trace_enabled'], expected)
                self.assertEqual(json.loads(path.read_text()), spec)
            path.write_text('{}')
            with patch('bench_v1_runtime.legacy', return_value=helper):
                self.assertEqual(inside('worker', path), 0)
            self.assertNotIn('cgroup_trace_enabled', measured.call_args.args[0])

    def test_observer_never_constructs_trace_in_contrast_mode(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'spec.json'
            path.write_text(json.dumps(dict(id='x', seed_sha256='a'*64, seed_timestamp=1,
                                            timeout_seconds=10, expected_entries=3,
                                            scenario='unchanged', cgroup_trace='contrast')))
            saved = []
            helper = SimpleNamespace(cgroup=lambda: None, cg_read=lambda _: {},
                                     snapshot_integrity=lambda _: 'same',
                                     dump=lambda _, value: saved.append(value))
            with patch('bench_v1_runtime.legacy', return_value=helper), \
                 patch('bench_v1_runtime.observe_database', return_value=(3, 'digest')), \
                 patch('bench_v1_cgroup_diagnostic.TraceSession') as trace:
                self.assertEqual(inside('observer', path), 0)
                trace.assert_not_called()
            self.assertTrue(saved[-1]['validated'])

    def test_host_freezes_contrast_and_alternates_worker_order(self):
        from types import SimpleNamespace
        import contextlib
        import io
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / 'out'
            prep = root / 'prep.json'
            prep.write_text('{}')
            args = arguments(['--baseline', '/fake', '--candidate', '/fake', '--output', str(output),
                              '--cgroup-trace', 'contrast', '--pairs', '2'])
            source = dict(sha256='a'*64, source_revision='revision', build=dict(
                compiler='same', target='same', flags='', profile='release', release_profile={}, cargo_config={}))
            def flush(directory, name):
                (directory / (name + '.cache-flush.json')).write_text('{}')
            helper = SimpleNamespace(dump=lambda path, value: path.write_text(json.dumps(value)),
                                     capture=lambda argv: '' if argv[:2] == ['docker', 'ps'] else '{}',
                                     occupancy=lambda _: {'total': {'allocated_bytes': 1}},
                                     filesystem_used=lambda _: 0, flush_caches=flush)
            calls = []
            def container(image, binary, seed, tree, out, spec_path, spec, mode, label):
                calls.append((spec['id'], mode, dict(spec)))
                if spec['id'].startswith('seed-'):
                    (out / spec['id'] / 'metrics.json').write_text('{"validated": true}')
            env = dict(GITHUB_ACTIONS='true', RUNNER_ENVIRONMENT='github-hosted',
                       FREESPACE_BENCHMARK_RUN_TOKEN='test', RUNNER_TEMP=tmp, BENCH_PREP_RECEIPT=str(prep))
            fixture_data = {'expected_entries': 103, 'expected_subtree_entries': 100}
            with patch.dict('os.environ', env), patch('bench_v1_pair.legacy', return_value=helper), \
                 patch('bench_v1_pair.provenance', return_value=source), patch('bench_v1_pair.subprocess.run'), \
                 patch('bench_v1_pair.check_capacity', return_value={'success': True}), \
                 patch('bench_v1_pair.runner_preparation_context', return_value={}), \
                 patch('bench_v1_pair.prepare', return_value=fixture_data), \
                 patch('bench_v1_pair.container', side_effect=container), \
                 patch('bench_v1_pair.seed_hash', side_effect=lambda _, **kw: ('c'*64, {'snapshot.prev.db': 'b'*64}) if kw else 'c'*64), \
                 patch('bench_v1_pair.seed_timestamp', return_value=1), patch('bench_v1_pair.time.sleep'), \
                 patch('bench_v1_pair.summarize', return_value={'valid': True}), \
                 patch('bench_v1_pair.markdown', return_value='ok'), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(host(args), 0)
            seeds = [s for ident, _, s in calls if ident.startswith('seed-')]
            self.assertTrue(all('cgroup_trace' not in s for s in seeds))
            workers = [ident for ident, mode, _ in calls if mode == 'worker' and ident.startswith('p')]
            self.assertEqual(workers, ['p1-baseline-v1', 'p1-candidate-v1', 'p2-candidate-v1', 'p2-baseline-v1'])
            manifest = json.loads((output / 'manifest.json').read_text())
            self.assertEqual(manifest['cgroup_trace_policy'], {'mode': 'contrast', 'baseline-v1': False, 'candidate-v1': True})
            for ident in workers:
                persisted = json.loads((output / ident / 'spec.json').read_text())
                self.assertEqual(persisted['cgroup_trace'], 'contrast')
                self.assertNotIn('cgroup_trace_enabled', persisted)

    def test_session_failure_is_diagnostic_and_closes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'snapshot').mkdir()
            with patch('bench_v1_cgroup_diagnostic.CgroupTrace') as fake:
                fake.return_value.start.return_value = False
                fake.return_value.result.return_value = {'valid': False}
                session = TraceSession(root, root, root / 'snapshot')
                session.tick()
                session.stop()
                result = session.receipt(1.0, 2.0)
                self.assertFalse(result['valid'])
                fake.return_value.tick.assert_not_called()
                fake.return_value.stop.assert_called_once()

    def make_case(self, root):
        from bench_v1_cgroup_trace import CgroupTrace
        fixture(root)
        manifest = json.loads((root / 'manifest.json').read_text())
        manifest.update(operation='scan', cgroup_trace_policy={'mode': 'contrast', 'baseline-v1': False, 'candidate-v1': True})
        put(root / 'manifest.json', manifest)
        setup = root.with_name(root.name + '-setup')
        for ordinal in (1, 2):
            put(setup / f'seed-{ordinal}.json', {'operation': 'scan'})
        for name in ('one', 'two'):
            directory = root / name
            spec = json.loads((directory / 'spec.json').read_text())
            metric = json.loads((directory / 'metrics.json').read_text())
            spec.update(operation='scan', cgroup_trace='contrast')
            metric.update(operation='scan', cgroup_trace='contrast', cgroup_trace_enabled=name == 'two')
            metric['cgroup_after']['memory.peak'] = '1234' if name == 'one' else '1400'
            metric['report']['errors'] = 0
            if name == 'two':
                clock = Clock()
                data = values()
                collector = CgroupTrace('/fake', directory, read=lambda _, key: data[key], clock=clock,
                                        cpu_clock=lambda: 0)
                self.assertTrue(collector.start())
                collector.tick()
                clock.now = 10
                receipt = collector.stop()
                trace = directory / 'cgroup-trace.jsonl'
                receipt.update(scope='worker cgroup includes CLI and diagnostic', cli_start_ns=1, cli_end_ns=9, devices={'output_st_dev': 1, 'snapshot_st_dev': 1},
                               error=None, sha256=hashlib.sha256(trace.read_bytes()).hexdigest(),
                               setup_wall_ns=0, setup_thread_cpu_ns=0,
                               finalize_wall_ns=0, finalize_thread_cpu_ns=0)
                metric['cgroup_trace_receipt'] = receipt
            put(directory / 'spec.json', spec)
            put(directory / 'metrics.json', metric)
        result = summarize(root)
        self.assertTrue(result['valid'], result['issues'])
        put(root / 'summary.json', result)

    def test_pending_host_summary_requires_cleanup_cli_rewrite(self):
        from bench_v1_summary import main as summary_main
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'measurements'
            self.make_case(root)
            (root / 'runner-cleanup.json').unlink()
            pending = summarize(root, require_runner_cleanup=False)
            self.assertTrue(pending['valid'], pending['issues'])
            pending['runner_cleanup_pending'] = True
            put(root / 'summary.json', pending)
            self.assertFalse(analyze(root)['valid'])
            put(root / 'runner-cleanup.json', {'success': True, 'exit_code': 0})
            self.assertFalse(analyze(root)['valid'])
            self.assertEqual(summary_main([str(root)]), 0)
            self.assertNotIn('runner_cleanup_pending', json.loads((root / 'summary.json').read_text()))
            result = analyze(root)
            self.assertTrue(result['valid'], result['issues'])

    def test_valid_contrast_and_tamper_detection(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'measurements'
            self.make_case(root)
            result = analyze(root)
            self.assertTrue(result['valid'], result['issues'])
            self.assertEqual(result['cgroup_peak_bytes'][0]['delta'], 166)
            metric_path = root / 'two' / 'metrics.json'
            metric = json.loads(metric_path.read_text())
            metric['cgroup_trace_receipt']['cli_start_ns'] = True
            put(metric_path, metric)
            self.assertFalse(analyze(root)['valid'])
            metric['cgroup_trace_receipt']['cli_start_ns'] = 1
            metric['cgroup_trace_receipt']['cli_end_ns'] = -1
            put(metric_path, metric)
            self.assertFalse(analyze(root)['valid'])
            metric['cgroup_trace_receipt']['cli_end_ns'] = 9
            put(metric_path, metric)
            path = root / 'two' / 'cgroup-trace.jsonl'
            before = path.read_bytes()
            path.write_bytes(before.replace(b'max 7', b'max 8'))
            self.assertFalse(analyze(root)['valid'])
            path.write_bytes(before)
            lines = [json.loads(line) for line in before.splitlines()]
            lines[-2]['start_ns'] = 8
            lines[-2]['actual_ns'] = 8
            altered = b''.join((json.dumps(line, separators=(',', ':')) + '\n').encode() for line in lines)
            path.write_bytes(altered)
            metric['cgroup_trace_receipt']['sha256'] = hashlib.sha256(altered).hexdigest()
            metric['cgroup_trace_receipt']['bytes'] = len(altered)
            put(metric_path, metric)
            self.assertFalse(analyze(root)['valid'])
            lines = [json.loads(line) for line in before.splitlines()]
            lines[-1]['samples'] += 1
            tampered = b''.join((json.dumps(line, separators=(',', ':')) + '\n').encode() for line in lines)
            path.write_bytes(tampered)
            metric = json.loads(metric_path.read_text())
            metric['cgroup_trace_receipt']['sha256'] = hashlib.sha256(tampered).hexdigest()
            metric['cgroup_trace_receipt']['bytes'] = len(tampered)
            put(metric_path, metric)
            self.assertFalse(analyze(root)['valid'])
            path.write_bytes(before)
            metric['cgroup_trace_receipt']['sha256'] = hashlib.sha256(before).hexdigest()
            metric['cgroup_trace_receipt']['bytes'] = len(before)
            put(metric_path, metric)
            raw_summary = json.loads((root / 'summary.json').read_text())
            raw_summary['pairs'][0]['elapsed_seconds']['delta'] += 1
            put(root / 'summary.json', raw_summary)
            self.assertFalse(analyze(root)['valid'])

    def test_invalid_trace_and_summary_do_not_overwrite(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'measurements'
            self.make_case(root)
            target = root / 'diagnostic-summary.json'
            target.write_bytes(b'foreign')
            with self.assertRaises(FileExistsError):
                main([str(root)])
            self.assertEqual(target.read_bytes(), b'foreign')
            target.unlink()
            (root / 'two' / 'cgroup-trace.jsonl').unlink()
            self.assertEqual(main([str(root)]), 1)
            self.assertFalse(json.loads(target.read_text())['valid'])
            self.assertTrue(summarize(root)['valid'])


if __name__ == '__main__':
    unittest.main()
