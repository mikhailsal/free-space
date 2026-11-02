#!/usr/bin/env python3
"""Orchestrate the fixed resource-cause diagnostic on hosted Actions VMs only."""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import stat
import sys
import tempfile
import time
import uuid

from bench_v1_admission import check_capacity, runner_preparation_context
from bench_v1_fixture import prepare
from bench_v1_pair import provenance, seed_timestamp
from bench_v1_runtime import container, file_hash, legacy, seed_hash
from cause_case import read, run_case
from cause_worker import PROTOCOL

SHA = 'ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980'
REVISION = 'e8265778ef78ec4acd9da1375eedb0be37a123f5'


def build_schedule(stage):
    if stage == 'smoke':
        return [dict(id='smoke-' + m, mode=m, sampler='full', pin=False, group='smoke') for m in ('idle', 'names', 'stat', 'scan')] + [
            dict(id='smoke-reserve', mode='scan', sampler='reserve', pin=False, group='collector'),
            dict(id='disk-pin', mode='scan', sampler='full', pin=True, group='disk')]
    if stage != 'screen':
        raise ValueError('invalid stage')
    orders = (('idle', 'names', 'scan', 'stat'), ('names', 'stat', 'idle', 'scan'),
              ('stat', 'scan', 'names', 'idle'), ('scan', 'idle', 'stat', 'names'))
    result = []
    for n, order in enumerate(orders):
        for mode in order:
            result.append(dict(id=f'lat{n+1}-{mode}', mode=mode, sampler='full', pin=False, group='latin', round=n+1))
    for n in range(1, 4):
        for sampler in (('reserve', 'full') if n == 2 else ('full', 'reserve')):
            result.append(dict(id=f'col{n}-{sampler}', mode='scan', sampler=sampler, pin=False, group='collector', round=n))
    result.append(dict(id='disk-pin', mode='scan', sampler='full', pin=True, group='disk'))
    return result


def guard():
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        raise RuntimeError('dedicated GitHub-hosted Actions runner required')


def empty_output(path):
    path = Path(path).absolute()
    if path.is_symlink() or (path.exists() and (not path.is_dir() or any(path.iterdir()))):
        raise ValueError('output must be absent or an empty directory')
    path.mkdir(parents=True, exist_ok=True)
    return path.resolve()


def verify_binary(binary):
    source = provenance(binary)
    build = source['build']
    if (source['sha256'] != SHA or source['source_revision'] != REVISION
            or build.get('target') != 'x86_64-unknown-linux-musl'
            or str(build.get('flags', {}).get('SOURCE_DATE_EPOCH')) != '1793464183'):
        raise ValueError('baseline provenance does not match pinned artifact')
    return source


def capture_sources(output):
    root = Path(__file__).resolve().parent.parent
    paths = list((root / 'scripts').glob('cause*.py'))
    paths += [root / p for p in ('scripts/cause_runner.sh', 'scripts/bench_v1_admission.py',
              'scripts/bench_v1_fixture.py', 'scripts/bench_v1_pair.py', 'scripts/bench_v1_runtime.py',
              'scripts/bench-v2-container.py', 'scripts/bench_v2_config.py', 'scripts/bench_v1_summary.py',
              'scripts/bench_v1_runner_prepare.py', 'scripts/bench-disk-calibration.py',
              'scripts/bench-v2-container/Dockerfile',
              '.github/workflows/v2-balance.yml')]
    target = output / 'source'
    mapping = {}
    for path in sorted(set(paths)):
        relative = path.relative_to(root)
        dest = target / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, dest)
        mapping[str(relative)] = file_hash(dest)
    return dict(revision=os.environ.get('GITHUB_SHA'), sha256=mapping)



def verify_seed_integrity(snapshot, expected, destination, helper):
    observed = None
    error = None
    try:
        observed = seed_hash(snapshot)
    except Exception as exc:
        error = repr(exc)
    receipt = dict(expected_seed_sha256=expected, observed_seed_sha256=observed,
                   unchanged=error is None and observed == expected)
    if error is not None:
        receipt['error'] = error
    helper.dump(destination, receipt)
    if not receipt['unchanged']:
        raise ValueError('probe changed seed or seed hash failed')


