#!/usr/bin/env python3
"""Planning estimates and fail-closed admission for the v1 benchmark.

These are explicit planning allowances, not measured peaks or universal filesystem
upper bounds. They apply only to the mixed metadata fixture described below;
filesystem overhead and SQLite growth must be checked against observations.
"""
import math
import os

GIB = 1024 ** 3


def _count(name, value):
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"{name} must be a nonnegative integer")


def estimate_capacity(files: int, expected_entries: int, seed_bytes: int = 0,
                      reserve_bytes: int = 2 * GIB) -> dict:
    """Estimate additional space, including all potentially coexisting copies.

    Assumes at most 1% of files allocate a 4 KiB payload block, plus 16 MiB
    for sparse extents; 512 bytes per entry for inode/directory metadata; and
    512 bytes per entry for each of four snapshot-sized files (current,
    previous, stage and output). The original seed remains present in addition
    to those files. Reserve covers other transient overhead. These constants are
    planning assumptions, not guarantees for arbitrary filesystems or schemas.
    """
    for name, value in (("files", files), ("expected_entries", expected_entries),
                        ("seed_bytes", seed_bytes), ("reserve_bytes", reserve_bytes)):
        _count(name, value)
    if expected_entries < files:
        raise ValueError("expected_entries must include all fixture files")
    components = {
        "fixture_metadata_bytes": expected_entries * 512,
        "fixture_payload_bytes": ((files + 99) // 100) * 4096,
        "sparse_extent_bytes": 16 * 1024 ** 2,
        "current_snapshot_bytes": expected_entries * 512,
        "previous_snapshot_bytes": expected_entries * 512,
        "stage_snapshot_bytes": expected_entries * 512,
        "output_snapshot_bytes": expected_entries * 512,
        "retained_seed_bytes": seed_bytes,
        "reserve_bytes": reserve_bytes,
    }
    return {
        "files": files,
        "expected_entries": expected_entries,
        "components": components,
        "required_bytes": sum(components.values()),
        "required_inodes": expected_entries + 10000,
        "assumptions": "Planning allowances, not measured peaks or filesystem-independent upper bounds; "
                       "1% allocated 4 KiB payload blocks, 16 MiB sparse extents, "
                       "512 bytes per fixture entry and per entry in each of four coexisting snapshots; "
                       "seed retained separately, plus byte and inode reserves.",
    }


def check_capacity(path, files: int, expected_entries: int, seed_bytes: int = 0,
                   reserve_bytes: int = 2 * GIB) -> dict:
    """Compare planning requirements with available blocks and inodes at path."""
    estimate = estimate_capacity(files, expected_entries, seed_bytes, reserve_bytes)
    receipt = dict(estimate, path=os.fspath(path), success=False)
    try:
        stats = os.statvfs(path)
        block_size = stats.f_frsize or stats.f_bsize
        if block_size <= 0 or stats.f_bavail < 0 or stats.f_favail < 0:
            raise ValueError("invalid statvfs availability")
        receipt["available_bytes"] = stats.f_bavail * block_size
        receipt["available_inodes"] = stats.f_favail
        receipt["success"] = (receipt["available_bytes"] >= estimate["required_bytes"] and
                              receipt["available_inodes"] >= estimate["required_inodes"])
        if not receipt["success"]:
            receipt["error"] = "insufficient available bytes or inodes"
    except (OSError, ValueError, AttributeError) as error:
        receipt["error"] = str(error)
    return receipt


def admit_time(remaining_seconds, estimated_seconds, margin=0.3) -> dict:
    """Fail closed when time is unknown or cannot accommodate the margin."""
    receipt = dict(remaining_seconds=remaining_seconds, estimated_seconds=estimated_seconds,
                   margin=margin, success=False)
    values = (remaining_seconds, estimated_seconds, margin)
    if any(isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v)
           for v in values) or remaining_seconds <= 0 or estimated_seconds <= 0 or margin < 0:
        receipt["error"] = "time estimate and remaining time must be positive and margin nonnegative"
        return receipt
    receipt["required_seconds"] = estimated_seconds * (1 + margin)
    receipt["success"] = math.isfinite(receipt["required_seconds"]) and remaining_seconds >= receipt["required_seconds"]
    if not receipt["success"]:
        receipt["error"] = "insufficient remaining time"
    return receipt
