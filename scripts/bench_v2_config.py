"""Argument parsing shared by the GitHub-only storage benchmark entry point."""

import argparse


def parse_arguments(description):
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument("--baseline", help="required unless --current-backends selects only the candidate binary")
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--steps", default="100000", help="increasing comma-separated file counts, each 1..8000000")
    parser.add_argument("--memory", type=int, choices=(128, 256, 512, 4096), default=256)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--scans", type=int, default=3)
    parser.add_argument("--refreshes", type=int, default=1)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--cpus", type=int, default=2)
    parser.add_argument("--io-device", help="host block device backing fixtures and snapshots")
    for option in ("read-bps", "write-bps", "read-iops", "write-iops"):
        parser.add_argument("--" + option, type=int, help="positive Docker block I/O limit")
    parser.add_argument("--timeout-seconds", type=int, default=900)
    parser.add_argument("--include-v1", action="store_true")
    parser.add_argument("--current-backends", action="store_true",
                        help="compare candidate v1 and candidate v2 only; overrides --include-v1")
    parser.add_argument("--scan-profile", choices=("disk", "memory", "both"), default="disk",
                        help="disk adds --low-memory; both runs memory then disk in separate output directories")
    parser.add_argument("--keep-previous", choices=("both", "true", "false"), default="both")
    args = parser.parse_args()
    if not args.current_backends and not args.baseline:
        parser.error("--baseline is required unless --current-backends is selected")
    try:
        steps = [int(value) for value in args.steps.split(",")]
    except ValueError:
        parser.error("--steps must be comma-separated integers")
    if not steps or any(not 1 <= value <= 8000000 for value in steps) or sorted(set(steps)) != steps:
        parser.error("--steps must be unique increasing counts in 1..8000000")
    for name in ("repeats", "workers", "cpus", "timeout_seconds"):
        if getattr(args, name) < 1:
            parser.error("--" + name.replace("_", "-") + " must be positive")
    if args.scans < 2 or args.refreshes < 1:
        parser.error("--scans >= 2 and --refreshes >= 1 required")
    limits = [getattr(args, name) for name in ("read_bps", "write_bps", "read_iops", "write_iops")]
    if any(value is not None and value <= 0 for value in limits):
        parser.error("I/O limits must be positive")
    if any(value is not None for value in limits) and not args.io_device:
        parser.error("--io-device is required with I/O limits")
    return args


def profile_arguments(args, profile, output):
    result = argparse.Namespace(**vars(args))
    result.scan_profile = profile
    result.output = str(output / profile)
    return result
