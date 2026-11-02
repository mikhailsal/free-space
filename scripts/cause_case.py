"""One hosted diagnostic case and bounded Docker/acknowledgement lifecycle."""
import json
import math
import os
import signal
from pathlib import Path
import subprocess
import time
import uuid

from cause_global import capture_global
from cause_observe import PinSet, capture_cgroup, db_breakdown, record_inventory, release_measurement
from cause_worker import PROTOCOL, publish
from cause_lifetime import LifetimeWatch


def read(path):
    path = Path(path)
    if path.is_symlink() or path.stat().st_size > 2 * 1024 * 1024:
        raise ValueError('unsafe or oversized receipt')
    return json.loads(path.read_text())


def identity(data, spec):
    if any(data.get(k) != spec[k] for k in ('protocol', 'id', 'nonce')):
        raise ValueError('receipt identity mismatch')


def command(image, binary, case, tree, output, spec, label, name):
    args = ['docker', 'run', '--name', name, '--label', label, '--hostname', 'freespace-benchmark',
            '--network', 'none', '--cpus', '2', '--memory', '4096m', '--memory-swap', '4096m',
            '--cgroupns', 'private', '--pids-limit', '128', '--user', f'{os.getuid()}:{os.getgid()}',
            '--entrypoint', 'python3']
    for env in ('GITHUB_ACTIONS=true', 'RUNNER_ENVIRONMENT=github-hosted', 'PYTHONDONTWRITEBYTECODE=1'):
        args += ['--env', env]
    for src, dst, ro in ((case, '/scratch', False), (tree, '/scratch/tree', True),
                          (output, '/results', False), (Path(__file__).resolve().parent, '/harness', True),
                          (binary, '/binary', True)):
        args += ['--mount', f'type=bind,src={src},dst={dst}' + (',readonly' if ro else '')]
    for flag, key in (('read-bps', 'rbps'), ('write-bps', 'wbps'), ('read-iops', 'riops'), ('write-iops', 'wiops')):
        args += ['--device-' + flag, f'{spec["io_device"]}:{spec["io_limits"][key]}']
    return args + [image, '/harness/cause_worker.py', '/results/' + spec['id'] + '.json']


def inspect(name):
    result = subprocess.run(['docker', 'inspect', name], capture_output=True, text=True, timeout=10, check=True)
    return json.loads(result.stdout)[0]


def foreign_containers(label):
    token = label.removeprefix('freespace.benchmark=')
    if not token or label != 'freespace.benchmark=' + token:
        raise ValueError('invalid ownership label')
    result = subprocess.run(['docker', 'ps', '--format', '{{.ID}}\t{{.Label "freespace.benchmark"}}'],
                            capture_output=True, text=True, timeout=10, check=True)
    if len(result.stdout) > 1024 * 1024:
        raise ValueError('container listing too large')
    rows = result.stdout.splitlines()
    if len(rows) > 128:
        raise ValueError('too many running containers to verify exclusivity')
    foreign = []
    for row in rows:
        parts = row.split('\t', 1)
        if len(parts) != 2 or not parts[0]:
            raise ValueError('invalid Docker listing')
        if parts[1] != token:
            foreign.append(dict(id=parts[0], benchmark_label=parts[1]))
    return dict(running_count=len(rows), foreign=foreign, exclusive=not foreign)


def require_quiet_host(capture):
    if not capture.get('required_fields_available') or not capture['background']['required_fields_available']:
        raise ValueError('global before sources unavailable')
    pressure = capture['background']['sources']['memory']['parsed']['some']['avg10']
    available = capture['sources']['meminfo']['parsed']['fields']['MemAvailable']
    if (type(pressure) not in (int, float) or not math.isfinite(pressure) or pressure < 0 or pressure > 1.0
            or available.get('unit') != 'kB' or type(available.get('value')) is not int
            or available['value'] * 1024 < 5 * 2**30):
        raise ValueError('host memory pressure or available memory outside quiet-run limits')


def disk_capture(pins, snapshot):
    held = record_inventory(snapshot, pins)
    if (not held['complete'] or held['pins']['stage']['path_state'] != 'absent'
            or not held['pins']['previous']['app_path_retired']):
        raise ValueError('pinned paths not retired or inventory incomplete')
    paths = pins.inspection_paths()
    databases = {name: db_breakdown(path) for name, path in {
        'stage': paths['stage'], 'previous': paths['previous'],
        'published_current': snapshot / 'snapshot.db', 'published_previous': snapshot / 'snapshot.prev.db'}.items()}
    if any(not x['available'] or x['errors'] for x in databases.values()):
        raise ValueError('database inspection failed')
    dropped = pins.drop_inspection_aliases()
    unlinked = record_inventory(snapshot, pins)
    if not dropped['available'] or not unlinked['complete'] or any(not unlinked['pins'][x]['naturally_unlinked'] for x in ('stage', 'previous')):
        raise ValueError('pinned inodes remain linked')
    releases = {name: release_measurement(pins, name) for name in ('stage', 'previous')}
    final = record_inventory(snapshot, pins)
    if not final['complete']:
        raise ValueError('final disk inventory incomplete')
    return dict(held=held, databases=databases, aliases_dropped=dropped, unlinked=unlinked,
                releases=releases, final=final)


