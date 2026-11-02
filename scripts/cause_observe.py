"""Bounded, read-only cause diagnostics; real observations require a hosted GHA VM.

FixtureAccess is restricted to private temporary test directories. Captures are
bracketed observations, never atomic inventories or exact ordinary-run peaks.
"""
import os
from pathlib import Path, PurePosixPath
import sqlite3
import stat
import tempfile
import time
from urllib.parse import quote


from cause_global import (FixtureAccess, _guard, _stamp, _identity, _open_dir, _read,
                          capture_global, parse_slabinfo)


CGROUP_FILES = ("memory.current", "memory.peak", "memory.stat", "memory.events",
                "memory.events.local", "memory.pressure", "memory.max", "memory.swap.max",
                "memory.swap.current", "io.stat", "io.max", "io.pressure", "cpu.stat",
                "cpu.max", "cpu.pressure", "cgroup.procs", "cgroup.type")


def capture_cgroup(pid, *, proc_root="/proc", cgroup_root="/sys/fs/cgroup", fixture=None):
    _guard([proc_root, cgroup_root], fixture)
    pid = int(pid)
    if pid <= 0:
        raise ValueError("positive host PID required")
    before = _stamp()
    pid_root = Path(proc_root) / str(pid)
    pid_fd = _open_dir(pid_root)
    try:
        membership = _read("cgroup", dir_fd=pid_fd)
        process_before = _read("stat", dir_fd=pid_fd)
    finally:
        os.close(pid_fd)
    if not membership["available"]:
        raise RuntimeError("host PID cgroup is unavailable: " + membership.get("error", ""))
    lines = [line[3:] for line in membership["raw"].splitlines() if line.startswith("0::")]
    if len(lines) != 1:
        raise ValueError("exactly one unified v2 membership required")
    relative = PurePosixPath(lines[0])
    if not relative.is_absolute() or ".." in relative.parts:
        raise ValueError("unsafe cgroup membership")
    group = Path(cgroup_root).absolute().joinpath(*relative.parts[1:])
    group_fd = _open_dir(group)
    try:
        identity = _identity(os.fstat(group_fd))
        sources = {name: _read(name, dir_fd=group_fd) for name in CGROUP_FILES}
        for name, source in sources.items():
            source["path"] = str(group / name)
    finally:
        os.close(group_fd)
    process_after = _read(pid_root / "stat")
    membership_after = _read(pid_root / "cgroup")
    def start_ticks(row):
        try:
            return row["raw"].rsplit(")", 1)[1].split()[19]
        except (KeyError, IndexError):
            return None
    a, b = start_ticks(process_before), start_ticks(process_after)
    stable = (a is not None and a == b and membership_after.get("available")
              and membership_after.get("raw") == membership["raw"])
    return dict(schema=1, scope="host PID unified v2 cgroup; no container namespace fallback",
                pid=pid, before=before, after=_stamp(), cgroup_path=str(group),
                root_path=str(Path(cgroup_root).absolute()), root_identity=_identity(Path(cgroup_root).stat()),
                group_identity=identity, process_identity_stable=bool(stable),
                process_stat_before=process_before, process_stat_after=process_after,
                membership=membership, membership_after=membership_after, sources=sources)