def execute(args, cancel_deadline=lambda: None):
    guard()
    started = time.monotonic()
    budget = args.budget_seconds
    output = empty_output(args.output)
    helper = legacy()
    failures, completed, cleaned = [], False, True
    def status(success=False):
        helper.dump(output / 'status.json', dict(success=success, completed=completed, cleanup_success=cleaned,
                                                 failures=failures, runner_cleanup_pending=True,
                                                 elapsed_seconds=max(0, time.monotonic() - started), budget_seconds=budget))
    status()
    scratch = None
    preserve = False
    try:
        setup = output.with_name(output.name + '-setup')
        setup.mkdir(exist_ok=False)
        binary = args.binary.resolve(strict=True)
        baseline = verify_binary(binary)
        preparation = runner_preparation_context(read(os.environ['BENCH_PREP_RECEIPT']))
        scratch_root = Path(os.environ['RUNNER_TEMP']).resolve(strict=True)
        device = os.stat(args.io_device)
        if not stat.S_ISBLK(device.st_mode) or device.st_rdev != scratch_root.stat().st_dev:
            raise ValueError('I/O block device does not directly back scratch')
        number = f'{os.major(device.st_rdev)}:{os.minor(device.st_rdev)}'
        scratch = Path(tempfile.mkdtemp(prefix='resource-cause-', dir=scratch_root))
        files = {'smoke': 100000, 'screen': 1000000}[args.stage]
        estimated = files + 3 + ((files + 999) // 1000) * 7
        capacity = check_capacity(scratch, files, estimated, seed_bytes=estimated * 1024)
        helper.dump(output / 'capacity-admission.json', capacity)
        if not capacity['success']:
            raise ValueError('insufficient fixture capacity')
        fixture_started = time.monotonic()
        fixture = prepare(scratch / 'fixture' / 'tree', files)
        fixture_seconds = time.monotonic() - fixture_started
        tree = scratch / 'fixture' / 'tree'
        limits = dict(rbps=args.read_bps, wbps=args.write_bps, riops=args.read_iops, wiops=args.write_iops)
        if limits != dict(rbps=33554432, wbps=33554432, riops=2200, wiops=3600):
            raise ValueError('I/O limits do not match fixed profile')
        label = 'freespace.benchmark=' + os.environ['FREESPACE_BENCHMARK_RUN_TOKEN']
        image = f'resource-cause-calibration:{os.environ["GITHUB_RUN_ID"]}-{os.environ.get("GITHUB_RUN_ATTEMPT", "1")}-{os.environ["GITHUB_JOB"]}'
        common = dict(backend='v1', single_operation=True, files=files, expected_entries=fixture['expected_entries'],
                      expected_subtree_entries=fixture['expected_subtree_entries'], memory_mib=4096,
                      observer_memory_mib=4096, scenario='unchanged', workers=1, cpus=2, keep_previous=True,
                      scan_profile='disk', timeout_seconds=1200, io_limits=limits, io_device=args.io_device,
                      io_device_number=number, minimum_free_bytes=2**31, operation='scan', target='/scratch/tree')
        seed_started = time.monotonic()
        seed = scratch / 'seed'
        (seed / 'snapshot' / 'tmp').mkdir(parents=True)
        (seed / 'tree').mkdir()
        (seed / 'config.toml').write_text('keep_previous = true\n')
        for ordinal in (1, 2):
            remaining = budget - (time.monotonic() - started)
            if remaining < 181:
                raise TimeoutError('host deadline before seed')
            ident = f'seed-{ordinal}'
            spec = dict(common, id=ident, timeout_seconds=min(1200, int(remaining - 180)),
                        allocation_diagnostics=False,
                        initial_snapshot_allocated_bytes=helper.occupancy(seed / 'snapshot')['total']['allocated_bytes'])
            (setup / ident).mkdir()
            path = setup / (ident + '.json')
            helper.dump(path, spec)
            try:
                container(image, binary, seed, tree, setup, path, spec, 'worker', label)
            except Exception:
                preserve = True
                raise
            if read(setup / ident / 'metrics.json').get('validated') is not True:
                raise ValueError('seed scan validation failed')
            time.sleep(1.05)
        digest, seed_files = seed_hash(seed / 'snapshot', include_files=True)
        timestamp = seed_timestamp(seed / 'snapshot')
        time.sleep(max(0, timestamp + 1.01 - time.time()))
        seed_setup_seconds = time.monotonic() - seed_started
        schedule = build_schedule(args.stage)
        source = capture_sources(output)
        runner_evidence = {name: read(output.parent / name) for name in
                           ('disk-calibration.json', 'filesystem.json', 'loop-devices.json')}
        manifest = dict(protocol=PROTOCOL, stage=args.stage, baseline=baseline, source=source,
                        budget_seconds=budget,
                        runner_evidence=runner_evidence,
                        fixture_receipt=fixture, runner_preparation=preparation,
                        setup=dict(seed_digest=digest, seed_files=seed_files, seed_timestamp=timestamp,
                                   fixture_seconds=fixture_seconds, seed_setup_seconds=seed_setup_seconds),
                        limits=dict(memory_mib=4096, cpus=2, workers=1, swap_mib=0, io_limits=limits,
                                    io_device=args.io_device, io_device_number=number),
                        cases=[dict(row, spec_path=row['id'] + '.json') for row in schedule],
                        order=[row['id'] for row in schedule], qualification=False,
                        github={k: os.environ.get(k) for k in ('GITHUB_SHA', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT', 'GITHUB_JOB', 'GITHUB_REPOSITORY')})
        helper.dump(output / 'manifest.json', manifest)
        probe_digest = None
        for row in schedule:
            case_started = time.monotonic()
            remaining = budget - (time.monotonic() - started)
            if remaining < 181:
                raise TimeoutError('host deadline before case')
            ident = row['id']
            directory = output / ident
            directory.mkdir()
            case = scratch / 'case'
            copy_started = time.monotonic()
            shutil.copytree(seed, case)
            if seed_hash(case / 'snapshot') != digest:
                raise ValueError('seed copy differs')
            copy_seconds = time.monotonic() - copy_started
            initial = helper.occupancy(case / 'snapshot')['total']['allocated_bytes']
            available = os.statvfs(case)
            if available.f_bavail * available.f_frsize < 2 * initial + 2**31:
                raise ValueError('insufficient case reserve')
            remaining = budget - (time.monotonic() - started)
            if remaining < 181:
                raise TimeoutError('host deadline after case setup')
            spec = dict(common, protocol=PROTOCOL, id=ident, nonce=uuid.uuid4().hex, mode=row['mode'],
                        sampler=row['sampler'], expected_entries=0 if row['mode'] == 'idle' else fixture['expected_entries'],
                        expected_regular_files=0 if row['mode'] == 'idle' else files,
                        expected_directories=0 if row['mode'] == 'idle' else fixture['directories'],
                        timeout_seconds=min(1200, int(remaining - 180)), seed_sha256=digest,
                        seed_timestamp=timestamp, initial_snapshot_allocated_bytes=initial)
            path = output / (ident + '.json')
            helper.dump(path, spec)
            helper.dump(directory / 'setup.json', dict(initial_snapshot_allocated_bytes=initial, seed_sha256=digest))
            flush_started = time.monotonic()
            helper.flush_caches(directory, 'cache')
            (directory / 'cache.cache-flush.json').rename(directory / 'cache-flush.json')
            flush_seconds = time.monotonic() - flush_started
            if budget - (time.monotonic() - started) <= 180:
                raise TimeoutError('host deadline after cache flush')
            worker_started = time.monotonic()
            try:
                host = run_case(image, binary, case, tree, output, spec, label, helper, row['pin'])
            except BaseException:
                preserve = True
                raise
            worker_seconds = time.monotonic() - worker_started
            if not host['container_cleanup']:
                preserve = True
            if not host['validated']:
                raise ValueError('host case failed: ' + ident + ' ' + repr(host['errors']))
            observer_seconds = 0.0
            if row['mode'] != 'scan':
                verify_seed_integrity(case / 'snapshot', digest, directory / 'seed-integrity.json', helper)
                if row['mode'] in ('names', 'stat'):
                    found = read(directory / 'metrics.json')['stdout']['path_kind_digest']
                    if probe_digest is not None and found != probe_digest:
                        raise ValueError('probe path digests differ')
                    probe_digest = found
            else:
                if budget - (time.monotonic() - started) <= 180:
                    raise TimeoutError('host deadline before observer')
                observer_started = time.monotonic()
                try:
                    container(image, binary, case, tree, output, path, spec, 'observer', label)
                except Exception:
                    preserve = True
                    raise
                observer = read(directory / 'observer.json')
                if (observer.get('validated') is not True or observer.get('previous_entries') != fixture['expected_entries']
                        or observer.get('current_entries') != fixture['expected_entries'] or observer.get('logical_content_unchanged') is not True):
                    raise ValueError('observer validation failed')
                observer_seconds = time.monotonic() - observer_started
            cleanup_started = time.monotonic()
            shutil.rmtree(case)
            cleanup_seconds = time.monotonic() - cleanup_started
            helper.dump(directory / 'host-timing.json', dict(
                copy_and_seed_hash_seconds=copy_seconds, cache_flush_seconds=flush_seconds,
                worker_and_capture_seconds=worker_seconds, observer_seconds=observer_seconds,
                case_cleanup_seconds=cleanup_seconds, total_seconds=time.monotonic() - case_started))
        completed = True
    except Exception as error:
        failures.append(repr(error))
    finally:
        # The measurement deadline excludes mandatory best-effort cleanup.
        cancel_deadline()
        if time.monotonic() - started >= budget and not failures:
            failures.append('host deadline exceeded')
            completed = False
        if scratch is not None and not preserve:
            try:
                for name in ('case', 'seed'):
                    path = scratch / name
                    if path.exists():
                        shutil.rmtree(path)
            except Exception as error:
                cleaned = False
                failures.append('scratch cleanup: ' + repr(error))
        elif preserve:
            cleaned = False
            failures.append('scratch preserved due to container cleanup failure')
        status(completed and cleaned and not failures)
    return 0 if completed and cleaned and not failures else 1


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--stage', required=True, choices=('smoke', 'screen'))
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--io-device', required=True)
    parser.add_argument('--budget-seconds', type=int, default=2100)
    for flag in ('read-bps', 'write-bps', 'read-iops', 'write-iops'):
        parser.add_argument('--' + flag, required=True, type=int)
    args = parser.parse_args(argv)
    if not 181 <= args.budget_seconds <= 2100:
        parser.error('--budget-seconds must be between 181 and 2100')
    def deadline(_signum, _frame):
        raise TimeoutError('host deadline exceeded')
    previous = signal.signal(signal.SIGALRM, deadline)
    signal.setitimer(signal.ITIMER_REAL, args.budget_seconds)
    try:
        return execute(args, cancel_deadline=lambda: signal.setitimer(signal.ITIMER_REAL, 0))
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


if __name__ == '__main__':
    sys.exit(main())
