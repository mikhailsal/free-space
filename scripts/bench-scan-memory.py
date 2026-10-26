#!/usr/bin/env python3
"""Compare full-scan engines, optionally inside a fresh no-swap systemd user cgroup.

Usage: [STEPS="10000 100000 1000000"] [MEMORY_MAX=128M] [REPEATS=1]
       [FILES_PER_DIR=1000] [TMPDIR=/scratch] scripts/bench-scan-memory.py BINARY OUTPUT_PARENT

MEMORY_MAX requires a systemd user manager with cgroup memory delegation. The
runner samples process RSS and snapshot-directory occupancy every 20 ms. Peak RSS
also comes from wait4 rusage. The scanner and measurement helper share the cgroup;
its memory.peak (when available), or sampled memory.current, includes the helper,
filesystem cache and kernel memory. SOURCE_REVISION records the built source hash.
"""

import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import shutil
import subprocess
import sys
import tempfile
import time


def write_json(path, data):
    path.write_text(json.dumps(data, indent=2) + "\n")


def positive(name, default):
    value = int(os.environ.get(name, default))
    if value < 1:
        raise ValueError(f"{name} must be positive")
    return value


def cgroup_folder():
    for line in Path("/proc/self/cgroup").read_text().splitlines():
        if line.startswith("0::"):
            return Path("/sys/fs/cgroup") / line[3:].lstrip("/")
    return None


def cgroup_metrics():
    for line in Path("/proc/self/cgroup").read_text().splitlines():
        if line.startswith("0::"):
            folder = Path("/sys/fs/cgroup") / line[3:].lstrip("/")
            result = {}
            for name in ("memory.peak", "memory.swap.peak", "memory.events", "memory.max", "memory.swap.max", "memory.pressure"):
                file = folder / name
                if file.exists():
                    result[name] = file.read_text().strip()
            return result
    return {}


def sample(pid, snapshot):
    rss = 0
    try:
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                rss = int(line.split()[1])
    except FileNotFoundError:
        pass
    allocated = 0
    for file in snapshot.iterdir():
        try:
            allocated += file.stat().st_blocks * 512
        except FileNotFoundError:
            pass
    return rss, allocated


def measured_run(spec_path):
    spec = json.loads(spec_path.read_text())
    prefix = Path(spec["prefix"])
    snapshot = Path(spec["snapshot"])
    started = time.monotonic()
    rss_peak, disk_peak, group_peak, swap_peak = 0, 0, 0, 0
    group = cgroup_folder() if spec["constrained"] else None
    with prefix.with_suffix(".stdout").open("w") as stdout, prefix.with_suffix(".stderr").open("w") as stderr:
        proc = subprocess.Popen(spec["command"], env=dict(os.environ, **spec["env"]), stdout=stdout, stderr=stderr)
        while proc.poll() is None:
            rss, disk = sample(proc.pid, snapshot)
            rss_peak, disk_peak = max(rss_peak, rss), max(disk_peak, disk)
            if group:
                group_peak = max(group_peak, int((group / "memory.current").read_text()))
                swap_peak = max(swap_peak, int((group / "memory.swap.current").read_text()))
            time.sleep(0.02)
    _, disk = sample(proc.pid, snapshot)
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    report = None
    if proc.returncode == 0:
        report = json.loads(prefix.with_suffix(".stdout").read_text())
        if report["entries"] != spec["expected_entries"] or report["errors"] or report["incomplete"]:
            raise RuntimeError("scan report does not match generated fixture")
    row = {"command": spec["command"], "entries": spec["expected_entries"], "exit_code": proc.returncode,
           "wall_seconds": time.monotonic() - started, "peak_rss_kib": usage.ru_maxrss,
           "sampled_peak_rss_kib": rss_peak, "sampled_peak_snapshot_disk_bytes": max(disk_peak, disk),
           "report": report, "cgroup": cgroup_metrics() if spec["constrained"] else None,
           "sampled_peak_cgroup_bytes": group_peak if group else None,
           "sampled_peak_swap_bytes": swap_peak if group else None}
    write_json(prefix.with_suffix(".metrics.json"), row)