class PinSet:
    """Own exactly the requested descriptors; context manager closes all pins."""
    def __init__(self, snapshot_dir, *, fixture=None, inspection_aliases=False):
        self.root = Path(snapshot_dir).absolute()
        _guard([self.root], fixture)
        self.fixture = fixture
        self.root_fd = _open_dir(self.root)
        self.root_identity = _identity(os.fstat(self.root_fd))
        self.pins = {}
        self.aliases, self.alias_dir, self.alias_fd = {}, None, None
        try:
            os.stat("snapshot.stage.tmp", dir_fd=self.root_fd, follow_symlinks=False)
        except FileNotFoundError:
            self.fresh_stage_absent = True
        except OSError:
            self.close()
            raise
        else:
            self.close()
            raise ValueError("fresh diagnostic case must have no stage path")
        if inspection_aliases:
            try:
                self.alias_dir = Path(tempfile.mkdtemp(prefix=".cause-inspect-", dir=self.root.parent))
                self.alias_fd = _open_dir(self.alias_dir)
                if os.fstat(self.alias_fd).st_dev != self.root_identity["device"]:
                    raise ValueError("inspection aliases must share snapshot filesystem")
            except BaseException:
                self.close()
                raise

    def _pin(self, name, filename):
        if name in self.pins:
            raise ValueError("pin already held")
        fd = os.open(filename, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK,
                     dir_fd=self.root_fd)
        try:
            info = _identity(os.fstat(fd))
            if not stat.S_ISREG(info["mode"]) or info["device"] != self.root_identity["device"]:
                raise ValueError("pin must be a regular file on snapshot device")
            if self.alias_fd is not None:
                os.link(filename, name + ".db", src_dir_fd=self.root_fd,
                        dst_dir_fd=self.alias_fd, follow_symlinks=False)
                self.aliases[name] = info
                linked = os.stat(name + ".db", dir_fd=self.alias_fd, follow_symlinks=False)
                if (linked.st_dev, linked.st_ino) != (info["device"], info["inode"]):
                    self.aliases[name] = _identity(linked)
                    raise ValueError("source changed while linking inspection alias")
            self.pins[name] = dict(fd=fd, filename=filename, opened=info, opened_at=_stamp())
            return dict(name=name, filename=filename, opened=info, opened_at=self.pins[name]["opened_at"])
        except BaseException:
            os.close(fd)
            raise

    def pin_old_previous(self):
        return self._pin("previous", "snapshot.prev.db")

    def pin_stage(self):
        return self._pin("stage", "snapshot.stage.tmp")

    def fd(self, name):
        return self.pins[name]["fd"]

    def inspection_paths(self):
        return {name: str(self.alias_dir / (name + ".db")) for name in self.aliases}

    def drop_inspection_aliases(self):
        errors = []
        for name, owned in list(self.aliases.items()):
            try:
                s = os.stat(name + ".db", dir_fd=self.alias_fd, follow_symlinks=False)
                if (s.st_dev, s.st_ino) != (owned["device"], owned["inode"]):
                    raise ValueError("inspection alias replaced; refusing to unlink foreign inode")
                os.unlink(name + ".db", dir_fd=self.alias_fd)
                self.aliases.pop(name)
            except FileNotFoundError as error:
                self.aliases.pop(name)
                errors.append(str(error))
            except (OSError, ValueError) as error:
                errors.append(str(error))
        if self.alias_fd is not None and not self.aliases:
            os.close(self.alias_fd)
            self.alias_fd = None
        if self.alias_dir is not None and not self.aliases:
            try:
                self.alias_dir.rmdir()
                self.alias_dir = None
            except OSError as error:
                errors.append(str(error))
        return dict(available=not errors, errors=errors, remaining_aliases=list(self.aliases))

    def release(self, name):
        if name in self.aliases:
            raise ValueError("drop inspection aliases before releasing a pin")
        pin = self.pins.pop(name)
        info = _identity(os.fstat(pin["fd"]))
        os.close(pin["fd"])
        return dict(name=name, released=info, after=_stamp())

    def close_stage(self):
        return self.release("stage")

    def close_previous(self):
        return self.release("previous")

    def close(self):
        self.drop_inspection_aliases()
        self.aliases.clear()  # Failed foreign-link checks must not leak our descriptors.
        if self.alias_fd is not None:
            os.close(self.alias_fd)
            self.alias_fd = None
        for name in list(self.pins):
            self.release(name)
        if self.root_fd is not None:
            os.close(self.root_fd)
            self.root_fd = None

    def __enter__(self):
        return self

    def __exit__(self, *unused):
        self.close()


