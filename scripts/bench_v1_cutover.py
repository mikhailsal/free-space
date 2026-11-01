"""Disposable, controlled-fixture backend cutover rehearsal for GitHub runners."""
import argparse
import fcntl
import hashlib
import json
import os
import re
import select
import signal
from pathlib import Path
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time

MIN_BYTES = 768 * 1024 * 1024
MIN_INODES = 128
DEADLINE = float('inf')
V1_GENERATED = {'snapshot.db', 'snapshot.prev.db', 'state.json', 'progress.json'}
V2_GENERATED = {'generations.db', 'generations.db-wal', 'generations.db-shm',
                'state.json', 'progress.json'}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def runner_temp(env, uid):
    require(env.get('GITHUB_ACTIONS') == 'true' and env.get('RUNNER_ENVIRONMENT') == 'github-hosted',
            'GitHub-hosted runner required')
    require(uid != 0, 'root execution refused')
    raw = env.get('RUNNER_TEMP', '')
    path = Path(raw)
    require(path.is_absolute() and path.is_dir() and path.resolve() == path,
            'RUNNER_TEMP must be an existing canonical absolute directory')
    require(path.stat().st_uid == uid, 'RUNNER_TEMP owner mismatch')
    return path


def capacity(path):
    v = os.statvfs(path)
    require(v.f_bavail * v.f_frsize >= MIN_BYTES and v.f_favail >= MIN_INODES,
            'insufficient available bytes or inodes')


def check_file(path):
    meta = path.lstat()
    require(stat.S_ISREG(meta.st_mode) and meta.st_uid == os.geteuid() and meta.st_nlink == 1,
            'unexpected generated file type, owner or link count')


def reset_owned(root, reader):
    """Only this private fixture's tracked child reader is checked; no host inventory."""
    require(root.is_absolute() and root.resolve() == root, 'fixture snapshot path is not canonical')
    for directory in (root, root / 'v2'):
        info = directory.lstat()
        require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.geteuid()
                and not directory.is_symlink(), 'fixture directory invalid')
    require(reader.poll() is not None, 'controlled fixture reader remains open')
    locks = []
    try:
        for directory in (root, root / 'v2'):
            fd = os.open(directory / '.lock', os.O_RDWR | os.O_NOFOLLOW)
            locks.append(fd)
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        lease = root / 'v2' / 'generations.db.readers'
        check_file(lease)
        fd = os.open(lease, os.O_RDONLY | os.O_NOFOLLOW)
        locks.append(fd)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        named, held = lease.stat(), os.fstat(fd)
        require((named.st_dev, named.st_ino) == (held.st_dev, held.st_ino), 'lease inode changed')
        capacity(root)
        removals = []
        for directory in (root, root / 'v2'):
            for entry in directory.iterdir():
                if directory == root and entry.name == 'v2':
                    require(entry.is_dir() and not entry.is_symlink(), 'invalid v2 directory')
                    continue
                if entry.name == '.lock' or (directory == root / 'v2' and entry.name == 'generations.db.readers'):
                    check_file(entry)
                    continue
                if directory == root and entry.name == 'unrelated.keep':
                    check_file(entry)
                    continue
                allowed = V1_GENERATED if directory == root else V2_GENERATED
                require(entry.name in allowed, 'unexpected snapshot path')
                check_file(entry)
                removals.append(entry)
        allocated_before = sum(path.stat().st_blocks * 512 for path in removals)
        available_before = os.statvfs(root).f_bavail * os.statvfs(root).f_frsize
        for path in removals:
            path.unlink()
        capacity(root)
        after = os.statvfs(root)
        return {'removed_allocated_bytes': allocated_before,
                'remaining_named_paths_allocated_bytes': sum(
                    path.stat().st_blocks * 512 for path in removals if path.exists()),
                'available_before_bytes': available_before,
                'available_after_bytes': after.f_bavail * after.f_frsize}
    finally:
        for fd in reversed(locks):
            os.close(fd)


