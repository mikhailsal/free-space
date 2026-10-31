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


def seed_hash(snapshot):
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
    return hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()


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
            for row in db.execute(f'SELECT * FROM "{table}" ORDER BY {ordering}'):
                digest.update(json.dumps([table, normalize(table, columns, row, cutoff)], separators=(",", ":")).encode())
                digest.update(b"\n")
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
        result["validated"] = result["logical_content_unchanged"] and current_entries == spec["expected_entries"]
    except Exception as error:
        result["errors"]["observer"] = repr(error)
    finally:
        result["resources"] = dict(elapsed_seconds=time.monotonic() - started,
                                   cgroup_before=before, cgroup=helper.cg_read(group))
        helper.dump(output, result)
    return 0 if result["validated"] else 1


def container(image, binary, case, tree, output, spec_path, spec, mode, label):
    """One fresh cgroup; bind mounts keep fixture and observed snapshots read-only."""
    helper = legacy()
    name = "fs-v1-pair-" + uuid.uuid4().hex
    readonly = ",readonly" if mode == "observer" else ""
    command = ["docker", "run", "--name", name, "--label", label, "--hostname", "freespace-benchmark",
               "--network", "none", "--cpus", str(spec["cpus"]), "--memory", f'{spec["memory_mib"]}m',
               "--memory-swap", f'{spec["memory_mib"]}m', "--cgroupns", "private", "--pids-limit", "128",
               "--user", f"{os.getuid()}:{os.getgid()}", "--entrypoint", "python3",
               "--mount", f"type=bind,src={case},dst=/scratch{readonly}",
               "--mount", f"type=bind,src={tree},dst=/scratch/tree,readonly",
               "--mount", f"type=bind,src={output},dst=/results",
               "--mount", f"type=bind,src={Path(__file__).parent},dst=/harness,readonly",
               "--mount", f"type=bind,src={binary},dst=/binary,readonly"]
    for flag, key in (("read-bps", "rbps"), ("write-bps", "wbps"), ("read-iops", "riops"), ("write-iops", "wiops")):
        if key in spec.get("io_limits", {}):
            command += ["--device-" + flag, f'{spec["io_device"]}:{spec["io_limits"][key]}']
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
