"""Synthetic protocol bundles and <=3-row SQLite fixtures; no app/Docker/proc work."""
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parents[1]))
import cause_observe as observe
import cause_summary as summary
import cause_validate as validate
import cause_forecast


def write(root, name, value):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))


def disk_fixture(root):
    snapshot = root / 'snapshot'
    snapshot.mkdir()
    access = observe.FixtureAccess(root)
    def db(name):
        path = snapshot / name
        with sqlite3.connect(path) as conn:
            conn.execute('CREATE TABLE entries(id INTEGER PRIMARY KEY,name TEXT)')
            conn.executemany('INSERT INTO entries VALUES(?,?)', [(n, str(n)) for n in range(3)])
        return path
    previous = db('snapshot.prev.db')
    current = db('snapshot.db')
    with observe.PinSet(snapshot, inspection_aliases=True, fixture=access) as pins:
        pins.pin_old_previous()
        stage = db('snapshot.stage.tmp')
        pins.pin_stage()
        stage.unlink()
        os.replace(current, previous)
        db('snapshot.db')
        held = observe.record_inventory(snapshot, pins)
        paths = pins.inspection_paths()
        databases = {k: observe.db_breakdown(p, fixture=access) for k, p in dict(
            paths, published_current=snapshot / 'snapshot.db', published_previous=previous).items()}
        dropped = pins.drop_inspection_aliases()
        unlinked = observe.record_inventory(snapshot, pins)
        releases = {k: observe.release_measurement(pins, k, seconds=0) for k in ('stage', 'previous')}
        final = observe.record_inventory(snapshot, pins)
    return dict(held=held, databases=databases, aliases_dropped=dropped, unlinked=unlinked, releases=releases, final=final)


def global_receipt(stamp):
    return dict(required_fields_available=True, scope='GLOBAL host; never per-cgroup attribution',
                before=dict(monotonic_ns=stamp), after=dict(monotonic_ns=stamp+1), sources=dict(
                    slabinfo=dict(available=True, raw='synthetic', parsed=dict(required_fields_available=True,
                        parse_errors=[], objects=[dict(name='dentry', active_objects=1, total_objects=2, total_slab_pages=1)])),
                    meminfo=dict(available=True, raw='synthetic', parsed=dict(parse_errors=[],
                        fields=dict(MemAvailable=dict(value=6*2**20,unit='kB'))))),
                background=dict(required_fields_available=True,sources={name:dict(available=True,raw='synthetic',
                    parsed=dict(some=dict(avg10=0.0))) for name in ('cpu','io','memory','loadavg')}))