def record_inventory(snapshot_dir, pins, *, max_entries=128):
    if not isinstance(pins, PinSet) or pins.root_fd is None:
        raise ValueError("live PinSet required")
    if Path(snapshot_dir).absolute() != pins.root:
        raise ValueError("inventory must use pinned snapshot root")
    if not 1 <= max_entries <= 128:
        raise ValueError("inventory entry bound must be 1..128")
    before, entries, errors, excluded = _stamp(), {}, [], []
    try:
        root_readback = _identity(pins.root.stat(follow_symlinks=False))
        root_matches = (root_readback["device"], root_readback["inode"]) == (
            pins.root_identity["device"], pins.root_identity["inode"])
    except OSError as error:
        root_readback, root_matches = dict(error=str(error)), False
    count = 0
    def add(alias, s):
        key = f"{s.st_dev}:{s.st_ino}"
        info = _identity(s)
        if key in entries:
            if entries[key]["observation"] != info:
                entries[key]["raced"] = True
            entries[key]["aliases"].append(alias)
        else:
            entries[key] = dict(observation=info, aliases=[alias], raced=False)
    def walk(fd, prefix):
        nonlocal count
        with os.scandir(fd) as scan:
            for entry in scan:
                count += 1
                if count > max_entries:
                    raise ValueError("inventory entry bound exceeded")
                path = prefix + entry.name
                try:
                    s = entry.stat(follow_symlinks=False)
                    if s.st_dev != pins.root_identity["device"] or stat.S_ISLNK(s.st_mode):
                        excluded.append(dict(path=path, reason="symlink or other device"))
                    elif stat.S_ISREG(s.st_mode):
                        add(dict(kind="named", path=path), s)
                    elif stat.S_ISDIR(s.st_mode):
                        sub = os.open(entry.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
                                      | os.O_CLOEXEC, dir_fd=fd)
                        try:
                            if os.fstat(sub).st_dev != pins.root_identity["device"]:
                                raise ValueError("directory device changed")
                            walk(sub, path + "/")
                        finally:
                            os.close(sub)
                    else:
                        excluded.append(dict(path=path, reason="not a regular file/directory"))
                except OSError as error:
                    errors.append(dict(path=path, error=str(error)))
    vfs_before = _filesystem(pins.root_fd)
    try:
        walk(pins.root_fd, "")
    except ValueError as error:
        errors.append(dict(error=str(error)))
    pin_rows = {}
    for name, pin in pins.pins.items():
        s = os.fstat(pin["fd"])
        add(dict(kind="pin", name=name, filename=pin["filename"]), s)
        try:
            named = os.stat(pin["filename"], dir_fd=pins.root_fd, follow_symlinks=False)
            path_same = (named.st_dev, named.st_ino) == (s.st_dev, s.st_ino)
            path_state = "same_inode" if path_same else "replaced"
        except FileNotFoundError:
            path_state = "absent"
        pin_rows[name] = dict(observation=_identity(s), path_state=path_state,
                              app_path_retired=path_state in ("absent", "replaced"),
                              naturally_unlinked=s.st_nlink == 0 and path_state in ("absent", "replaced"))
    for name, path in pins.inspection_paths().items():
        s = os.stat(name + ".db", dir_fd=pins.alias_fd, follow_symlinks=False)
        add(dict(kind="inspection_alias", path=path, intervention="deliberate hardlink retention"), s)
    vfs_after = _filesystem(pins.root_fd)
    return dict(schema=1, before=before, after=_stamp(), root_path=str(pins.root),
                root_identity=pins.root_identity, scope="bounded named files plus explicit pins; not all process descriptors",
                root_path_readback=root_readback, root_path_matches=root_matches,
                complete=root_matches and not errors and not excluded and not any(r["raced"] for r in entries.values()),
                entries_seen=count, entries=entries, pins=pin_rows, errors=errors, excluded=excluded,
                allocated_bytes=sum(r["observation"]["allocated_bytes"] for r in entries.values()),
                logical_bytes=sum(r["observation"]["logical_bytes"] for r in entries.values()),
                filesystem_before=vfs_before, filesystem_after=vfs_after,
                limitation="bracketed traversal, not atomic; no independent peaks are summed")


def _filesystem(fd):
    s = os.fstatvfs(fd)
    return dict(at=_stamp(), block_size=s.f_bsize, fragment_size=s.f_frsize,
                used_bytes=(s.f_blocks - s.f_bfree) * s.f_frsize,
                available_bytes=s.f_bavail * s.f_frsize, free_bytes=s.f_bfree * s.f_frsize)


def settling_samples(pins, *, seconds=0.5, interval=0.05):
    """Immediate plus bounded samples, without sync, cache flush or peak claims."""
    if not 0 <= seconds <= 2 or not 0.01 <= interval <= 1:
        raise ValueError("settling budget exceeds bounds")
    rows, deadline = [], time.monotonic() + seconds
    while True:
        rows.append(_filesystem(pins.root_fd))
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return dict(samples=rows, scope="whole filesystem; metadata and activity included",
                        quantization_bytes=rows[0]["fragment_size"],
                        residual_bound="unbounded without filesystem exclusivity and aligned inventories")
        time.sleep(min(interval, remaining))


def release_measurement(pins, name, *, seconds=0.5):
    """Measure a pin release step; filesystem residual is not automatically causal."""
    before = _filesystem(pins.root_fd)
    release = pins.release(name)
    result = settling_samples(pins, seconds=seconds)
    expected = release["released"]["allocated_bytes"]
    result.update(before=before, release=release, expected_inode_bytes=expected,
                  requires_unlinked=release["released"]["nlink"] == 0,
                  comparisons=[dict(free_delta_bytes=row["free_bytes"] - before["free_bytes"],
                                    residual_bytes=row["free_bytes"] - before["free_bytes"] - expected,
                                    quantization_bytes=row["fragment_size"])
                               for row in result["samples"]])
    return result


