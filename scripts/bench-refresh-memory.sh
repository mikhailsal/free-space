#!/usr/bin/env bash
# Unprivileged synthetic scan/refresh scaling benchmark. See docs/benchmarks.md.
set -euo pipefail
if [[ ${1:-} == --help ]]; then
  cat <<'HELP'
Usage: [STEPS="100 1000"] [WORKERS=2] [REPEATS=1] [SMALL_FILES=16]
       [FILES_PER_DIR=1000] [TMPDIR=/scratch] scripts/bench-refresh-memory.sh BINARY OUTPUT_PARENT
Creates a unique results directory in OUTPUT_PARENT and a disposable tree in TMPDIR.
STEPS are strictly increasing counts of unrelated empty files (not total entries).
Requires Bash, Python 3, and GNU /usr/bin/time. No root or cache dropping required.
HELP
  exit 0
fi
if [[ $# != 2 ]]; then
  echo 'expected BINARY OUTPUT_PARENT (see --help)' >&2
  exit 2
fi
command -v python3 >/dev/null
/usr/bin/time --version | head -n 1 | python3 -c 'import sys; assert "GNU" in sys.stdin.read(), "GNU time required"'
# Python handles argument validation, deterministic generation, and JSON serialization.
exec python3 - "$1" "$2" <<'PY'
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

FIXED_MTIME = 1_700_000_000


def positive(name, default):
    value = int(os.environ.get(name, default))
    if value < 1:
        raise ValueError(f"{name} must be positive")
    return value


def binary_digest(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def disk_usage(path):
    # Endpoint occupancy, including retained generations; not peak temporary space.
    files = [p for p in path.rglob("*") if p.is_file()]
    return {"snapshot_file_bytes": sum(p.stat().st_size for p in files),
            "snapshot_allocated_bytes": sum(p.stat().st_blocks * 512 for p in files)}


def generate_files(parent, start, stop, per_dir):
    for index in range(start, stop):
        directory = parent / f"d{index // per_dir:08d}"
        directory.mkdir(exist_ok=True)
        file = directory / f"f{index:012d}"
        file.touch(exist_ok=False)
        os.utime(file, (FIXED_MTIME, FIXED_MTIME))
    # Stable directory times independent of generation speed.
    for directory in parent.iterdir():
        os.utime(directory, (FIXED_MTIME, FIXED_MTIME))
    os.utime(parent, (FIXED_MTIME, FIXED_MTIME))


def main():
    workers = positive("WORKERS", "2")
    repeats = positive("REPEATS", "1")
    small_files = positive("SMALL_FILES", "16")
    per_dir = positive("FILES_PER_DIR", "1000")
    steps = [int(value) for value in os.environ.get("STEPS", "100 1000").split()]
    if not steps or steps[0] < 1 or any(b <= a for a, b in zip(steps, steps[1:])):
        raise ValueError("STEPS must be strictly increasing positive integers")
    binary = Path(shutil.which(sys.argv[1]) or sys.argv[1]).resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError(f"not an executable binary: {binary}")
    parent = Path(sys.argv[2]).resolve()
    parent.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix="refresh-memory-", dir=parent))
    print(f"Results: {output}", flush=True)
    # Both locations are newly created. Only this scratch directory is removed.
    with tempfile.TemporaryDirectory(prefix="freespace-scaling-") as scratch_name:
        scratch = Path(scratch_name).resolve()
        tree, snapshot = scratch / "tree", scratch / "snapshot"
        small, unrelated = tree / "small", tree / "unrelated"
        small.mkdir(parents=True)
        unrelated.mkdir()
        snapshot.mkdir()
        generate_files(small, 0, small_files, per_dir)
        config = scratch / "config.toml"
        config.write_text("keep_previous = true\n")
        env = dict(os.environ, LC_ALL="C", FREESPACE_SNAPSHOT_DIR=str(snapshot), FREESPACE_CONFIG=str(config))
        version = subprocess.run([str(binary), "--version"], capture_output=True, text=True, check=True)
        manifest = {
            "schema_version": 1, "started_utc": datetime.now(timezone.utc).isoformat(),
            "binary": str(binary), "binary_sha256": binary_digest(binary),
            "binary_version": version.stdout.strip(), "platform": platform.platform(),
            "python": platform.python_version(),
            "gnu_time_version": subprocess.check_output(["/usr/bin/time", "--version"], text=True).splitlines()[0],
            "workers": workers, "repeats": repeats,
            "unrelated_file_steps": steps, "small_files": small_files, "files_per_dir": per_dir,
            "file_payload_bytes": 0, "tree": str(tree), "snapshot_dir": str(snapshot),
            "cache_policy": "uncontrolled; freshly generated metadata; refresh follows scan",
            "snapshot_policy": "reused across steps and repeats; keep_previous=true",
            "config_toml": config.read_text(),
            "environment": {key: env[key] for key in ("LC_ALL", "FREESPACE_CONFIG", "FREESPACE_SNAPSHOT_DIR")},
        }
        if Path("/proc/meminfo").exists():
            manifest["meminfo_before"] = Path("/proc/meminfo").read_text()
        fs = subprocess.run(["stat", "-f", "-c", "%T", str(tree)], capture_output=True, text=True)
        manifest["filesystem_type"] = fs.stdout.strip() if fs.returncode == 0 else None
        write_json(output / "manifest.json", manifest)
        summary = output / "results.jsonl"
        previous = 0
        for step in steps:
            generate_files(unrelated, previous, step, per_dir)
            previous = step
            os.utime(tree, (FIXED_MTIME, FIXED_MTIME))
            expected_entries = 3 + small_files + (small_files + per_dir - 1) // per_dir + step + (step + per_dir - 1) // per_dir
            for repeat in range(1, repeats + 1):
                for operation, target in (("scan", tree), ("refresh", small)):
                    prefix = f"files-{step}-r{repeat}-{operation}"
                    report_path = output / f"{prefix}.json"
                    time_path = output / f"{prefix}.time"
                    stderr_path = output / f"{prefix}.stderr"
                    command = [str(binary), operation, str(target), "--workers", str(workers), "--json"]
                    time_format = '{"wall_seconds":%e,"peak_rss_kib":%M,"fs_inputs":%I,"fs_outputs":%O,"exit_code":%x}'
                    with report_path.open("w") as stdout, stderr_path.open("w") as stderr:
                        result = subprocess.run(["/usr/bin/time", "-f", time_format, "-o", str(time_path), *command],
                                                env=env, stdout=stdout, stderr=stderr)
                    # GNU time may prepend a failure message; its formatted row is last.
                    metrics = json.loads(time_path.read_text().splitlines()[-1])
                    report = None
                    validation_error = None
                    try:
                        report = json.loads(report_path.read_text())
                        if report["entries"] != expected_entries or report.get("incomplete", False) or report.get("errors", 0):
                            raise ValueError("unexpected entry count or incomplete scan")
                        expected_small = 1 + small_files + (small_files + per_dir - 1) // per_dir
                        if operation == "refresh" and report["subtree_entries"] != expected_small:
                            raise ValueError("unexpected refreshed subtree size")
                    except (ValueError, KeyError) as error:
                        validation_error = str(error)
                    row = {
                        "schema_version": 1, "operation": operation, "unrelated_files": step,
                        "expected_entries": expected_entries, "repeat": repeat, "workers": workers,
                        "command": command, "time": metrics, "report": report,
                        "validation_error": validation_error, "process_exit_code": result.returncode,
                        **disk_usage(snapshot),
                    }
                    with summary.open("a") as stream:
                        stream.write(json.dumps(row) + "\n")
                    print(f"{prefix}: exit={result.returncode}, peak_rss_kib={metrics['peak_rss_kib']}", flush=True)
                    if result.returncode or validation_error:
                        raise RuntimeError(f"{prefix} failed; inspect {stderr_path} and {report_path}: {validation_error}")
        manifest["finished_utc"] = datetime.now(timezone.utc).isoformat()
        write_json(output / "manifest.json", manifest)


try:
    main()
except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
    print(f"benchmark: {error}", file=sys.stderr)
    sys.exit(1)
PY
