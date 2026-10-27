#!/usr/bin/env python3
"""GitHub-only immutable-observer correctness diagnostic; no performance claims.

python3 scripts/check-v2-observer.py --baseline binaries/baseline \
    --candidate binaries/candidate --output observer-results
Uses the benchmark's existing Docker helper image definition. Each tiny v2 store
is seeded separately, then observed in a separate container with an OS-enforced
read-only snapshot mount. No cache dropping or large benchmark rerun is needed.
"""

import argparse
import errno
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import uuid


SPEC = importlib.util.spec_from_file_location("v2_benchmark", Path(__file__).with_name("bench-v2-container.py"))
BENCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCH)


def regressions(output):
    """Run only on GH: real file hashes plus replay of recorded allocation drift."""
    checks = {}
    with tempfile.TemporaryDirectory(prefix="observer-regression-", dir=output) as temporary:
        root = Path(temporary)
        (root / "v2").mkdir()
        db = root / "v2" / "generations.db"
        db.write_bytes(b"same-size-before")
        (db.parent / "generations.db-shm").write_bytes(b"\0" * 32768)
        original = BENCH.snapshot_integrity(root)
        # Exact SHM occupancy transition from run 40418037469, independent of
        # content: logical 32768 unchanged, allocated 36864 -> 32768.
        before = {"v2/generations.db-shm": dict(logical_bytes=32768, allocated_bytes=36864)}
        after = {"v2/generations.db-shm": dict(logical_bytes=32768, allocated_bytes=32768)}
        replay = BENCH.observer_comparison(original, original, before, after)
        checks["real_shm_allocation_only_drift_accepted"] = replay["logical_content_unchanged"] and bool(replay["allocation_drift"])
        db.write_bytes(b"same-size-after!")
        assert db.stat().st_size == original["v2/generations.db"]["logical_bytes"]
        checks["same_size_byte_mutation_rejected"] = not BENCH.observer_comparison(
            original, BENCH.snapshot_integrity(root), {}, {})["logical_content_unchanged"]
        db.write_bytes(b"same-size-before")
        db.rename(db.parent / "renamed.db")
        checks["name_mutation_rejected"] = not BENCH.observer_comparison(
            original, BENCH.snapshot_integrity(root), {}, {})["logical_content_unchanged"]
        (db.parent / "renamed.db").rename(db)
        db.write_bytes(b"longer-than-before")
        checks["logical_size_mutation_rejected"] = not BENCH.observer_comparison(
            original, BENCH.snapshot_integrity(root), {}, {})["logical_content_unchanged"]
    BENCH.dump(output / "regressions.json", checks)
    if not all(checks.values()):
        raise RuntimeError(f"integrity regressions failed: {checks}")


def seed(spec):
    scratch = Path("/scratch")
    tree = scratch / "tree"
    child = tree / "small" / "d000000"
    child.mkdir(parents=True)
    for index in range(16):
        (child / f"f{index:09}").touch()
    snapshot = scratch / "snapshot"
    snapshot.mkdir()
    (scratch / "config.toml").write_text("keep_previous = true\n")
    env = dict(os.environ, FREESPACE_CONFIG=str(scratch / "config.toml"),
               FREESPACE_SNAPSHOT_DIR=str(snapshot), LC_ALL="C")
    result = subprocess.run(["/binary", "--backend", "v2", "scan", str(tree),
                             "--low-memory", "--workers", "1", "--json"],
                            capture_output=True, text=True, timeout=120, env=env)
    prefix = Path("/results") / spec["arm"]
    prefix.with_suffix(".seed.stdout.json").write_text(result.stdout)
    prefix.with_suffix(".seed.stderr.log").write_text(result.stderr)
    result.check_returncode()
    report = json.loads(result.stdout)
    if report["entries"] != 19 or report.get("errors") or report.get("incomplete"):
        raise RuntimeError("tiny seed report invalid")