def generate(root, start, stop, per_dir):
    for index in range(start, stop):
        folder = root / f"d{index // per_dir:08}"
        folder.mkdir(exist_ok=True)
        (folder / f"f{index:012}").touch(exist_ok=False)


def run_case(binary, output, tree, snapshot, count, mode, repeat, limit):
    prefix = output / f"{count}-{mode}-{repeat}"
    env = dict(FREESPACE_CONFIG=str(output / "config.toml"), FREESPACE_SNAPSHOT_DIR=str(snapshot), LC_ALL="C")
    command = [str(binary), "scan", str(tree), "--workers", "1", "--json"]
    if mode == "disk":
        command.append("--low-memory")
    spec = {"prefix": str(prefix), "snapshot": str(snapshot), "command": command, "env": env,
            "expected_entries": count, "constrained": bool(limit)}
    # Do not persist the inherited environment: it may contain secrets.
    spec_path = output / f"{count}-{mode}-{repeat}.run.json"
    write_json(spec_path, spec)
    invocation = [sys.executable, str(Path(__file__).resolve()), "--run", str(spec_path)]
    if limit:
        invocation = ["systemd-run", "--user", "--wait", "--pipe", "--quiet",
                      "--property=MemoryMax=" + limit, "--property=MemorySwapMax=0",
                      "--property=OOMPolicy=continue", *invocation]
    try:
        subprocess.run(invocation, check=True)
    finally:
        spec_path.unlink(missing_ok=True)
    row = json.loads(prefix.with_suffix(".metrics.json").read_text())
    row.update(mode=mode, repeat=repeat)
    with (output / "results.jsonl").open("a") as stream:
        stream.write(json.dumps(row) + "\n")
    print(f"{mode}: {count} entries, exit {row['exit_code']}, {row['peak_rss_kib']} KiB, {row['wall_seconds']:.2f}s", flush=True)
    if mode == "disk" and row["exit_code"]:
        raise RuntimeError("disk-backed scan failed; inspect retained artifacts")


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--run":
        measured_run(Path(sys.argv[2]))
        return
    if len(sys.argv) != 3 or sys.argv[1] == "--help":
        print(__doc__)
        raise SystemExit(0 if "--help" in sys.argv else 2)
    steps = [int(v) for v in os.environ.get("STEPS", "10000 100000 1000000").split()]
    if not steps or steps[0] < 1 or any(b <= a for a, b in zip(steps, steps[1:])):
        raise ValueError("STEPS must be strictly increasing positive file counts")
    repeats, per_dir = positive("REPEATS", "1"), positive("FILES_PER_DIR", "1000")
    limit = os.environ.get("MEMORY_MAX")
    binary = Path(shutil.which(sys.argv[1]) or sys.argv[1]).resolve()
    parent = Path(sys.argv[2]).resolve()
    parent.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix="scan-memory-", dir=parent))
    (output / "config.toml").write_text("keep_previous = false\n")
    print(f"Results: {output}", flush=True)
    write_json(output / "manifest.json", {"binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "source_revision": os.environ.get("SOURCE_REVISION"),
        "platform": platform.platform(), "memory_max": limit, "swap_max": "0" if limit else None,
        "steps": steps, "repeats": repeats, "files_per_dir": per_dir,
        "meminfo": Path("/proc/meminfo").read_text(), "cache_policy": "uncontrolled, freshly generated metadata"})
    with tempfile.TemporaryDirectory(prefix="freespace-scan-memory-") as scratch:
        tree = Path(scratch) / "tree"
        tree.mkdir()
        previous = 0
        for step in steps:
            generate(tree, previous, step, per_dir)
            previous = step
            count = 1 + step + (step + per_dir - 1) // per_dir
            for repeat in range(1, repeats + 1):
                for mode in ("fast", "disk"):
                    snapshot = Path(scratch) / f"snapshot-{step}-{repeat}-{mode}"
                    snapshot.mkdir()
                    run_case(binary, output, tree, snapshot, count, mode, repeat, limit)
                    shutil.rmtree(snapshot)


if __name__ == "__main__":
    main()