def run_case(image, binary, case, tree, output, spec, label, helper, pin=False):
    directory = output / spec['id']
    name = 'fs-cause-' + uuid.uuid4().hex
    errors, state, cgroup = [], {}, {}
    cleanup = False
    proc = pins = watch = None
    ready = directory / 'worker-ready.json'
    host = dict(protocol=PROTOCOL, id=spec['id'], nonce=spec['nonce'], validated=False, errors=errors,
                global_before=f'{spec["id"]}/global-before/global.json',
                global_after=f'{spec["id"]}/global-after/global.json', cgroup_after=cgroup,
                docker_state=state, pin=pin, disk_path=f'{spec["id"]}/disk.json' if pin else None,
                container_cleanup=False, lifetime_path=f'{spec["id"]}/lifetime.json' if pin else None)
    try:
        if pin:
            pins = PinSet(case / 'snapshot', inspection_aliases=True)
            pins.pin_old_previous()
        foreign_before = foreign_containers(label)
        helper.dump(directory / 'foreign-containers-before.json', foreign_before)
        if not foreign_before['exclusive'] or foreign_before['running_count'] != 0:
            raise ValueError('foreign running container before case')
        before = capture_global(directory / 'global-before', include_background=True)
        require_quiet_host(before)
        with (directory / 'docker.log').open('x') as log:
            proc = subprocess.Popen(command(image, binary, case, tree, output, spec, label, name), stdout=log, stderr=subprocess.STDOUT)
            started = time.monotonic()
            pid = None
            while not ready.exists():
                if watch is not None:
                    watch.poll()
                if pin and 'stage' not in pins.pins and (case / 'snapshot' / 'snapshot.stage.tmp').exists():
                    try:
                        pins.pin_stage()
                    except FileNotFoundError:
                        pass
                if pid is None:
                    try:
                        current = inspect(name)
                        if current['State'].get('Pid', 0) > 0:
                            pid = current['State']['Pid']
                            host['docker_inspect_before_ack'] = current
                    except (subprocess.CalledProcessError, KeyError, IndexError):
                        pass
                    if pid is not None and pin and watch is None:
                        watch = LifetimeWatch(case / 'snapshot', pins, pid)
                        watch.poll()
                    if pid is None and time.monotonic() - started > 60:
                        raise TimeoutError('container start timeout')
                if proc.poll() is not None:
                    raise RuntimeError('worker exited before ready')
                if time.monotonic() - started > spec['timeout_seconds'] + 150:
                    raise TimeoutError('worker ready timeout')
                time.sleep(.05)
            identity(read(ready), spec)
            metrics = read(directory / 'metrics.json')
            identity(metrics, spec)
            if metrics.get('validated') is not True:
                raise ValueError('worker metrics invalid')
            live = inspect(name)
            if pid is None:
                pid = live['State'].get('Pid')
            if not pid or live['State'].get('Pid') != pid or not live['State'].get('Running'):
                raise ValueError('container identity changed before acknowledgement')
            host['docker_inspect_before_ack'] = live
            if pin:
                if watch is None:
                    watch = LifetimeWatch(case / 'snapshot', pins, pid)
                watch.poll()
            cgroup = capture_cgroup(pid)
            host['cgroup_after'] = cgroup
            if not cgroup['process_identity_stable'] or any(not v.get('available') for v in cgroup['sources'].values()):
                raise ValueError('host cgroup capture incomplete')
            after = capture_global(directory / 'global-after', include_background=True)
            if not after['required_fields_available']:
                raise ValueError('global after sources unavailable')
            foreign_after = foreign_containers(label)
            helper.dump(directory / 'foreign-containers-after.json', foreign_after)
            if not foreign_after['exclusive'] or foreign_after['running_count'] != 1:
                raise ValueError('foreign running container after case')
            if pin:
                helper.dump(directory / 'lifetime.json', watch.finish())
                if 'stage' not in pins.pins:
                    raise ValueError('stage was not observed while CLI ran')
                helper.dump(directory / 'disk.json', disk_capture(pins, case / 'snapshot'))
    except Exception as error:
        errors.append(repr(error))
    finally:
        # Defer the host deadline until container and descriptor cleanup has finished.
        prior_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGALRM})
        try:
            finish_case_cleanup(proc, pins, ready, spec, directory, name, helper, host, errors)
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, prior_mask)
    return host


def finish_case_cleanup(proc, pins, ready, spec, directory, name, helper, host, errors):
    if ready.exists():
        try:
            identity(read(ready), spec)
            publish(helper, directory / 'host-ack.json', {k: spec[k] for k in ('protocol', 'id', 'nonce')})
        except Exception as error:
            errors.append('ack: ' + repr(error))
    if proc is not None:
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            errors.append('worker exit timeout')
        try:
            state = inspect(name)['State']
            host['docker_state'] = state
            if state.get('Running') or state.get('ExitCode') != 0 or state.get('OOMKilled'):
                errors.append('invalid Docker terminal state')
            terminal = read(directory / 'worker-terminal.json')
            identity(terminal, spec)
            if terminal.get('validated') is not True or terminal.get('acknowledged') is not True:
                errors.append('invalid worker terminal')
        except Exception as error:
            errors.append('terminal: ' + repr(error))
        try:
            subprocess.run(['docker', 'rm', '-f', name], check=True, timeout=30, stdout=subprocess.DEVNULL)
            cleanup = True
        except Exception as error:
            errors.append('container cleanup: ' + repr(error))
        if proc.poll() is None:
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                errors.append('Docker CLI did not exit')
    else:
        cleanup = True
    if pins is not None:
        try:
            pins.close()
        except Exception as error:
            errors.append('pin cleanup: ' + repr(error))
    host['container_cleanup'] = cleanup
    host['validated'] = not errors and cleanup
    helper.dump(directory / 'host.json', host)
