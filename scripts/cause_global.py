"""Global host context capture and bounded read primitives for cause diagnostics."""
import errno
import json
import os
import math
from pathlib import Path
import selectors
import subprocess
import tempfile
import time


class FixtureAccess:
    """Explicit test-only access to a private, tiny fixture; never real proc/sys."""
    def __init__(self, root):
        self.root = Path(root).resolve(strict=True)
        if (self.root.parent != Path(tempfile.gettempdir()).resolve()
                or not self.root.name.startswith("cause-observe-fixture-")
                or not self.root.is_dir()):
            raise ValueError("fixture must be a private cause-observe-fixture-* temp directory")

    def check(self, path):
        path = Path(path).absolute()
        if not path.resolve().is_relative_to(self.root):
            raise ValueError("fixture access escapes private root")


def _guard(paths, fixture):
    if fixture is not None:
        if not isinstance(fixture, FixtureAccess):
            raise TypeError("explicit FixtureAccess required")
        for path in paths:
            fixture.check(path)
    elif (os.environ.get("GITHUB_ACTIONS") != "true"
          or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"):
        raise RuntimeError("real diagnostics require GitHub-hosted Actions")


def _stamp():
    return {"monotonic_ns": time.monotonic_ns(), "wall_ns": time.time_ns()}


def _identity(s):
    return dict(device=s.st_dev, inode=s.st_ino, nlink=s.st_nlink,
                logical_bytes=s.st_size, blocks_512=s.st_blocks,
                allocated_bytes=s.st_blocks * 512, mode=s.st_mode)


def _open_dir(path):
    """Open every component without following symlinks, including parent paths."""
    path = Path(path).absolute()
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in path.parts[1:]:
            new = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC
                          | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = new
        return fd
    except BaseException:
        os.close(fd)
        raise


def _read(path, *, dir_fd=None):
    try:
        fd = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=dir_fd)
        with os.fdopen(fd, encoding="utf-8") as stream:
            raw = stream.read(2 * 1024 * 1024 + 1)
        if len(raw) > 2 * 1024 * 1024:
            raise ValueError("source exceeds diagnostic byte bound")
        return dict(available=True, path=str(path), raw=raw)
    except (OSError, ValueError, UnicodeError) as error:
        return dict(available=False, path=str(path), error=str(error),
                    error_type=type(error).__name__, errno=getattr(error, "errno", None))


def _sudo_slabinfo():
    """One fixed read command, bounded while streaming rather than after buffering."""
    _guard(["/proc"], None)
    command = ["sudo", "-n", "cat", "/proc/slabinfo"]
    result = dict(available=False, reader="fixed-path sudo cat", command=command,
                  before=_stamp(), timeout_seconds=5, maximum_output_bytes=2 * 1024 * 1024)
    proc = None
    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    try:
        deadline = time.monotonic() + 5
        proc = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        with selectors.DefaultSelector() as selector:
            selector.register(proc.stdout, selectors.EVENT_READ, "stdout")
            selector.register(proc.stderr, selectors.EVENT_READ, "stderr")
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("fixed slabinfo read exceeded five-second deadline")
                for key, _ in selector.select(remaining):
                    total = sum(len(value) for value in buffers.values())
                    chunk = os.read(key.fileobj.fileno(), min(65536, result["maximum_output_bytes"] + 1 - total))
                    buffers[key.data].extend(chunk)
                    if sum(len(value) for value in buffers.values()) > result["maximum_output_bytes"]:
                        raise ValueError("fixed slabinfo read exceeds two-MiB output bound")
                    if not chunk:
                        selector.unregister(key.fileobj)
            result["returncode"] = proc.wait(timeout=max(0, deadline - time.monotonic()))
        if result["returncode"] != 0:
            raise RuntimeError("fixed slabinfo read failed")
        result.update(available=True, raw=buffers["stdout"].decode("utf-8"))
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        result.update(error=str(error), error_type=type(error).__name__)
    finally:
        if proc is not None:
            if proc.poll() is None:
                try:
                    proc.kill()
                    proc.wait(timeout=0.2)
                except (OSError, subprocess.TimeoutExpired) as error:
                    result["cleanup_error"] = str(error)
            proc.stdout.close()
            proc.stderr.close()
        result.update(after=_stamp(), output_bytes=sum(len(value) for value in buffers.values()),
                      stderr=buffers["stderr"].decode("utf-8", errors="replace"))
    return result


