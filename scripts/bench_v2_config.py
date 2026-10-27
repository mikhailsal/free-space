"""Argument parsing shared by the GitHub-only storage benchmark entry point."""

import argparse


def parse_arguments(description):
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument("--baseline", help="required unless --current-backends selects only the candidate binary")
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--steps", default="100000")
    parser.add_argument("--memory", type=int, choices=(128, 256, 512, 4096), default=256)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--scans", type=int, default=3)
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
    return args


def profile_arguments(args, profile, output):
    result = argparse.Namespace(**vars(args))
    result.scan_profile = profile
    result.output = str(output / profile)
    return result