def observe(spec):
    snapshot = Path("/snapshot")
    dbs = sorted(snapshot.rglob("*.db"))
    if not dbs:
        raise RuntimeError("seed did not produce a database")
    probes = {}
    for label, path, mode in (("create", snapshot / ".observer-must-not-create", "xb"),
                              ("write_existing", dbs[0], "r+b")):
        try:
            with path.open(mode):
                pass
        except OSError as error:
            probes[label] = dict(errno=error.errno, erofs=error.errno == errno.EROFS)
        else:
            raise RuntimeError(f"snapshot mount permits {label}")
    if not all(probe["erofs"] for probe in probes.values()):
        raise RuntimeError(f"mount probe must specifically return EROFS: {probes}")
    before = BENCH.snapshot_integrity(snapshot)
    allocation_before = BENCH.file_occupancy(snapshot)
    databases = BENCH.db_breakdown(snapshot)  # The actual benchmark observer.
    after = BENCH.snapshot_integrity(snapshot)
    result = BENCH.observer_comparison(before, after, allocation_before, BENCH.file_occupancy(snapshot))
    result.update(readonly_probes=probes, databases=databases, performance_claim=False)
    def useful(db):
        counts = db.get("row_counts", {})
        return (counts.get("identities") == 19
                and counts.get("live_rows", counts.get("live")) == 19
                and counts.get("version_rows", counts.get("versions")) == 19
                and bool(db.get("dbstat"))
                and not any(key in db for key in ("error", "dbstat_error", "unavailable")))
    result["valid"] = result["logical_content_unchanged"] and bool(databases) and all(
        useful(db) for db in databases.values())
    BENCH.dump(Path("/results") / (spec["arm"] + ".observer.json"), result)
    if not result["valid"]:
        raise RuntimeError("read-only observer diagnostic failed")


def host(args):
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("Diagnostic requires an ephemeral GitHub-hosted Actions VM")
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    regressions(output)
    harness = Path(__file__).resolve().parent
    image = "freespace-observer:" + uuid.uuid4().hex
    subprocess.run(["docker", "build", "-t", image, str(harness / "bench-v2-container")], check=True, timeout=600)
    status = dict(success=False, performance_claim=False)
    try:
        for arm, argument in (("baseline", args.baseline), ("candidate", args.candidate)):
            binary = Path(argument).resolve()
            BENCH.dump(output / (arm + ".provenance.json"), dict(
                source_revision=binary.with_name(binary.name + ".revision.txt").read_text().strip(),
                binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                version=BENCH.capture([str(binary), "--version"])))
            spec = dict(arm=arm)
            spec_path = output / (arm + ".spec.json")
            BENCH.dump(spec_path, spec)
            with tempfile.TemporaryDirectory(prefix="observer-store-", dir=os.environ.get("RUNNER_TEMP")) as temporary:
                scratch = Path(temporary)
                for phase in ("seed", "observe"):
                    name = "fs-observer-" + uuid.uuid4().hex
                    command = ["docker", "run", "--name", name, "--network", "none", "--cpus", "2",
                               "--memory", "128m", "--memory-swap", "128m", "--pids-limit", "128",
                               "--user", f"{os.getuid()}:{os.getgid()}", "--entrypoint", "python3",
                               "--mount", f"type=bind,src={harness},dst=/harness,readonly",
                               "--mount", f"type=bind,src={output},dst=/results"]
                    if phase == "seed":
                        command += ["--mount", f"type=bind,src={scratch},dst=/scratch",
                                    "--mount", f"type=bind,src={binary},dst=/binary,readonly"]
                    else:
                        command += ["--mount", f"type=bind,src={scratch / 'snapshot'},dst=/snapshot,readonly"]
                    command += [image, "/harness/check-v2-observer.py", "--inside-" + phase, "/results/" + spec_path.name]
                    try:
                        with (output / (arm + "." + phase + ".docker.log")).open("w") as log:
                            subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=180)
                        inspection = json.loads(BENCH.capture(["docker", "inspect", name]))[0]
                        BENCH.dump(output / (arm + "." + phase + ".docker-state.json"), inspection["State"])
                        if inspection["State"].get("OOMKilled"):
                            raise RuntimeError("diagnostic container OOM")
                    finally:
                        subprocess.run(["docker", "rm", "-f", name], stdout=subprocess.DEVNULL,
                                       stderr=subprocess.DEVNULL, check=False, timeout=30)
        status["success"] = True
    finally:
        BENCH.dump(output / "status.json", status)
        subprocess.run(["docker", "image", "rm", image], stdout=subprocess.DEVNULL, check=False, timeout=60)


def main():
    if len(sys.argv) == 3 and sys.argv[1] in ("--inside-seed", "--inside-observe"):
        spec = json.loads(Path(sys.argv[2]).read_text())
        return seed(spec) if sys.argv[1] == "--inside-seed" else observe(spec)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--output", required=True)
    host(parser.parse_args())


if __name__ == "__main__":
    main()
