#!/usr/bin/env python3
"""Verify the runner's disk envelope. Never calibrates or loads the local host.

Limits are an explicit sensitivity model, not measured local-device capacity.
Direct I/O probes validate enforcement before application timings are accepted.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import uuid


def capture(command):
    return subprocess.check_output(command, text=True, timeout=120)


def run(args):
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("disk calibration requires an ephemeral GitHub-hosted runner")
    output = Path(args.output)
    device_stat = os.stat(args.device)
    device_number = f"{os.major(device_stat.st_rdev)}:{os.minor(device_stat.st_rdev)}"
    limits = dict(rbps=33554432, wbps=16777216, riops=1000, wiops=500) if args.profile == "limited" else {}
    name = "freespace-calibration-" + uuid.uuid4().hex
    command = ["docker", "run", "-d", "--name", name, "--network", "none",
               "--cpus", "2", "--memory", "4096m", "--memory-swap", "4096m",
               "--cgroupns", "private", "--mount", f"type=bind,src={args.directory},dst=/disk",
               "--entrypoint", "sleep"]
    for flag, key in (("read-bps", "rbps"), ("write-bps", "wbps"), ("read-iops", "riops"), ("write-iops", "wiops")):
        if key in limits:
            command += ["--device-" + flag, f"{args.device}:{limits[key]}"]
    command += [args.image, "infinity"]
    report = dict(profile=args.profile, device=device_number, requested_limits=limits,
                  scope="runner-only direct I/O; synthetic envelope, not local calibration", jobs=[], success=False)
    try:
        capture(command)
        report["io_max"] = capture(["docker", "exec", name, "cat", "/sys/fs/cgroup/io.max"])
        installed = {line.split()[0]: dict(item.split("=") for item in line.split()[1:])
                     for line in report["io_max"].splitlines()}
        if any(installed.get(device_number, {}).get(key) != str(value) for key, value in limits.items()):
            raise RuntimeError("Docker did not install requested device limits")
        report["io_stat_before"] = capture(["docker", "exec", name, "cat", "/sys/fs/cgroup/io.stat"])
        for mode, block, direction, metric, bound in (
            ("write", "1m", "write", "bw_bytes", limits.get("wbps")),
            ("read", "1m", "read", "bw_bytes", limits.get("rbps")),
            ("randwrite", "4k", "write", "iops", limits.get("wiops")),
            ("randread", "4k", "read", "iops", limits.get("riops")),
        ):
            raw = capture(["docker", "exec", name, "fio", "--name=" + mode,
                           "--filename=/disk/calibration.bin", "--size=64m", "--direct=1",
                           "--ioengine=psync", "--iodepth=1", "--rw=" + mode, "--bs=" + block,
                           "--runtime=10", "--time_based=1", "--end_fsync=1", "--output-format=json"])
            (output / ("fio-" + mode + ".json")).write_text(raw)
            result = json.loads(raw)
            job = result["jobs"][0]
            value = job[direction][metric]
            report["jobs"].append(dict(mode=mode, metric=metric, observed=value, limit=bound))
            if job["error"] or (bound and value > bound * 1.25):
                raise RuntimeError(f"disk envelope probe failed: {mode} {metric}={value}, bound={bound}")
        report["io_stat_after"] = capture(["docker", "exec", name, "cat", "/sys/fs/cgroup/io.stat"])
        matched = [line for line in report["io_stat_after"].splitlines() if line.startswith(device_number + " ")]
        if limits and (not matched or not any(int(v.split("=")[1]) > 0 for v in matched[0].split()[1:])):
            raise RuntimeError("requested device has no observed I/O; cannot validate throttling")
        report["success"] = True
    except Exception as error:
        report["error"] = repr(error)
        raise
    finally:
        (output / "disk-calibration.json").write_text(json.dumps(report, indent=2) + "\n")
        subprocess.run(["docker", "rm", "-f", name], check=False, timeout=30)
        (Path(args.directory) / "calibration.bin").unlink(missing_ok=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("image", "device", "directory", "output"):
        parser.add_argument("--" + option, required=True)
    parser.add_argument("--profile", choices=("native", "limited"), required=True)
    run(parser.parse_args())
