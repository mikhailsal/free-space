"""Shared container execution for isolated v1 commands and read-only observers."""
from contextlib import closing
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import time
import uuid


def sqlite_i64(value):
    if type(value) is not int or not 0 <= value < 2**64:
        raise ValueError('expected u64')
    return value if value < 2**63 else value - 2**64


def legacy():
    spec = importlib.util.spec_from_file_location("bench_legacy", Path(__file__).with_name("bench-v2-container.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def file_hash(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def seed_hash(snapshot, *, include_files=False):
    files = {}
    for path in sorted(snapshot.rglob("*")):
        if path.is_symlink():
            raise ValueError("symlink in seed")
        if path.is_file():
            if path.name.endswith("-wal") and path.stat().st_size:
                raise ValueError("seed contains nonempty WAL")
            files[str(path.relative_to(snapshot))] = file_hash(path)
    if "snapshot.db" not in files or "snapshot.prev.db" not in files:
        raise ValueError("seed has no databases")
    digest = hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()
    return (digest, files) if include_files else digest


def full_link_facts(snapshot, plan, phase, deadline):
    """Bounded component lookups of the two designated inode groups."""
    deadline = min(deadline, time.monotonic() + 10)
    if time.monotonic() >= deadline:
        raise TimeoutError('hardlink lookup deadline exceeded')
    with closing(sqlite3.connect(snapshot.as_uri() + '?mode=ro&immutable=1', uri=True, timeout=0)) as db:
        db.set_progress_handler(lambda: int(time.monotonic() >= deadline), 1000)
        root = db.execute('SELECT entry_id FROM roots WHERE path=?', (b'/scratch/tree',)).fetchone()
        if root is None:
            raise ValueError('missing fixture root')
        def get(path):
            ident = root[0]
            for component in path.split('/'):
                if time.monotonic() >= deadline:
                    raise TimeoutError('hardlink lookup deadline exceeded')
                row = db.execute('SELECT id FROM entries WHERE parent_id=? AND name=?', (ident, component.encode())).fetchall()
                if len(row) != 1:
                    raise ValueError('missing or ambiguous fixture path: ' + path)
                ident = row[0][0]
            row = db.execute('SELECT e.flags,e.allocated,e.apparent,e.mtime,e.nlink,h.dev,h.ino,h.allocated,h.apparent FROM entries e LEFT JOIN hardlinks h ON h.entry_id=e.id WHERE e.id=?', (ident,)).fetchone()
            return list(row)
        paths = ([plan['old_owner_path'], plan['old_outside_path'], plan['new_owner_path']]
                 if phase == 'seed' else [plan['old_outside_path'], plan['new_owner_path'], plan['new_link_path']])
        facts = {path: get(path) for path in paths}
    old = facts[plan['old_owner_path'] if phase == 'seed' else plan['old_outside_path']]
    outside_old = facts[plan['old_outside_path']]
    new_owner = facts[plan['new_owner_path']]
    def hardlink(row, prefix):
        return row[5:9] == [sqlite_i64(plan[prefix + '_dev']), sqlite_i64(plan[prefix + '_inode']), plan[prefix + '_allocated'], plan[prefix + '_size']]
    if not (hardlink(old, 'old_owner') and hardlink(outside_old, 'old_owner')
            and not (old[0] & 2) and old[1:4] == [plan['old_owner_allocated'], plan['old_owner_size'], plan['old_owner_mtime'] // 10**9]
            and new_owner[1:4] == [plan['outside_owner_allocated'], plan['outside_owner_size'], plan['outside_owner_mtime'] // 10**9]
            and new_owner[5] is None):
        raise ValueError('fixture owner facts differ from expected')
    if phase == 'seed' and not (outside_old[0] & 2 and outside_old[1:3] == [0, 0] and new_owner[4] == 1):
        raise ValueError('seed owner or alias differs from expected')
    if phase == 'refresh':
        new_link = facts[plan['new_link_path']]
        if not (hardlink(new_link, 'outside_owner') and new_link[0] & 2 and new_link[1:3] == [0, 0]
                and new_link[3] == plan['outside_owner_mtime'] // 10**9 and new_link[4] == 2):
            raise ValueError('new link is not the expected zeroed alias')
    return facts


def normalize(table, columns, row, cutoff):
    result = []
    for name, value in zip(columns, row):
        if table == "meta" and name in ("snapshot_id", "scan_seconds", "freespace_version"):
            value = "generated"
        elif table in ("roots", "mounts") and name in ("fs_used", "fs_total", "fs_avail", "inodes_used"):
            value = "filesystem observation"
        elif name in ("scanned_at", "min_scanned_at"):
            if not isinstance(value, int):
                raise ValueError("timestamp must be an integer")
            if value > cutoff:
                value = "new observation"
        if isinstance(value, bytes):
            value = {"bytes": value.hex()}
        result.append(value)
    return result


class EntryValidator:
    """Validate preorder intervals and aggregate extrema using O(depth) state."""
    def __init__(self, columns):
        self.index = {name: columns.index(name) for name in (
            "id", "parent_id", "kind", "items", "subtree_end", "scanned_at",
            "min_scanned_at", "mtime", "newest_mtime")}
        self.stack = []
        self.next_id = 0

    def close(self, last_id):
        frame = self.stack.pop()
        if frame["end"] != last_id or frame["items"] != frame["end"] - frame["id"]:
            raise ValueError("invalid subtree interval or items")
        if frame["stored_min"] != frame["minimum"]:
            raise ValueError("invalid freshness minimum")
        newest = frame["newest"]
        if frame["stored_newest"] != (0 if newest is None or newest == -(1 << 63) else newest):
            raise ValueError("invalid newest file mtime")
        if self.stack:
            parent = self.stack[-1]
            parent["minimum"] = min(parent["minimum"], frame["minimum"])
            if newest is not None:
                parent["newest"] = newest if parent["newest"] is None else max(parent["newest"], newest)

    def add(self, row):
        v = {name: row[index] for name, index in self.index.items()}
        ident = v["id"]
        if ident != self.next_id:
            raise ValueError("entry ids are not dense")
        while self.stack and self.stack[-1]["end"] < ident:
            self.close(ident - 1)
        if v["parent_id"] != (self.stack[-1]["id"] if self.stack else None):
            raise ValueError("invalid parent or subtree interval")
        if v["kind"] not in (0, 1, 2, 3, 4) or v["subtree_end"] < ident:
            raise ValueError("invalid entry kind or subtree interval")
        if v["kind"] != 1 and v["subtree_end"] != ident:
            raise ValueError("non-directory has descendants")
        self.stack.append(dict(id=ident, end=v["subtree_end"], items=v["items"],
                               stored_min=v["min_scanned_at"], stored_newest=v["newest_mtime"],
                               minimum=v["scanned_at"],
                               newest=min(v["mtime"], v["scanned_at"]) if v["kind"] == 0 else None))
        self.next_id += 1

    def finish(self):
        while self.stack:
            self.close(self.next_id - 1)


def observe_database(path, cutoff, deadline):
    wal = path.with_name(path.name + "-wal")
    if wal.exists() and wal.stat().st_size:
        raise ValueError("nonempty WAL cannot be observed with immutable SQLite")
    with closing(sqlite3.connect(path.as_uri() + "?mode=ro&immutable=1", uri=True)) as db:
        db.set_progress_handler(lambda: int(time.monotonic() > deadline), 10000)
        if db.execute("PRAGMA quick_check").fetchone() != ("ok",):
            raise ValueError("SQLite quick_check failed")
        entries = db.execute("SELECT count(*) FROM entries").fetchone()[0]
        if db.execute("SELECT count(*) FROM entries WHERE min_scanned_at > scanned_at").fetchone()[0]:
            raise ValueError("invalid freshness ordering")
        digest = hashlib.sha256()
        tables = sorted(row[0] for row in db.execute("SELECT name FROM sqlite_schema WHERE type='table'"))
        for table in tables:
            if not table.replace("_", "").isalnum():
                raise ValueError("unexpected table name")
            columns = [row[1] for row in db.execute(f'PRAGMA table_info("{table}")')]
            digest.update(json.dumps([table, columns], separators=(",", ":")).encode())
            primary = "id" if table == "entries" else "entry_id" if "entry_id" in columns else columns[0]
            order = [primary] + [name for name in columns if name != primary]
            ordering = ",".join('"' + name.replace('"', '""') + '"' for name in order)
            validator = EntryValidator(columns) if table == "entries" else None
            for row in db.execute(f'SELECT * FROM "{table}" ORDER BY {ordering}'):
                if validator:
                    validator.add(row)
                digest.update(json.dumps([table, normalize(table, columns, row, cutoff)], separators=(",", ":")).encode())
                digest.update(b"\n")
            if validator:
                validator.finish()
        return entries, digest.hexdigest()


def inside(mode, spec_path):
    helper = legacy()
    spec = json.loads(Path(spec_path).read_text())
    if mode == "worker":
        return helper.measured_command(spec)
    output = Path("/results") / spec["id"] / "observer.json"
    snapshot = Path("/scratch/snapshot")
    group = helper.cgroup()
    before = helper.cg_read(group)
    started = time.monotonic()
    result = dict(validated=False, errors={}, seed_sha256=spec["seed_sha256"])
    try:
        integrity = helper.snapshot_integrity(snapshot)
        current_entries, current = observe_database(snapshot / "snapshot.db", spec["seed_timestamp"],
                                                    started + spec["timeout_seconds"])
        previous_entries, previous = observe_database(snapshot / "snapshot.prev.db", spec["seed_timestamp"],
                                                       started + spec["timeout_seconds"])
        result.update(current_entries=current_entries, previous_entries=previous_entries,
                      semantic_sha256=hashlib.sha256((current + previous).encode()).hexdigest(),
                      logical_content_unchanged=integrity == helper.snapshot_integrity(snapshot))
        if spec.get('scenario') == 'changed-full':
            plan = spec['mutation_plan']
            deadline = started + spec['timeout_seconds']
            result['full_link_facts'] = dict(current=full_link_facts(snapshot / 'snapshot.db', plan, 'refresh', deadline),
                                             previous=full_link_facts(snapshot / 'snapshot.prev.db', plan, 'seed', deadline))
            result['previous_sha256'] = integrity['snapshot.prev.db']['sha256']
        result["validated"] = result["logical_content_unchanged"] and current_entries == spec["expected_entries"]
    except Exception as error:
        result["errors"]["observer"] = repr(error)
    finally:
        result["resources"] = dict(elapsed_seconds=time.monotonic() - started,
                                   cgroup_before=before, cgroup=helper.cg_read(group))
        helper.dump(output, result)
    return 0 if result["validated"] else 1


def container_memory(spec, mode):
    return spec.get("observer_memory_mib", spec["memory_mib"]) if mode == "observer" else spec["memory_mib"]


def container(image, binary, case, tree, output, spec_path, spec, mode, label):
    """One fresh cgroup; bind mounts keep fixture and observed snapshots read-only."""
    helper = legacy()
    name = "fs-v1-pair-" + uuid.uuid4().hex
    readonly = ",readonly" if mode == "observer" else ""
    memory = container_memory(spec, mode)
    command = ["docker", "run", "--name", name, "--label", label, "--hostname", "freespace-benchmark",
               "--network", "none", "--cpus", str(spec["cpus"]), "--memory", f'{memory}m',
               "--memory-swap", f'{memory}m', "--cgroupns", "private", "--pids-limit", "128",
               "--user", f"{os.getuid()}:{os.getgid()}", "--entrypoint", "python3",
               "--mount", f"type=bind,src={case},dst=/scratch{readonly}",
               "--mount", f"type=bind,src={tree},dst=/scratch/tree,readonly",
               "--mount", f"type=bind,src={output},dst=/results",
               "--mount", f"type=bind,src={Path(__file__).parent},dst=/harness,readonly",
               "--mount", f"type=bind,src={binary},dst=/binary,readonly"]
    for flag, key in (("read-bps", "rbps"), ("write-bps", "wbps"), ("read-iops", "riops"), ("write-iops", "wiops")):
        if key in spec.get("io_limits", {}):
            command += ["--device-" + flag, f'{spec["io_device"]}:{spec["io_limits"][key]}']
    if mode == "worker" and spec.get("allocation_diagnostics") is True:
        command += ["--env", f'FREESPACE_MEMORY_PROFILE=/results/{spec["id"]}/allocation-profile.jsonl']
    command += [image, "/harness/bench_v1_runtime.py", mode, "/results/" + spec_path.name]
    result_path = output / spec["id"] / (mode + "-state.json")
    # writer-state is the public receipt name used by the pair validator.
    if mode == "worker":
        result_path = output / spec["id"] / "writer-state.json"
    try:
        with (output / spec["id"] / (mode + ".log")).open("w") as log:
            run = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT,
                                 timeout=spec["timeout_seconds"] + 180)
        state = json.loads(helper.capture(["docker", "inspect", name]))[0]["State"]
        helper.dump(result_path, state)
        if run.returncode or state["OOMKilled"]:
            raise RuntimeError(f"{mode} failed: {spec['id']}")
    finally:
        subprocess.run(["docker", "rm", "-f", name], check=True, timeout=30,
                       stdout=subprocess.DEVNULL)


if __name__ == "__main__":
    import sys
    raise SystemExit(inside(sys.argv[1], sys.argv[2]))