def fixture_layout(root):
    entries = []
    for directory, prefix in ((root, ''), (root / 'v2', 'v2/')):
        for entry in directory.iterdir():
            require(len(entries) < 128, 'fixture layout exceeds bound')
            mode = entry.lstat().st_mode
            kind = 'symlink' if stat.S_ISLNK(mode) else 'directory' if stat.S_ISDIR(mode) else 'file'
            entries.append({'path': prefix + entry.name, 'kind': kind})
    return sorted(entries, key=lambda item: item['path'])


class RehearsalError(Exception):
    def __init__(self, message, *, returncode=None, stderr=None):
        super().__init__(message)
        self.returncode = returncode
        self.stderr = stderr


def sanitized(text, fixture):
    text = text.replace(str(fixture), '<fixture>')
    return re.sub(r'/[^\s]+', '<path>', text)[:2048]


def call(binary, env, *args, success=True):
    remaining = DEADLINE - time.monotonic()
    require(remaining > 0, 'rehearsal deadline expired')
    try:
        result = subprocess.run([str(binary), *args, '--json'], env=env, capture_output=True,
                                text=True, timeout=min(30, remaining), check=False)
    except subprocess.TimeoutExpired as error:
        raise RehearsalError('CLI command timed out', stderr=sanitized(str(error.stderr or ''), env['HOME'])) from error
    stderr = sanitized(result.stderr, env['HOME'])
    if (result.returncode == 0) != success:
        raise RehearsalError('CLI exit differs from expectation', returncode=result.returncode, stderr=stderr)
    try:
        return json.loads(result.stdout)
    except (ValueError, TypeError) as error:
        raise RehearsalError('CLI output is not JSON', returncode=result.returncode, stderr=stderr) from error


def verify_fresh(binary, env, root, snapshot, config, saved_config, config_inode):
    try:
        config.write_text('backend = "v1"\nlow_memory = true\nkeep_previous = true\n')
        report = call(binary, env, 'scan', str(root))
        status = call(binary, env, 'status')
        listing = call(binary, env, 'ls', str(root))
        require(listing.get('generation') == report.get('generation'), 'fresh query generation differs')
        diff = call(binary, env, 'diff', str(root), success=False)
        require(diff.get('error', {}).get('code') == 'NO_PREV', 'fresh v1 history is not NO_PREV')
        require(report.get('generation') == 1 and report.get('entries') == 2
                and status.get('generation') == 1,
                'fresh v1 count or status differs')
        with sqlite3.connect(f'file:{snapshot / "snapshot.db"}?mode=ro&immutable=1', uri=True) as db:
            require(db.execute('PRAGMA integrity_check').fetchone() == ('ok',), 'integrity check failed')
            require(db.execute('SELECT count(*) FROM entries').fetchone() == (2,), 'snapshot count differs')
            names = {row[0] for row in db.execute('SELECT name FROM entries')}
            require(b'fresh-only' in names and b'sample' not in names, 'fresh snapshot contains stale content')
        require(not (snapshot / 'snapshot.prev.db').exists(), 'unexpected previous snapshot')
    finally:
        active = sys.exc_info()[1]
        try:
            config.write_bytes(saved_config)
            require(config.read_bytes() == saved_config and config.stat().st_ino == config_inode,
                    'fixture configuration was not recoverable')
        except Exception as restore_error:
            if active is None:
                raise
            note = 'fixture configuration restore failed: ' + type(restore_error).__name__
            if hasattr(active, 'add_note'):
                active.add_note(note)
            else:
                active.__notes__ = [*getattr(active, '__notes__', []), note]


def await_reader(reader):
    deadline = time.monotonic() + 5
    data = b''
    while len(data) < 6:
        remaining = deadline - time.monotonic()
        require(remaining > 0, 'fixture reader startup timed out')
        ready, _, _ = select.select([reader.stdout], [], [], remaining)
        require(bool(ready), 'fixture reader startup timed out')
        chunk = os.read(reader.stdout.fileno(), 6 - len(data))
        require(bool(chunk), 'fixture reader exited before readiness')
        data += chunk
    require(data == b'ready\n' and reader.poll() is None, 'fixture reader not ready')


