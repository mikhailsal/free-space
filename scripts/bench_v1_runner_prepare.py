#!/usr/bin/env python3
"""Bounded disk preparation on dedicated ephemeral GitHub-hosted runners."""
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import time
from bench_v1_admission import GIB, V1_HARD_FLOOR_GIB, V1_PREP_HEADROOM_GIB

FLOOR = V1_HARD_FLOOR_GIB * GIB
TARGET = (V1_HARD_FLOOR_GIB + V1_PREP_HEADROOM_GIB) * GIB
TOTAL_SECONDS = 240
COMMAND_SECONDS = 90
ALLOWLIST = (
    '/usr/local/lib/android', '/usr/share/dotnet', '/usr/local/.ghcup', '/opt/ghc',
    '/opt/hostedtoolcache/CodeQL', '/usr/share/swift', '/usr/lib/jvm', '/usr/local/go',
    '/usr/share/powershell',
)


def target_info(env):
    path = env.get('RUNNER_TEMP', '')
    if not path or not os.path.isabs(path) or os.path.normpath(path) != path or os.path.realpath(path) != path:
        raise ValueError('RUNNER_TEMP must be an existing absolute canonical directory')
    info = os.stat(path)
    if not stat.S_ISDIR(info.st_mode):
        raise ValueError('RUNNER_TEMP must be a directory')
    return path, info.st_dev


def disk(path):
    usage = shutil.disk_usage(path)
    return dict(total_bytes=usage.total, used_bytes=usage.used, available_bytes=usage.free)


def mountpoints():
    # Kernel mountinfo escapes whitespace and backslash as octal sequences.
    with open('/proc/self/mountinfo', encoding='utf-8') as stream:
        lines = stream.readlines()
    result = []
    for line in lines:
        fields = line.split()
        if len(fields) < 10 or '-' not in fields:
            raise ValueError('malformed mountinfo')
        result.append(re.sub(r'\\([0-7]{3})', lambda m: chr(int(m.group(1), 8)), fields[4]))
    if not result:
        raise ValueError('empty mountinfo')
    return result


def safe_directory(path, target_dev, mounts):
    """Reject symlinks, mountpoints/descendants, and other filesystems."""
    p = Path(path)
    if not p.is_absolute() or str(p) != path or os.path.realpath(path) != path:
        return False
    if any(m == path or m.startswith(path + '/') for m in mounts):
        return False
    for part in reversed((p, *p.parents)):
        if part == Path('/'):
            continue
        try:
            info = os.lstat(part)
        except FileNotFoundError:
            return False
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            return False
    return info.st_dev == target_dev


def prepare(env, *, get_target=target_info, read_disk=disk, read_mounts=mountpoints,
            safe=safe_directory, run=subprocess.run, now=time.monotonic, update=lambda _receipt: None):
    receipt = dict(success=False, floor_bytes=FLOOR, target_bytes=TARGET, target_path=env.get('RUNNER_TEMP'),
                   target_dev=None, before=None, after=None, actions=[], errors=[])
    started = now()
    try:
        if env.get('GITHUB_ACTIONS') != 'true' or env.get('RUNNER_ENVIRONMENT') != 'github-hosted':
            raise ValueError('requires ephemeral GitHub-hosted runner')
        path, dev = get_target(env)
        receipt.update(target_path=path, target_dev=dev)
        receipt['before'] = read_disk(path)
        receipt['after'] = receipt['before']
        update(receipt)
        # No deletion if the preparation target has already been met.
        mounts = read_mounts() if receipt['after']['available_bytes'] < TARGET else []
        for sdk in ALLOWLIST:
            if receipt['after']['available_bytes'] >= TARGET:
                break
            if TOTAL_SECONDS - (now() - started) <= 5:
                raise TimeoutError('preparation deadline exceeded')
            if not safe(sdk, dev, mounts):
                receipt['actions'].append(dict(path=sdk, status='skipped-unsafe-or-absent'))
                update(receipt)
                continue
            remaining = TOTAL_SECONDS - (now() - started)
            limit = min(COMMAND_SECONDS, int(remaining - 20))
            if limit <= 0:
                raise TimeoutError('preparation deadline exceeded')
            begin = now()
            action = dict(path=sdk, status='started', command_timeout_seconds=limit)
            receipt['actions'].append(action)
            update(receipt)
            before = receipt['after']['available_bytes']
            try:
                # The root-owned GNU timeout bounds rm; Python cannot reliably kill root sudo.
                run(['sudo', '-n', 'timeout', '--kill-after=2s', f'{limit}s', 'rm', '-rf',
                     '--one-file-system', '--', sdk], check=True, timeout=limit + 5,
                    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                action['status'] = 'removed'
            except (OSError, subprocess.SubprocessError) as error:
                action.update(status='error', error=str(error))
                if isinstance(error, PermissionError):
                    # Python's timeout is advisory for a root-owned sudo; root
                    # GNU timeout and the outer step guard supply the bounds.
                    action['python_timeout_advisory'] = True
                if isinstance(error, subprocess.CalledProcessError):
                    action['exit_code'] = error.returncode
                    if error.stderr:
                        action['stderr'] = (error.stderr.decode(errors='replace') if isinstance(error.stderr, bytes) else str(error.stderr))[:4096]
                raise
            finally:
                action['duration_seconds'] = max(0, now() - begin)
                try:
                    sync_limit = min(10, int(TOTAL_SECONDS - (now() - started) - 5))
                    if sync_limit <= 0:
                        raise TimeoutError('no time remains to sync target filesystem')
                    run(['timeout', '--kill-after=2s', f'{sync_limit}s', 'sync', '-f', '--', path],
                        check=True, timeout=sync_limit + 5, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                except (OSError, subprocess.SubprocessError) as error:
                    action['sync_error'] = str(error)
                    raise
                finally:
                    receipt['after'] = read_disk(path)
                    action['freed_bytes'] = receipt['after']['available_bytes'] - before
                    update(receipt)
        receipt['success'] = receipt['after']['available_bytes'] >= TARGET
        if not receipt['success']:
            receipt['errors'].append('insufficient free space after permitted preparation')
    except (OSError, ValueError, TimeoutError, subprocess.SubprocessError, KeyError) as error:
        receipt['errors'].append(str(error))
        receipt['success'] = False
    receipt['duration_seconds'] = max(0, now() - started)
    update(receipt)
    return receipt


def main():
    destination = Path('representative-results/runner-preparation')
    destination.mkdir(parents=True, exist_ok=True)
    def update(receipt):
        tmp = destination / 'receipt.json.tmp'
        tmp.write_text(json.dumps(receipt, indent=2, allow_nan=False) + '\n')
        os.replace(tmp, destination / 'receipt.json')
    receipt = prepare(os.environ, update=update)
    print(json.dumps(receipt, allow_nan=False))
    return 0 if receipt['success'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
