"""GH-only v1 pair host: isolated seeded commands and separately metered observers."""
import argparse
from contextlib import closing
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import time
import uuid

from bench_v1_admission import check_capacity
from bench_v1_fixture import prepare, mutate
from bench_v1_runtime import container, file_hash, legacy, seed_hash
from bench_v1_summary import PROTOCOL, summarize, markdown


def arguments(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("baseline", "candidate", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--files", type=int, default=100000)
    parser.add_argument("--pairs", type=int, default=1)
    parser.add_argument("--operation", choices=("scan", "refresh"), default="scan")
    parser.add_argument("--scenario", choices=("unchanged", "changed"), default="unchanged")
    parser.add_argument("--memory", type=int, choices=(128, 256, 512, 1024, 2048, 4096), default=4096)
    parser.add_argument("--timeout-seconds", type=int, default=1200)
    parser.add_argument("--io-device")
    for flag in ("read-bps", "write-bps", "read-iops", "write-iops"):
        parser.add_argument("--" + flag, type=int)
    args = parser.parse_args(argv)
    if not 100 <= args.files <= 8000000 or not 1 <= args.pairs <= 3 or args.timeout_seconds < 1:
        parser.error("files 100..8000000, pairs 1..3 and positive timeout required")
    limits = [getattr(args, x) for x in ("read_bps", "write_bps", "read_iops", "write_iops")]
    if any(x is not None and x <= 0 for x in limits) or (any(limits) and not args.io_device):
        parser.error("positive I/O limits require --io-device")
    if args.scenario == "changed" and args.operation != "refresh":
        parser.error("changed scenario is refresh-only")
    return args


def provenance(binary):
    revision = binary.with_name(binary.name + ".revision.txt").read_text().strip()
    build = json.loads(binary.with_name(binary.name + ".build.json").read_text())
    if build["source_revision"] != revision or build["sha256"] != file_hash(binary):
        raise ValueError("binary build provenance mismatch")
    return dict(sha256=build["sha256"], source_revision=revision, build=build)


def seed_timestamp(snapshot):
    uri = (snapshot / "snapshot.db").as_uri() + "?mode=ro&immutable=1"
    with closing(sqlite3.connect(uri, uri=True)) as db:
        return db.execute("SELECT scanned_at FROM meta").fetchone()[0]


def host(args):
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("paired benchmark requires a dedicated ephemeral GitHub-hosted runner")
    helper = legacy()
    output = args.output.resolve()
    output.mkdir(exist_ok=False)
    setup_output = output.with_name(output.name + "-setup")
    setup_output.mkdir(exist_ok=False)
    binaries = {"baseline-v1": args.baseline.resolve(), "candidate-v1": args.candidate.resolve()}
    sources = {arm: provenance(binary) for arm, binary in binaries.items()}
    build_keys = ("compiler", "target", "flags", "profile")
    if any(sources["baseline-v1"]["build"][key] != sources["candidate-v1"]["build"][key] for key in build_keys):
        raise ValueError("both arms must use identical toolchains/targets/flags")
    image = "freespace-pairs:" + uuid.uuid4().hex
    label = "freespace.benchmark=" + os.environ["FREESPACE_BENCHMARK_RUN_TOKEN"]
    subprocess.run(["docker", "build", "-t", image, str(Path(__file__).parent / "bench-v2-container")], check=True, timeout=600)
    completed, cleaned, failures = False, True, []
    temporary = tempfile.mkdtemp(prefix="freespace-v1-pairs-", dir=os.environ["RUNNER_TEMP"])
    scratch = Path(temporary)
    try:
        # Additional entries from deterministic mixed directory shape. This is an
        # upper estimate; receipt is replaced by exact generator accounting below.
        estimated_entries = args.files + 3 + ((args.files + 999) // 1000) * 7
        admission = check_capacity(scratch, args.files, estimated_entries,
                                   seed_bytes=estimated_entries * 256 * 2)
        helper.dump(output / "capacity-admission.json", admission)
        if not admission["success"]:
            raise RuntimeError("insufficient fixture/snapshot bytes or inodes")
        fixture = prepare(scratch / "fixture" / "tree", args.files)
        tree = scratch / "fixture" / "tree"
        io_limits = {key: getattr(args, flag) for flag, key in (("read_bps", "rbps"), ("write_bps", "wbps"),
                     ("read_iops", "riops"), ("write_iops", "wiops")) if getattr(args, flag) is not None}
        device_number = None
        if args.io_device:
            dev = os.stat(args.io_device)
            device_number = f"{os.major(dev.st_rdev)}:{os.minor(dev.st_rdev)}"
            if device_number != f"{os.major(scratch.stat().st_dev)}:{os.minor(scratch.stat().st_dev)}":
                raise ValueError("I/O device does not directly back scratch")
        common = dict(backend="v1", single_operation=True, files=args.files,
                      expected_entries=fixture["expected_entries"], expected_subtree_entries=100,
                      memory_mib=args.memory, workers=1, cpus=2, keep_previous=True,
                      scan_profile="disk", timeout_seconds=args.timeout_seconds,
                      io_limits=io_limits, io_device=args.io_device, io_device_number=device_number,
                      minimum_free_bytes=2**31)
        seed = scratch / "seed"
        (seed / "snapshot" / "tmp").mkdir(parents=True)
        (seed / "tree").mkdir()
        (seed / "config.toml").write_text("keep_previous = true\n")
        # Two preparation scans supply the same current/previous history to every
        # measured operation. Their resources/time are setup receipts, not results.
        for ordinal in (1, 2):
            ident = f"seed-{ordinal}"
            spec = dict(common, id=ident, operation="scan", target="/scratch/tree",
                        initial_snapshot_allocated_bytes=helper.occupancy(seed / "snapshot")["total"]["allocated_bytes"])
            (setup_output / ident).mkdir()
            spec_path = setup_output / (ident + ".json")
            helper.dump(spec_path, spec)
            container(image, binaries["baseline-v1"], seed, tree, setup_output, spec_path, spec, "worker", label)
            if not json.loads((setup_output / ident / "metrics.json").read_text())["validated"]:
                raise ValueError("seed scan validation failed")
            time.sleep(1.05)
        seed_digest = seed_hash(seed / "snapshot")
        cutoff = seed_timestamp(seed / "snapshot")
        if args.scenario == "changed":
            mutate(tree)
        expected_ids = [f"p{pair}-{arm}" for pair in range(1, args.pairs + 1) for arm in binaries]
        manifest = dict(measurement_protocol=PROTOCOL, pairs=args.pairs, files=args.files,
                        operation=args.operation, scenario=args.scenario, fixture_receipt=fixture,
                        expected_arms=expected_ids, provenance=sources, seed_sha256=seed_digest,
                        sample_interval_seconds=0.02, cache_policy="cold after seed copy, before single CLI",
                        observer_policy="separate no-swap container, read-only snapshot, full-file integrity and semantic digest",
                        filesystem=json.loads(helper.capture(["findmnt", "-T", str(scratch), "-J"])),
                        github={k: os.environ.get(k) for k in ("GITHUB_SHA", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT")})
        helper.dump(output / "manifest.json", manifest)
        for pair in range(1, args.pairs + 1):
            arms = list(binaries) if pair % 2 else list(reversed(binaries))
            for arm in arms:
                ident = f"p{pair}-{arm}"
                directory = output / ident
                directory.mkdir()
                case = scratch / "case"
                started_setup = time.monotonic()
                shutil.copytree(seed, case)
                if seed_hash(case / "snapshot") != seed_digest:
                    raise ValueError("copied seed digest mismatch")
                initial = helper.occupancy(case / "snapshot")["total"]["allocated_bytes"]
                available = os.statvfs(case)
                helper.dump(directory / "setup.json", dict(elapsed_seconds=time.monotonic() - started_setup,
                            copied_seed_bytes=initial, filesystem_used_bytes=helper.filesystem_used(case)))
                if available.f_bavail * available.f_frsize < 2 * initial + 2**31:
                    raise ValueError("not enough free space for output/staging and reserve")
                spec = dict(common, id=ident, arm=arm, pair=pair, operation=args.operation,
                            target="/scratch/tree/small" if args.operation == "refresh" else "/scratch/tree",
                            seed_sha256=seed_digest, seed_timestamp=cutoff,
                            initial_snapshot_allocated_bytes=initial,
                            binary_sha256=sources[arm]["sha256"], source_revision=sources[arm]["source_revision"])
                spec_path = output / (ident + ".json")
                helper.dump(spec_path, spec)
                helper.dump(directory / "spec.json", spec)
                helper.flush_caches(directory, "cache")
                (directory / "cache.cache-flush.json").rename(directory / "cache-flush.json")
                container(image, binaries[arm], case, tree, output, spec_path, spec, "worker", label)
                container(image, binaries[arm], case, tree, output, spec_path, spec, "observer", label)
                shutil.rmtree(case)
        completed = True
    except Exception as error:
        failures.append(repr(error))
        raise
    finally:
        try:
            remaining = helper.capture(["docker", "ps", "-aq", "--filter", "label=" + label])
            if remaining:
                raise RuntimeError("preserving scratch: owned containers still exist")
            shutil.rmtree(scratch)
        except Exception as error:
            cleaned = False
            failures.append("scratch cleanup: " + repr(error))
        try:
            subprocess.run(["docker", "image", "rm", image], check=True, timeout=60, stdout=subprocess.DEVNULL)
        except Exception as error:
            cleaned = False
            failures.append("image cleanup: " + repr(error))
        helper.dump(output / "status.json", dict(success=completed and cleaned and not failures,
                    completed=completed, cleanup_success=cleaned, failures=failures))
    result = summarize(output)
    helper.dump(output / "summary.json", result)
    (output / "summary.md").write_text(markdown(result) + "\n")
    print(markdown(result))
    return 0 if result["valid"] else 1


def main(argv=None):
    return host(arguments(argv))


if __name__ == "__main__":
    raise SystemExit(main())