def parse_slabinfo(raw):
    objects, errors = [], []
    for line in raw.splitlines():
        if not line or line.startswith(("#", "slabinfo")):
            continue
        fields = line.split()
        try:
            row = dict(name=fields[0], active_objects=int(fields[1]),
                       total_objects=int(fields[2]), object_size_bytes=int(fields[3]),
                       objects_per_slab=int(fields[4]), pages_per_slab=int(fields[5]))
            if "slabdata" in fields:
                i = fields.index("slabdata")
                row.update(active_slabs=int(fields[i + 1]), total_slabs=int(fields[i + 2]))
                row["total_slab_pages"] = row["total_slabs"] * row["pages_per_slab"]
            objects.append(row)
        except (IndexError, ValueError) as error:
            errors.append(dict(line=line, error=str(error)))
    return dict(schema="linux-slabinfo-2.x", objects=objects, parse_errors=errors,
                required_fields_available=bool(objects) and not errors,
                page_size_bytes=os.sysconf("SC_PAGE_SIZE"))


def background_sources(proc_fd):
    sources = {}
    for name, path in ((n, 'pressure/' + n) for n in ('cpu', 'io', 'memory')):
        row = _read(path, dir_fd=proc_fd)
        if row['available']:
            try:
                parsed = {}
                for line in row['raw'].splitlines():
                    kind, *fields = line.split()
                    need_unique = kind not in parsed
                    values = dict(field.split('=', 1) for field in fields)
                    values = {k: int(v) if k == 'total' else float(v) for k, v in values.items()}
                    if not need_unique or set(values) != {'avg10', 'avg60', 'avg300', 'total'} or any(
                            not math.isfinite(v) or v < 0 or (k != 'total' and v > 100) for k, v in values.items()):
                        raise ValueError('invalid PSI fields')
                    parsed[kind] = values
                if 'some' not in parsed or (name != 'cpu' and 'full' not in parsed):
                    raise ValueError('missing PSI class')
                row['parsed'] = parsed
            except (ValueError, TypeError) as error:
                row.update(available=False, error=str(error))
        sources[name] = row
    row = _read('loadavg', dir_fd=proc_fd)
    if row['available']:
        try:
            a, b, c, tasks, pid = row['raw'].split()
            averages = [float(v) for v in (a, b, c)]
            running, total = map(int, tasks.split('/'))
            if any(not math.isfinite(v) or v < 0 for v in averages) or not 0 <= running <= total:
                raise ValueError('invalid host load')
            row['parsed'] = dict(averages=averages, running=running, total=total, last_pid=int(pid))
        except (ValueError, TypeError) as error:
            row.update(available=False, error=str(error))
    sources['loadavg'] = row
    return dict(required_fields_available=all(r['available'] for r in sources.values()), sources=sources)


def capture_global(destination, *, proc_root="/proc", fixture=None, include_background=False):
    _guard([proc_root], fixture)
    before = _stamp()
    proc_fd = _open_dir(proc_root)
    try:
        sources = {name: _read(name, dir_fd=proc_fd) for name in ("slabinfo", "meminfo")}
        root_identity = _identity(os.fstat(proc_fd))
        background = background_sources(proc_fd) if include_background else None
    finally:
        os.close(proc_fd)
    for name, source in sources.items():
        source["path"] = str(Path(proc_root) / name)
        source["reader"] = "direct nofollow read"
    direct = sources["slabinfo"]
    if (fixture is None and str(proc_root) == "/proc" and not direct["available"]
            and direct.get("error_type") == "PermissionError"
            and direct.get("errno") in (errno.EACCES, errno.EPERM)):
        sources["slabinfo"] = dict(_sudo_slabinfo(), path="/proc/slabinfo", direct_attempt=direct)
    if sources["slabinfo"]["available"]:
        sources["slabinfo"]["parsed"] = parse_slabinfo(sources["slabinfo"]["raw"])
    if sources["meminfo"]["available"]:
        values, errors = {}, []
        for line in sources["meminfo"]["raw"].splitlines():
            try:
                key, value = line.split(":", 1)
                parts = value.split()
                values[key] = dict(value=int(parts[0]), unit=parts[1] if len(parts) > 1 else None)
            except (ValueError, IndexError) as error:
                errors.append(dict(line=line, error=str(error)))
        sources["meminfo"]["parsed"] = dict(fields=values, parse_errors=errors)
    result = dict(schema=1, scope="GLOBAL host; never per-cgroup attribution",
                  before=before, after=_stamp(), sources=sources,
                  root_path=str(proc_root), root_identity=root_identity)
    if background is not None:
        result['background'] = background
    result["required_fields_available"] = bool(
        sources["slabinfo"].get("parsed", {}).get("required_fields_available")
        and sources["meminfo"].get("parsed", {}).get("fields")
        and not sources["meminfo"].get("parsed", {}).get("parse_errors")
        and (background is None or background['required_fields_available']))
    destination = Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    for name, row in sources.items():
        if row["available"]:
            (destination / (name + ".raw")).write_text(row["raw"])
    (destination / "global.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