def db_breakdown(path_or_fd, *, deadline_seconds=10, fixture=None):
    """Inspect a quiescent standalone database; close SQLite before caller unpins.

    The caller binds current E826 journal-OFF stage/output provenance. Immutable
    aliases cannot discover sidecars beside another pathname for the inode;
    header versions and auto_vacuum remain explicit evidence for host validation.
    """
    if not 0 < deadline_seconds <= 30:
        raise ValueError("SQLite deadline must be in (0,30]")
    is_fd = isinstance(path_or_fd, int)
    path = Path(f"/proc/self/fd/{path_or_fd}") if is_fd else Path(path_or_fd).absolute()
    if is_fd:
        _guard([], fixture)
        s = os.fstat(path_or_fd)
        if fixture is not None:
            target = os.readlink(path)
            fixture.check(target.removesuffix(" (deleted)"))
        if s.st_nlink != 0:
            raise ValueError("FD database inspection requires an unlinked quiescent pin")
    else:
        _guard([path], fixture)
        parent = _open_dir(path.parent)
        try:
            s = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
            for suffix in ("-wal", "-journal"):
                try:
                    sidecar = os.stat(path.name + suffix, dir_fd=parent, follow_symlinks=False)
                except FileNotFoundError:
                    continue
                if not stat.S_ISREG(sidecar.st_mode) or sidecar.st_size:
                    raise ValueError("immutable inspection refuses nonempty/unsafe sidecar " + suffix)
        finally:
            os.close(parent)
    if not stat.S_ISREG(s.st_mode):
        raise ValueError("database must be a regular file")
    before = _stamp()
    result = dict(schema=1, source=str(path), source_identity=_identity(s), before=before,
                  immutable=True, read_only=True, quiescent_required=True, available=False, errors=[],
                  sqlite_version=sqlite3.sqlite_version,
                  sidecar_limitation="immutable inspection requires quiescence; alias names do not discover sidecars beside the original name")
    result["deadline_scope"] = "SQLite VM progress checks, not interruption of blocking kernel I/O"
    conn = None
    try:
        header_fd = path_or_fd if is_fd else os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
        try:
            header_stat = os.fstat(header_fd)
            if (header_stat.st_dev, header_stat.st_ino) != (s.st_dev, s.st_ino):
                raise ValueError("source changed before header read")
            header = os.pread(header_fd, 100, 0)
        finally:
            if not is_fd:
                os.close(header_fd)
        result["header"] = dict(available=len(header) == 100, bytes_read=len(header),
                                signature_valid=header[:16] == b"SQLite format 3\x00")
        if len(header) != 100 or not result["header"]["signature_valid"]:
            raise ValueError("missing/invalid 100-byte SQLite header")
        result["header"].update(write_version=header[18], read_version=header[19])
        uri = "file:" + quote(str(path), safe="/") + "?mode=ro&immutable=1"
        conn = sqlite3.connect(uri, uri=True, timeout=0)
        deadline = time.monotonic() + deadline_seconds
        conn.set_progress_handler(lambda: int(time.monotonic() > deadline), 1000)
        result["compile_options"] = [r[0] for r in conn.execute("PRAGMA compile_options")]
        result["table_schema"] = conn.execute("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name").fetchall()
        if fixture is not None:
            tables = [r[1] for r in result["table_schema"] if r[0] == "table" and not r[1].startswith("sqlite_")]
            if sum(conn.execute('SELECT count(*) FROM "' + t.replace('"', '""') + '"').fetchone()[0]
                   for t in tables) > 64:
                raise ValueError("private SQLite fixture exceeds 64 rows")
        result["pragmas"] = {name: conn.execute("PRAGMA " + name).fetchone()[0]
                             for name in ("page_size", "page_count", "freelist_count", "auto_vacuum")}
        result["dbstat_columns"] = ["name", "pagetype", "pages", "bytes", "payload", "unused"]
        result["dbstat"] = conn.execute("SELECT name,pagetype,count(*),sum(pgsize),sum(payload),sum(unused) FROM dbstat GROUP BY name,pagetype ORDER BY name,pagetype").fetchall()
        result["available"] = True
    except (sqlite3.Error, ValueError, OSError) as error:
        result["errors"].append(str(error))
    finally:
        if conn is not None:
            conn.close()
        try:
            after_source = os.fstat(path_or_fd) if is_fd else path.stat(follow_symlinks=False)
            result["source_identity_after"] = _identity(after_source)
            result["source_identity_stable"] = all(getattr(s, k) == getattr(after_source, k)
                for k in ("st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns"))
        except OSError as error:
            result["source_identity_stable"] = False
            result["errors"].append(str(error))
        if not result["source_identity_stable"]:
            result["available"] = False
            result["errors"].append("source identity/content metadata changed during inspection")
        result["after"] = _stamp()
        result["query_connection_closed"] = True
    return result