def bundle(root, disk):
    cases = [dict(id='smoke-' + m, mode=m, sampler='full', pin=False, group='smoke', spec_path='smoke-' + m + '.json')
             for m in ('idle', 'names', 'stat', 'scan')]
    cases += [dict(id='smoke-reserve', mode='scan', sampler='reserve', pin=False, group='collector', spec_path='smoke-reserve.json'),
              dict(id='disk-pin', mode='scan', sampler='full', pin=True, group='disk', spec_path='disk-pin.json')]
    source_root = Path(__file__).parents[2]
    paths = [*source_root.glob('scripts/cause_*.py'), source_root / 'scripts/cause_runner.sh',
             source_root / '.github/workflows/v2-balance.yml']
    hashes = {}
    for path in paths:
        name = str(path.relative_to(source_root))
        dest = root / 'source' / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, dest)
        hashes[name] = hashlib.sha256(dest.read_bytes()).hexdigest()
    manifest = dict(protocol=validate.PROTOCOL, stage='smoke', qualification=False,
                    baseline=dict(sha256=validate.SHA, source_revision=validate.REVISION,
                        build=dict(sha256=validate.SHA, target='x86_64-unknown-linux-musl', flags=dict(SOURCE_DATE_EPOCH='1793464183'))),
                    fixture_receipt=dict(files=100000, directories=449, expected_entries=100449, expected_subtree_entries=100),
                    limits=dict(memory_mib=4096, cpus=2, workers=1, swap_mib=0, io_limits=validate.IO, io_device_number='7:7'),
                    setup=dict(seed_digest='a'*64, seed_timestamp=100, fixture_seconds=1, seed_setup_seconds=3), cases=cases, order=[r['id'] for r in cases],
                    source=dict(revision='a'*40, sha256=hashes), github=dict(GITHUB_SHA='a'*40),
                    runner_preparation=dict(success=True, floor_bytes=42*2**30, target_bytes=44*2**30, duration_seconds=1),
                    runner_evidence={'disk-calibration.json': dict(success=True, profile='local', device='7:7',
                        requested_limits=validate.IO, jobs=[dict(mode=m, observed=1, limit=10) for m in ('write','read','randwrite','randread')]),
                        'filesystem.json': dict(filesystems=[dict(source='/dev/loop7', fstype='ext4')]),
                        'loop-devices.json': dict(loopdevices=[dict(name='/dev/loop7', dio=True, ro=False)])})
    manifest['limits']['io_device'] = '/dev/loop7'
    write(root, 'manifest.json', manifest)
    write(root, 'status.json', dict(success=True, completed=True, cleanup_success=True, failures=[], elapsed_seconds=10))
    write(root, 'runner-cleanup.json', dict(success=True, exit_code=0))
    stats = 'anon 1000\nfile 2000\nkernel 3000\nslab_reclaimable 2500\nslab_unreclaimable 100\npgscan 0\npgsteal 0'
    values = {'memory.max': str(validate.RAM), 'memory.peak': '50000', 'memory.current': '6000',
              'memory.swap.max': '0', 'memory.swap.current': '0', 'memory.swap.peak': '0', 'cpu.max': '200000 100000',
              'memory.events': 'max 0\noom 0\noom_kill 0', 'memory.events.local': 'max 0\noom 0\noom_kill 0',
              'memory.stat': stats, 'memory.pressure': '', 'io.pressure': '',
              'io.max': '7:7 rbps=33554432 wbps=33554432 riops=2200 wiops=3600', 'io.stat': '', 'cpu.stat': ''}
    common = ['--backend', 'v1', 'scan', '/scratch/tree', '--workers', '1', '--json', '--low-memory']
    for row in cases:
        ident, mode = row['id'], row['mode']
        identity = dict(protocol=validate.PROTOCOL, id=ident, nonce='b'*32)
        spec = dict(identity, mode=mode, sampler=row['sampler'], expected_entries=0 if mode == 'idle' else 100449,
                    memory_mib=4096, cpus=2, workers=1, io_limits=validate.IO, io_device_number='7:7', io_device='/dev/loop7',
                    seed_sha256='a'*64, seed_timestamp=100, backend='v1', operation='scan', scenario='unchanged',
                    keep_previous=True, timeout_seconds=1200)
        write(root, ident + '.json', spec)
        report = dict(entries=spec['expected_entries'], errors=0, incomplete=False, workers=1)
        if mode != 'scan':
            report.update(protocol=validate.PROTOCOL, mode=mode, validated=True, traversal=mode != 'idle',
                regular_files=0 if mode == 'idle' else 100000, directories=0 if mode == 'idle' else 449,
                directory_opens=0 if mode == 'idle' else 449, root_lstat_calls=0 if mode == 'idle' else 1,
                entry_stat_calls=100448 if mode == 'stat' else 0, path_kind_digest=None if mode == 'idle' else 'c'*64)
        timing = dict(elapsed_seconds=1, peak_rss_kib=100, exit_code=0)
        sampling = dict(coverage_valid=True, ticks=50, errors=[], capture_errors=0, interval_seconds=.02,
            reserve_max_gap_ns=250000000, max_tick_gap_ns=20000001, coverage_start_ns=3, coverage_end_ns=1000000002,
            availability='sampled-lower-bound' if row['sampler'] == 'full' else 'unavailable-reserve-only')
        for key in ('sampled_disk_peak', 'sampled_file_peaks', 'sampled_memory_peak', 'filesystem_used_peak'):
            sampling[key] = {} if row['sampler'] == 'full' else None
        metrics = dict(identity, mode=mode, sampler=row['sampler'], validated=True, cli_validated=True, errors=[], capture_errors=0,
            qualification=False, exit_code=0, timed_out=False, cli_start_ns=100, cli_end_ns=1000000100,
            elapsed_seconds=1, cgroup_before=dict(start_ns=1, end_ns=2, values=values),
            cgroup_after=dict(start_ns=1000000101, end_ns=1000000102, values=values), kernel_memory_peak_bytes=50000,
            kernel_peak_scope='frozen worker endpoint', stdout=report, gnu_time=timing, sampling=sampling,
            command=(['/binary'] if mode == 'scan' else ['/usr/bin/python3', '/harness/cause_probe.py']) + common)
        metrics['sampling']['coverage_end_ns'] = 1000000101
        write(root, ident + '/metrics.json', metrics)
        write(root, ident + '/stdout.json', report)
        write(root, ident + '/time.log', timing)
        write(root, ident + '/worker-ready.json', dict(identity, status='ready', cli_validated=True,
              kernel_peak_frozen_in='metrics.json', ready_ns=1000000103))
        write(root, ident + '/worker-terminal.json', dict(identity, validated=True, success=True, acknowledged=True,
              ack_status='matched', errors=[], terminal_ns=1000000110))
        write(root, ident + '/host-ack.json', identity)
        write(root, ident+'/host-timing.json', dict(total_seconds=1, copy_and_seed_hash_seconds=.1,
              cache_flush_seconds=.1, worker_and_capture_seconds=.5, observer_seconds=.1 if mode == 'scan' else 0,
              case_cleanup_seconds=.1))
        write(root, ident + '/cache-flush.json', dict(success=True, runner_environment='github-hosted',
              sync_command='os.sync()', flush_input='3\n', flush_command=['sudo', '-n', 'tee', '/proc/sys/vm/drop_caches'],
              started_unix_seconds=0, finished_unix_seconds=1))
        if mode != 'scan':
            write(root, ident+'/seed-integrity.json', dict(unchanged=True, expected_seed_sha256='a'*64, observed_seed_sha256='a'*64))
        config = dict(Memory=validate.RAM, MemorySwap=validate.RAM, NanoCpus=2000000000, PidsLimit=128,
                      NetworkMode='none', CgroupnsMode='private')
        for flag, key in (('ReadBps', 'rbps'), ('WriteBps', 'wbps'), ('ReadIOps', 'riops'), ('WriteIOps', 'wiops')):
            config['BlkioDevice' + flag] = [dict(Path='/dev/loop7', Rate=validate.IO[key])]
        host = dict(identity, validated=True, errors=[], pin=row['pin'], container_cleanup=True,
                    global_before=ident+'/global-before/global.json', global_after=ident+'/global-after/global.json',
                    docker_inspect_before_ack=dict(State=dict(Running=True, Pid=123), HostConfig=config,
                        Mounts=[dict(Destination=p, RW=rw) for p, rw in [('/scratch/tree', False), ('/binary', False),
                            ('/harness', False), ('/scratch', True), ('/results', True)]]),
                    docker_state=dict(Running=False, ExitCode=0, OOMKilled=False),
                    cgroup_after=dict(process_identity_stable=True, pid=123, sources={'memory.peak': dict(available=True)}),
                    disk_path=ident+'/disk.json' if row['pin'] else None)
        write(root, ident+'/host.json', host)
        for phase,count in (('before',0),('after',1)):
            write(root,f'{ident}/foreign-containers-{phase}.json',dict(exclusive=True,foreign=[],running_count=count))
        write(root, host['global_before'], global_receipt(4))
        write(root, host['global_after'], global_receipt(1000000104))
        if mode == 'scan':
            write(root, ident+'/observer.json', dict(validated=True, errors={}, logical_content_unchanged=True,
                  seed_sha256='a'*64, current_entries=100449, previous_entries=100449, semantic_sha256='d'*64))
        if row['pin']:
            write(root, host['disk_path'], disk)
            host['lifetime_path'] = ident + '/lifetime.json'
            host['cgroup_after']['cgroup_path'] = '/mock-cgroup'
            write(root, ident+'/host.json', host)
            inventory=dict(available=True,before=dict(monotonic_ns=1),after=dict(monotonic_ns=2),
                           entries={},allocated_bytes=0)
            write(root,host['lifetime_path'],dict(available=True,host_pid=123,cgroup_path='/mock-cgroup',
                counts=dict(polls=1,complete_polls=1,errors=0,races=0,limits=0,transition_records_dropped=0),
                maxima=dict(named_snapshot=inventory,combined=inventory,open_deleted=None),
                transitions=[dict(path='named:snapshot.stage.tmp',previous=None,
                    current={k:disk['held']['pins']['stage']['observation'][k] for k in ('device','inode')}),
                    dict(path='named:snapshot.stage.tmp',current=dict(state='absent'),
                    previous={k:disk['held']['pins']['stage']['observation'][k] for k in ('device','inode')})]))


class SummaryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='cause-observe-fixture-')
        self.root = Path(self.tmp.name)
        self.disk = disk_fixture(self.root)
        self.results = self.root / 'results'
        bundle(self.results, self.disk)

    def tearDown(self):
        self.tmp.cleanup()

    def modify(self, relative, change):
        data = json.loads((self.results / relative).read_text())
        change(data)
        write(self.results, relative, data)

    def test_complete_diagnostic_requires_cleanup_and_never_qualifies(self):
        pending = summary.summarize(self.results)
        self.assertEqual(pending['status'], 'pending-runner-cleanup')
        result = summary.summarize(self.results, True)
        self.assertEqual(result['issues'], [])
        self.assertTrue(result['valid'])
        self.assertFalse(result['qualification'])
        self.assertEqual(len(result['rows']), 6)

    def test_wrong_provenance_source_bytes_and_missing_cleanup_are_rejected(self):
        for relative, change in (
            ('manifest.json', lambda d: d['baseline'].update(sha256='0'*64)),
            ('manifest.json', lambda d: d['order'].reverse()),
            ('runner-cleanup.json', lambda d: d.update(success=False, exit_code=1))):
            original = (self.results / relative).read_text()
            self.modify(relative, change)
            self.assertFalse(summary.summarize(self.results, True)['valid'])
            (self.results / relative).write_text(original)
        (self.results / 'source/scripts/cause_probe.py').write_text('altered')
        self.assertFalse(summary.summarize(self.results, True)['valid'])

    def test_counterfactuals_reject_changed_probe_digest_semantics_and_nonce(self):
        for relative, change in (
            ('smoke-stat/metrics.json', lambda d: d['stdout'].update(path_kind_digest='e'*64)),
            ('smoke-scan/observer.json', lambda d: d.update(semantic_sha256='e'*64)),
            ('smoke-scan/host-ack.json', lambda d: d.update(nonce='z'*32)),
            ('smoke-idle/metrics.json', lambda d: d['stdout'].update(root_lstat_calls=1))):
            original = (self.results / relative).read_text()
            self.modify(relative, change)
            self.assertFalse(summary.summarize(self.results, True)['valid'], relative)
            (self.results / relative).write_text(original)

    def test_no_reserve_peaks_no_reclaim_no_missing_coverage(self):
        for relative, change in (
            ('smoke-reserve/metrics.json', lambda d: d['sampling'].update(sampled_memory_peak=0)),
            ('smoke-scan/metrics.json', lambda d: d['sampling'].update(max_tick_gap_ns=250000001)),
            ('smoke-scan/metrics.json', lambda d: d['cgroup_after']['values'].update(**{'memory.stat':
                d['cgroup_after']['values']['memory.stat'].replace('pgscan 0', 'pgscan 1')})),
            ('smoke-scan/metrics.json', lambda d: d.update(kernel_memory_peak_bytes=123))):
            original = (self.results / relative).read_text()
            self.modify(relative, change)
            self.assertFalse(summary.summarize(self.results, True)['valid'], relative)
            (self.results / relative).write_text(original)

    def test_disk_dedup_release_identity_and_page_accounting(self):
        validate.disk_ok(self.disk)
        for change in (lambda d: d['held'].update(allocated_bytes=0),
                       lambda d: d['unlinked']['pins']['stage'].update(naturally_unlinked=False),
                       lambda d: d['releases']['stage']['comparisons'][0].update(residual_bytes=999),
                       lambda d: d['databases']['stage']['pragmas'].update(page_count=999)):
            altered = copy.deepcopy(self.disk)
            change(altered)
            with self.assertRaises(ValueError):
                validate.disk_ok(altered)

    def test_screen_forecast_uses_every_fixed_case_and_both_caps(self):
        result = cause_forecast.forecast(self.results, 200)
        self.assertTrue(result['admitted'])
        self.assertAlmostEqual(result['host_forecast_seconds'], 270)
        self.assertAlmostEqual(result['whole_step_forecast_seconds'], 460)
        self.modify('smoke-stat/host-timing.json', lambda d: d.update(total_seconds=100))
        self.modify('status.json', lambda d: d.update(elapsed_seconds=109))
        result = cause_forecast.forecast(self.results, 300)
        self.assertFalse(result['admitted'])

    def test_forecast_rejects_incomplete_or_impossible_timing(self):
        self.modify('smoke-stat/host-timing.json', lambda d: d.update(total_seconds=.01))
        with self.assertRaises(ValueError):
            cause_forecast.forecast(self.results, 200)

    def test_initial_absence_is_not_a_stage_retirement(self):
        self.modify('disk-pin/lifetime.json', lambda d: d.update(transitions=[dict(
            path='named:snapshot.stage.tmp',previous=None,current=dict(state='absent'))]))
        result = summary.summarize(self.results, True)
        self.assertFalse(result['valid'])
        self.assertTrue(result['groups']['ram']['valid'])
        self.assertTrue(any('existing-to-absent' in s for s in result['issues']))

    def test_unknown_poll_does_not_erase_observed_lifetime_bounds(self):
        self.modify('disk-pin/lifetime.json', lambda d: d['transitions'].insert(1, dict(
            path='named:snapshot.stage.tmp',previous=d['transitions'][0]['current'],
            current=dict(state='unavailable'))))
        self.assertTrue(summary.summarize(self.results, True)['valid'])


if __name__ == '__main__':
    unittest.main()
