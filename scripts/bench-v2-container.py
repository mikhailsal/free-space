#!/usr/bin/env python3
"""GitHub-runner Docker benchmark. Never use the loaded development host.

Host: --baseline BIN --candidate BIN --output DIR [--steps 100000,1000000]
      [--memory 128|256|512] [--repeats 2] [--include-v1]
Each comparison arm receives a fresh 2-CPU, no-swap cgroup; bind-mounted fixtures
and snapshots bypass overlayfs. On a dedicated ephemeral GitHub-hosted VM,
host fixture setup is synced and caches dropped before each arm container.
Peaks sampled at 20ms are lower bounds; cgroup memory.peak is arm-cumulative
for CLI/helper/observations, excluding host setup. RSS covers each CLI process.
"""

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid


def dump(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def capture(command, **kwargs):
    return subprocess.check_output(command, text=True, timeout=120, **kwargs).strip()


def occupancy(snapshot):
    result = {name: {"logical_bytes": 0, "allocated_bytes": 0}
              for name in ("db", "wal", "shm", "stage", "other", "total")}
    for file in snapshot.rglob("*"):
        try:
            stat = file.stat()
            if not file.is_file():
                continue
        except FileNotFoundError:
            continue
        name = file.name
        kind = ("stage" if ".stage.tmp" in name else "wal" if name.endswith("-wal")
                else "shm" if name.endswith("-shm") else "db" if name.endswith(".db") else "other")
        for key in (kind, "total"):
            result[key]["logical_bytes"] += stat.st_size
            result[key]["allocated_bytes"] += stat.st_blocks * 512
    return result


def file_occupancy(snapshot):
    result = {}
    for file in snapshot.rglob("*"):
        try:
            stat = file.stat()
            if file.is_file():
                result[str(file.relative_to(snapshot))] = dict(logical_bytes=stat.st_size,
                                                              allocated_bytes=stat.st_blocks * 512)
        except FileNotFoundError:
            pass
    return result


def snapshot_integrity(snapshot):
    """Hash named regular files in bounded chunks, outside command timing."""
    result = {}
    for file in sorted(snapshot.rglob("*")):
        if not file.is_file():
            continue
        before = file.stat()
        digest = hashlib.sha256()
        with file.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        after = file.stat()
        if before.st_size != after.st_size:
            raise RuntimeError(f"file size changed while hashing {file}")
        result[str(file.relative_to(snapshot))] = dict(logical_bytes=after.st_size,
                                                       sha256=digest.hexdigest())
    return result


def observer_comparison(before_integrity, after_integrity, before_allocation, after_allocation):
    # st_blocks can settle asynchronously after a writer exits. It remains a
    # measurement, not a substitute for logical/content mutation detection.
    return dict(integrity_before=before_integrity, integrity_after=after_integrity,
                files_before=before_allocation, files_after=after_allocation,
                logical_content_unchanged=before_integrity == after_integrity,
                files_unchanged=before_allocation == after_allocation,
                allocation_drift={name: dict(before=before_allocation.get(name),
                                             after=after_allocation.get(name))
                                  for name in sorted(before_allocation.keys() | after_allocation.keys())
                                  if before_allocation.get(name) != after_allocation.get(name)})


def cgroup():
    relative = next(line[3:] for line in Path("/proc/self/cgroup").read_text().splitlines()
                    if line.startswith("0::"))
    # Docker's private cgroup namespace exposes the container group at its root.
    candidates = [Path("/sys/fs/cgroup") / relative.lstrip("/"), Path("/sys/fs/cgroup")]
    return next(folder for folder in candidates if (folder / "memory.max").exists())


def cg_read(folder):
    return {name: (folder / name).read_text().strip() for name in (
        "memory.current", "memory.peak", "memory.max", "memory.events",
        "memory.swap.current", "memory.swap.peak", "memory.swap.max", "memory.pressure",
        "cpu.max", "cpu.stat") if (folder / name).exists()}


def measured_command(spec):
    output = Path("/results") / spec["id"]
    output.mkdir()
    group = cgroup()
    before = cg_read(group)
    expected_max = str(spec["memory_mib"] * 1024 * 1024)
    if before.get("memory.max") != expected_max or before.get("memory.swap.max") != "0":
        raise RuntimeError(f"container is not in the requested no-swap cgroup: {before}")
    quota, period = before.get("cpu.max", "max 1").split()
    if quota == "max" or int(quota) != 2 * int(period):
        raise RuntimeError("CPU quota must be exactly 2 CPUs")
    dump(output / "cgroup-start.json", dict(path=str(group), metrics=before))
    command = ["/binary", "--backend", spec["backend"], spec["operation"], spec["target"],
               "--workers", "1", "--json"]
    if spec["operation"] == "scan":
        command.append("--low-memory")
    env = dict(os.environ, LC_ALL="C", FREESPACE_CONFIG="/scratch/config.toml",
               FREESPACE_SNAPSHOT_DIR="/scratch/snapshot")
    peaks = occupancy(Path("/scratch/snapshot"))
    file_peaks = {}
    sampled_memory = 0
    stopped = threading.Event()
    timed_out = threading.Event()
    sampler_errors = []

    def sample():
        nonlocal sampled_memory
        try:
            while not stopped.is_set():
                current = occupancy(Path("/scratch/snapshot"))
                for name, values in file_occupancy(Path("/scratch/snapshot")).items():
                    prior = file_peaks.setdefault(name, dict(logical_bytes=0, allocated_bytes=0))
                    for key, value in values.items():
                        prior[key] = max(prior[key], value)
                for kind in peaks:
                    for key in peaks[kind]:
                        peaks[kind][key] = max(peaks[kind][key], current[kind][key])
                sampled_memory = max(sampled_memory, int((group / "memory.current").read_text()))
                stopped.wait(0.02)
        except Exception as error:
            sampler_errors.append(repr(error))

    print(f'command start {spec["id"]}', flush=True)
    sampler = threading.Thread(target=sample, name="allocation-sampler")
    sampler.start()
    try:
        with (output / "stdout.json").open("w") as out, (output / "stderr.log").open("w") as err:
            started = time.monotonic()
            proc = subprocess.Popen(["/usr/bin/time", "-f",
                '{"elapsed_seconds":%e,"peak_rss_kib":%M,"inputs":%I,"outputs":%O,"exit_code":%x}',
                "-o", str(output / "time.log"), *command], stdout=out, stderr=err,
                env=env, start_new_session=True)

            def expire():
                if proc.poll() is None:
                    timed_out.set()
                    try:
                        os.killpg(proc.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

            watchdog = threading.Timer(spec["timeout_seconds"], expire)
            watchdog.start()
            try:
                # POSIX Popen.wait(timeout=...) itself polls with sleeps. A
                # watchdog keeps the bound while blocking waitpid wakes on exit.
                proc.wait()
                finished = time.monotonic()
            finally:
                watchdog.cancel()
                watchdog.join()
    finally:
        stopped.set()
        sampler.join()
    elapsed = finished - started
    print(f'command end {spec["id"]}: exit={proc.returncode} elapsed={elapsed:.6f}s', flush=True)
    after = cg_read(group)
    endpoint = occupancy(Path("/scratch/snapshot"))
    for kind in peaks:
        for key in peaks[kind]:
            peaks[kind][key] = max(peaks[kind][key], endpoint[kind][key])
    for name, values in file_occupancy(Path("/scratch/snapshot")).items():
        prior = file_peaks.setdefault(name, dict(logical_bytes=0, allocated_bytes=0))
        for key, value in values.items():
            prior[key] = max(prior[key], value)
    row = dict(spec, command=command, exit_code=proc.returncode, timed_out=timed_out.is_set(),
               sampler_errors=sampler_errors, elapsed_seconds=elapsed, cgroup_path=str(group),
               cgroup_before=before, cgroup_after=after, sampled_memory_peak=sampled_memory,
               sampled_disk_peak=peaks, endpoint_disk=endpoint, sample_interval_seconds=0.02)
    row["memory_peak_scope"] = "arm cumulative CLI, helper and prior observations; excludes host fixture generation"
    row["sampled_memory_peak_scope"] = "current command sampled memory.current; lower bound"
    row["sampled_file_peaks"] = file_peaks
    row["endpoint_files"] = file_occupancy(Path("/scratch/snapshot"))
    try:
        row["time"] = json.loads((output / "time.log").read_text().splitlines()[-1])
        report = json.loads((output / "stdout.json").read_text())
        row["report"] = report
        if report["entries"] != spec["expected_entries"] or report.get("errors") or report.get("incomplete"):
            raise ValueError("incomplete or unexpected full entry count")
        if spec["operation"] == "refresh" and report["subtree_entries"] != 18:
            raise ValueError("refresh must rescan exactly 18 entries")
        row["validated"] = proc.returncode == 0 and not timed_out.is_set() and not sampler_errors
    except (OSError, ValueError, KeyError) as error:
        row["validation_error"] = str(error)
        row["validated"] = False
    dump(output / "metrics.json", row)
    return 0 if row["validated"] else 1


def db_breakdown(snapshot):
    """Read-only observation after CLI exit, outside timed span; cache is retained."""
    result = {}
    for path in sorted(snapshot.rglob("*.db")):
        key = str(path.relative_to(snapshot))
        wal = path.with_name(path.name + "-wal")
        # immutable ignores WAL entirely and never performs sidecar cleanup.
        # It is safe here only after the closed CLI has emptied/removed WAL.
        if wal.exists() and wal.stat().st_size:
            result[key] = {"unavailable": "nonempty WAL: immutable observer would omit committed data",
                           "wal_logical_bytes": wal.stat().st_size}
            continue
        try:
            with closing(sqlite3.connect(path.as_uri() + "?mode=ro&immutable=1", uri=True, timeout=10)) as db:
                deadline = time.monotonic() + 120
                db.set_progress_handler(lambda: int(time.monotonic() > deadline), 10000)
                schema = db.execute("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name").fetchall()
                tables = [row[1] for row in schema if row[0] == "table"]
                counts = {name: db.execute('SELECT count(*) FROM "' + name.replace('"', '""') + '"').fetchone()[0]
                          for name in tables}
                result[key] = dict(schema=schema, row_counts=counts,
                    page_size=db.execute("PRAGMA page_size").fetchone()[0],
                    page_count=db.execute("PRAGMA page_count").fetchone()[0],
                    freelist_count=db.execute("PRAGMA freelist_count").fetchone()[0],
                    dbstat_columns=["name", "pages", "bytes", "payload", "unused"])
                try:
                    result[key]["dbstat"] = db.execute("SELECT name,count(*),sum(pgsize),sum(payload),sum(unused) FROM dbstat GROUP BY name ORDER BY name").fetchall()
                except sqlite3.Error as error:
                    result[key]["dbstat_error"] = str(error)
        except sqlite3.Error as error:
            result[key] = {"error": str(error)}
    return result


def generate(root, start, stop, checkpoint):
    """Create deterministic fixtures with constant Python state and bounded batches.

    Host preparation happens outside the measured container. Sync drains dirty
    metadata before the dedicated VM cache flush; no setup memory is CLI RAM.
    """
    fixed_time = (1700000000, 1700000000)
    checkpoint(root.name, start, "start")
    for index in range(start, stop):
        if index == start or index % 1000 == 0:
            directory = root / f"d{index // 1000:06}"
            directory.mkdir(exist_ok=True)
        file = directory / f"f{index:09}"
        file.touch(exist_ok=False)
        os.utime(file, fixed_time)
        if (index + 1) % 1000 == 0 or index + 1 == stop:
            os.utime(directory, fixed_time)
        if (index + 1) % 10000 == 0:
            checkpoint(root.name, index + 1, "before_sync")
            os.sync()
            checkpoint(root.name, index + 1, "after_sync")
    os.utime(root, fixed_time)
    checkpoint(root.name, stop, "before_final_sync")
    os.sync()
    checkpoint(root.name, stop, "complete")


def prepare_fixture(case, output, ident, files):
    """Prepare outside the measured cgroup, then cold-start the entire arm."""
    def checkpoint(subtree, created, phase):
        row = dict(subtree=subtree, created_files=created, phase=phase,
                   unix_seconds=time.time(), scope="host setup outside measured container")
        with (output / (ident + ".fixture-progress.jsonl")).open("a") as stream:
            stream.write(json.dumps(row) + "\n")
            stream.flush()
            os.fsync(stream.fileno())
        print(f"host fixture {ident} {subtree} {created} {phase}", flush=True)

    tree = case / "tree"
    (tree / "small").mkdir(parents=True)
    (tree / "unrelated").mkdir()
    generate(tree / "small", 0, 16, checkpoint)
    generate(tree / "unrelated", 0, files, checkpoint)
    os.utime(tree, (1700000000, 1700000000))
    checkpoint("tree", files + 16, "before_final_sync")
    os.sync()
    checkpoint("tree", files + 16, "complete")
    command = ["sudo", "-n", "tee", "/proc/sys/vm/drop_caches"]
    receipt = dict(arm=ident, policy="sync then drop_caches=3 on this dedicated ephemeral GitHub-hosted VM only",
                   runner_environment=os.environ["RUNNER_ENVIRONMENT"],
                   sync_command="os.sync()", flush_command=command, flush_input="3\n",
                   started_unix_seconds=time.time(), success=False)
    receipt_path = output / (ident + ".cache-flush.json")
    dump(receipt_path, receipt)
    # The progress receipt was written after the final fixture sync. Flush it
    # too, with every fixture/progress file closed before dropping VM caches.
    os.sync()
    try:
        subprocess.run(command, input="3\n", text=True, stdout=subprocess.DEVNULL,
                       check=True, timeout=60)
        receipt["success"] = True
    except (OSError, subprocess.SubprocessError) as error:
        receipt["error"] = repr(error)
        raise
    finally:
        receipt["finished_unix_seconds"] = time.time()
        dump(receipt_path, receipt)


def inside(spec_path):
    spec = json.loads(spec_path.read_text())
    group = cgroup()
    dump(Path("/results") / (spec["id"] + ".arm-start.json"), cg_read(group))
    for index, operation in enumerate(spec["operations"]):
        command_spec = dict(spec, id=f'{spec["id"]}-{index}-{operation}', operation=operation,
                            target="/scratch/tree/small" if operation == "refresh" else "/scratch/tree")
        code = measured_command(command_spec)
        # SQLite uses a read-only connection and no checkpoint PRAGMA. This
        # observation can warm/evict cache; record it rather than reset cache.
        snapshot = Path("/scratch/snapshot")
        before_integrity = snapshot_integrity(snapshot)
        before_files = file_occupancy(snapshot)
        observation = dict(cache_policy="bounded full-file integrity reads then immutable observer then integrity reads; outside CLI timing, cache retained in arm cgroup",
                           cgroup_before=cg_read(group),
                           databases=db_breakdown(snapshot),
                           cgroup_after=cg_read(group))
        after_integrity = snapshot_integrity(snapshot)
        after_files = file_occupancy(snapshot)
        observation.update(observer_comparison(before_integrity, after_integrity, before_files, after_files))
        dump(Path("/results") / command_spec["id"] / "dbstat.json", observation)
        if not observation["logical_content_unchanged"]:
            raise RuntimeError("SQLite observation changed snapshot names, logical sizes or bytes")
        if code:
            return code
    final = cg_read(group)
    dump(Path("/results") / (spec["id"] + ".arm-end.json"), final)
    events = dict(line.split() for line in final["memory.events"].splitlines())
    return 1 if int(events.get("oom_kill", 0)) or int(events.get("oom", 0)) else 0


def host(args):
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("Setup, cache flushing and measurements require an ephemeral GitHub-hosted Actions VM")
    steps = [int(value) for value in args.steps.split(",")]
    if any(value not in (100000, 1000000) for value in steps) or sorted(set(steps)) != steps:
        raise ValueError("steps must be 100000 and/or 1000000 in increasing order")
    if args.repeats < 1 or args.scans < 2:
        raise ValueError("repeats >= 1 and scans >= 2 required")
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    harness = Path(__file__).resolve()
    binaries = {"baseline-v2": (Path(args.baseline).resolve(), "v2"),
                "candidate-v2": (Path(args.candidate).resolve(), "v2")}
    if args.include_v1:
        binaries["baseline-v1"] = (Path(args.baseline).resolve(), "v1")
    image = "freespace-balance:" + uuid.uuid4().hex
    subprocess.run(["docker", "build", "-t", image, str(harness.parent / "bench-v2-container")], check=True, timeout=600)
    failures = []
    completed = False
    manifest = dict(schema_version=1, arguments=vars(args), kernel=platform.release(),
        measurement_protocol="host-setup-cold-arm-integrity-v3", cache_flush_receipts=[],
        started_unix_seconds=time.time(),
        cpu=Path("/proc/cpuinfo").read_text(), docker=capture(["docker", "version"]),
        image=capture(["docker", "image", "inspect", image]),
        resource_policy="dedicated ephemeral GitHub-hosted VM; host setup excluded; each arm has 2 CPU quota, selected RAM MiB, memory-swap=memory, no OOM override",
        cache_policy="host-generated deterministic fixture, os.sync every 10000 files and at completion; os.sync then sudo -n tee /proc/sys/vm/drop_caches with input 3 before each arm on this job's ephemeral VM only; failure aborts, never warm fallback; clean OS input caches requested dropped (active pages and hardware caches not reset), then cache retained across CLI processes, full-file SHA256 reads before/after immutable post-command dbstat; integrity reads outside CLI timing warm/evict cache; alternating paired arm order",
        measurement_policy="complete CLI incl close/checkpoint; helper shares cgroup; memory.peak arm-cumulative CLI/helper/observations excluding host fixture generation; per-command sampled current lower bound and GNU time RSS; disk peaks sampled lower bounds",
        provenance={arm: dict(sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                     source_revision=binary.with_name(binary.name + ".revision.txt").read_text().strip(),
                     version=capture([str(binary), "--version"])) for arm, (binary, _) in binaries.items()},
        github={key: os.environ.get(key) for key in ("GITHUB_SHA", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "RUNNER_NAME", "RUNNER_OS", "RUNNER_ENVIRONMENT")})
    dump(output / "manifest.json", manifest)
    try:
        with tempfile.TemporaryDirectory(prefix="freespace-gh-balance-", dir=os.environ.get("RUNNER_TEMP")) as temporary:
            scratch = Path(temporary)
            manifest["filesystem"] = capture(["findmnt", "-T", str(scratch), "-J", "-o", "TARGET,SOURCE,FSTYPE,OPTIONS"])
            dump(output / "manifest.json", manifest)
            ordinal = 0
            for keep in (False, True):
                for repeat in range(args.repeats):
                    for step_index, count in enumerate(steps):
                        arms = list(binaries)
                        if (repeat + step_index + int(keep)) % 2:
                            arms.reverse()
                        for arm in arms:
                            # Host setup is excluded; each arm owns one container
                            # and cgroup for all scans, refresh and observation.
                            case = scratch / "case"
                            case.mkdir()
                            try:
                                (case / "snapshot").mkdir()
                                (case / "config.toml").write_text(f"keep_previous = {str(keep).lower()}\n")
                                ordinal += 1
                                ident = f"{ordinal:04}-{arm}-{count}-keep{int(keep)}-r{repeat + 1}"
                                binary, backend = binaries[arm]
                                operations = ["scan"] * args.scans + ["refresh"]
                                spec = dict(id=ident, arm=arm, backend=backend,
                                    expected_entries=count + (count + 999) // 1000 + 20,
                                    operations=operations, memory_mib=args.memory,
                                    keep_previous=keep, repeat=repeat + 1, files=count,
                                    timeout_seconds=args.timeout_seconds, order=ordinal)
                                spec_path = output / (ident + ".spec.json")
                                dump(spec_path, spec)
                                try:
                                    prepare_fixture(case, output, ident, count)
                                finally:
                                    receipt_path = output / (ident + ".cache-flush.json")
                                    if receipt_path.exists():
                                        manifest["cache_flush_receipts"].append(json.loads(receipt_path.read_text()))
                                        dump(output / "manifest.json", manifest)
                                name = "fs-balance-" + uuid.uuid4().hex
                                print(f"running {ident}: memory={args.memory}MiB CPUs=2 swap=0", flush=True)
                                command = ["docker", "run", "--name", name, "--network", "none", "--cpus", "2",
                                    "--memory", f"{args.memory}m", "--memory-swap", f"{args.memory}m",
                                    "--cgroupns", "private", "--pids-limit", "128", "--user", f"{os.getuid()}:{os.getgid()}",
                                    "--mount", f"type=bind,src={case},dst=/scratch",
                                    "--mount", f"type=bind,src={output},dst=/results",
                                    "--mount", f"type=bind,src={harness.parent},dst=/harness,readonly",
                                    "--mount", f"type=bind,src={binary},dst=/binary,readonly",
                                    image, "/results/" + spec_path.name]
                                try:
                                    with (output / (ident + ".docker.log")).open("w") as log:
                                        run = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT,
                                            timeout=300 + len(operations) * (args.timeout_seconds + 180))
                                    inspection = json.loads(capture(["docker", "inspect", name]))[0]
                                    dump(output / (ident + ".docker-state.json"), inspection["State"])
                                    valid = True
                                    for index, operation in enumerate(operations):
                                        metrics_path = output / f"{ident}-{index}-{operation}" / "metrics.json"
                                        metrics = json.loads(metrics_path.read_text()) if metrics_path.exists() else {}
                                        valid = valid and metrics.get("validated", False)
                                    if run.returncode or inspection["State"].get("OOMKilled") or not valid:
                                        failures.append(ident)
                                except subprocess.TimeoutExpired:
                                    failures.append(ident + ":docker-timeout")
                                finally:
                                    subprocess.run(["docker", "rm", "-f", name], stdout=subprocess.DEVNULL,
                                        stderr=subprocess.DEVNULL, timeout=30, check=False)
                            finally:
                                shutil.rmtree(case)
            completed = True
    finally:
        dump(output / "status.json", {"failures": failures, "completed": completed,
                                      "success": completed and not failures,
                                      "finished_unix_seconds": time.time()})
        subprocess.run(["docker", "image", "rm", image], stdout=subprocess.DEVNULL, timeout=60, check=False)
    return 1 if failures else 0


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--inside":
        return inside(Path(sys.argv[2]))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--steps", default="100000")
    parser.add_argument("--memory", type=int, choices=(128, 256, 512), default=256)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--scans", type=int, default=3)
    parser.add_argument("--timeout-seconds", type=int, default=900)
    parser.add_argument("--include-v1", action="store_true")
    return host(parser.parse_args())


if __name__ == "__main__":
    sys.exit(main())