def close_reader(reader):
    try:
        if reader.stdin is not None:
            try:
                reader.stdin.close()
            except (OSError, ValueError):
                pass
        try:
            reader.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                reader.terminate()
            except ProcessLookupError:
                pass
            try:
                reader.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    reader.kill()
                except ProcessLookupError:
                    pass
                reader.wait(timeout=5)
    finally:
        if reader.stdout is not None:
            reader.stdout.close()


def rehearsal(binary, parent, update):
    require(binary.is_absolute() and not binary.is_symlink()
            and binary.resolve(strict=True) == binary and binary.is_file(),
            'candidate binary must be an absolute canonical regular file')
    with tempfile.TemporaryDirectory(prefix='freespace-cutover-', dir=parent) as temp:
        home = Path(temp)
        require(home.stat().st_uid == os.geteuid() and stat.S_IMODE(home.stat().st_mode) == 0o700,
                'fixture must be private and owned')
        root, snapshot = home / 'tree', home / 'snapshot'
        root.mkdir(); snapshot.mkdir(); (snapshot / 'v2').mkdir()
        (root / 'sample').write_text('sample\n')
        (snapshot / 'unrelated.keep').write_text('keep\n')
        config = home / 'config.toml'
        config.write_text('backend = "v2"\nlow_memory = true\nkeep_previous = true\n')
        saved_config = config.read_bytes()
        env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home),
               'XDG_STATE_HOME': str(home / 'state'), 'FREESPACE_CONFIG': str(config),
               'FREESPACE_SNAPSHOT_DIR': str(snapshot)}
        capacity(snapshot)
        # Generate old v1 and v2 history only inside the private fixture.
        old_v1 = call(binary, env, '--backend', 'v1', 'scan', str(root))
        old_v2 = call(binary, env, '--backend', 'v2', 'scan', str(root))
        old_v2_next = call(binary, env, '--backend', 'v2', 'scan', str(root))
        require(old_v1.get('generation') == 1 and old_v2.get('generation') == 1
                and old_v2_next.get('generation') == 2, 'fixture history missing')
        (root / 'sample').unlink()
        (root / 'fresh-only').write_text('fresh replacement\n')
        locks = [snapshot / '.lock', snapshot / 'v2' / '.lock',
                 snapshot / 'v2' / 'generations.db.readers']
        lock_inodes = [(p.stat().st_dev, p.stat().st_ino) for p in locks]
        protected = [binary, config, snapshot / 'unrelated.keep']
        before = [(p.stat().st_dev, p.stat().st_ino, hashlib.sha256(p.read_bytes()).hexdigest()) for p in protected]
        update({'status': 'in_progress', 'layout_before_reset': fixture_layout(snapshot)})
        old_databases = [snapshot / 'snapshot.db', snapshot / 'v2' / 'generations.db']
        old_hashes = [hashlib.sha256(p.read_bytes()).hexdigest() for p in old_databases]
        reader = subprocess.Popen([sys.executable, '-c',
            'import sys; f=open(sys.argv[1],"rb"); print("ready",flush=True); sys.stdin.readline(); f.close()',
            str(snapshot / 'v2' / 'generations.db')], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        try:
            await_reader(reader)
            try:
                reset_owned(snapshot, reader)
            except ValueError as error:
                require(str(error) == 'controlled fixture reader remains open', 'unexpected reader refusal')
            else:
                raise ValueError('live fixture reader was not refused')
        finally:
            close_reader(reader)
        require([hashlib.sha256(p.read_bytes()).hexdigest() for p in old_databases] == old_hashes,
                'reader refusal changed history')
        # Demonstrate writer refusal before deletion while holding the actual v2 lock.
        with (snapshot / 'v2' / '.lock').open('r+') as held:
            fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
            try:
                reset_owned(snapshot, reader)
            except BlockingIOError:
                pass
            else:
                raise ValueError('busy writer was not refused')
        require([hashlib.sha256(p.read_bytes()).hexdigest() for p in old_databases] == old_hashes,
                'writer refusal changed history')
        reset = reset_owned(snapshot, reader)
        require([(p.stat().st_dev, p.stat().st_ino) for p in locks] == lock_inodes, 'lock inode changed')
        require([(p.stat().st_dev, p.stat().st_ino, hashlib.sha256(p.read_bytes()).hexdigest()) for p in protected] == before,
                'protected file changed')
        require(config.read_bytes() == saved_config, 'config backup changed')
        verify_fresh(binary, env, root, snapshot, config, saved_config, before[1][1])
        require(sorted(p.name for p in (snapshot / 'v2').iterdir()) == ['.lock', 'generations.db.readers'], 'unexpected v2 remnants')
        require([(p.stat().st_dev, p.stat().st_ino) for p in locks] == lock_inodes, 'lock inode changed after scan')
        require(hashlib.sha256((snapshot / 'unrelated.keep').read_bytes()).hexdigest() == before[2][2],
                'unrelated file changed after scan')
        return {'valid': True, 'reader_scope': 'controlled fixture child only', 'entries': 2,
                'reset_capacity': reset,
                'previous': 'NO_PREV', 'binary_sha256': before[0][2],
                'note': 'Production reader quiescence requires stopped services and privileged inventory.'}


def atomic_receipt(output, value):
    fd, name = tempfile.mkstemp(prefix='.cutover-receipt-', dir=output.parent)
    try:
        with os.fdopen(fd, 'w') as receipt:
            json.dump(value, receipt, indent=2)
            receipt.write('\n')
            receipt.flush()
            os.fsync(receipt.fileno())
        os.replace(name, output)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def main():
    global DEADLINE
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    parent = runner_temp(os.environ, os.geteuid())
    output = args.output
    require(output.is_absolute() and output.parent.resolve() == parent and not output.exists(),
            'output must be a new direct RUNNER_TEMP child')
    target = os.environ.get('BENCH_TARGET', '')
    require(target in ('x86_64-unknown-linux-musl', 'x86_64-unknown-linux-gnu'),
            'explicit benchmark target required')
    state = {'valid': False, 'status': 'in_progress', 'target': target,
             'reader_scope': 'controlled fixture child only'}
    fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as receipt:
        json.dump(state, receipt, indent=2)
        receipt.write('\n')
        receipt.flush()
        os.fsync(receipt.fileno())
    def update(fields):
        state.update(fields)
        atomic_receipt(output, state)
    def interrupt(signum, _frame):
        signal.signal(signum, signal.SIG_IGN)
        raise KeyboardInterrupt('received termination signal')
    handlers = {sig: signal.signal(sig, interrupt) for sig in (signal.SIGINT, signal.SIGTERM)}
    DEADLINE = time.monotonic() + 240
    try:
        result = rehearsal(args.binary, parent, update)
        update({**result, 'status': 'success'})
    except BaseException as error:
        interrupted = isinstance(error, (KeyboardInterrupt, SystemExit))
        fields = {'valid': False, 'status': 'interrupted' if interrupted else 'error',
                  'error_type': type(error).__name__, 'message': sanitized(str(error), parent)}
        if isinstance(error, RehearsalError):
            fields.update(returncode=error.returncode, stderr=error.stderr)
        if getattr(error, '__notes__', None):
            fields['notes'] = [sanitized(note, parent) for note in error.__notes__]
        try:
            update(fields)
        except OSError:
            pass  # Initial valid JSON receipt remains the recovery evidence.
        raise
    finally:
        for sig, handler in handlers.items():
            signal.signal(sig, handler)


if __name__ == '__main__':
    main()
