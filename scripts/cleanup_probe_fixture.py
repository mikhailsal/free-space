#!/usr/bin/env python3
"""Build fixed allocated-size threshold fixtures on an ephemeral Actions VM."""
import json
import os
from pathlib import Path
import sys

STAMP = 1700000000
GIB = 1024 ** 3
CHUNK_CHILDREN = 1000  # children of each bulk chunk directory g000000, g000001, ...
SMALL_FILES = 99       # empty files in 'small'
CONTROL_FILES = 5      # below, equalish, above, deep/large and sparse
GROUP_FILES = 64       # files in 'large/group'


def receipt_shape(profile, count):
    """Entry counts the construction loops of `build` produce, kept apart so the plan's contract tests use them."""
    bulk_files = count if profile == 'files' else count // 2
    bulk_directories = 0 if profile == 'files' else count // 2
    files = bulk_files + SMALL_FILES + CONTROL_FILES + GROUP_FILES + 1  # the last one is the raw-name file
    dirs = bulk_directories + count // CHUNK_CHILDREN + 7
    return dict(bulk_files=bulk_files, bulk_directories=bulk_directories,
                expected_counts=dict(files=files, dirs=dirs, others=0, entries=files + dirs))


def guard():
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        raise RuntimeError('fixture requires a GitHub-hosted runner')


def check_space(root):
    fs = os.statvfs(root)
    if fs.f_bavail * fs.f_frsize < 2 * GIB:
        raise RuntimeError('fixture exhausted inner 2 GiB writer reserve')
    for name in ('CLEANUP_OUTER_PATH', 'CLEANUP_OUTPUT'):
        fs = os.statvfs(os.environ[name])
        if fs.f_bavail * fs.f_frsize < 2 * GIB:
            raise RuntimeError(f'fixture exhausted outer reserve on {name}')


def allocate(path, size):
    with path.open('xb') as f:
        os.posix_fallocate(f.fileno(), 0, size)
    os.utime(path, (STAMP, STAMP))
    s = path.stat()
    return dict(apparent=s.st_size, allocated=s.st_blocks * 512)


def build(root, profile, count):
    guard()
    if profile not in ('files', 'directories') or count not in (100000, 1000000):
        raise ValueError('invalid fixture shape')
    root = Path(root)
    root.mkdir(parents=True, exist_ok=False)
    fs = os.statvfs(root)
    capacity = fs.f_blocks * fs.f_frsize
    threshold = (capacity + 999) // 1000
    if not (490 * GIB <= capacity <= 500 * GIB):
        raise ValueError('inner filesystem capacity is outside the frozen 500 GiB sparse-image envelope')
    block = fs.f_frsize
    rounded = (threshold // block) * block
    # The counts in receipt_shape come from the construction loops below, including all controls,
    # directory chunks and the root. No second enumeration is needed.
    receipt = dict(profile=profile, scale=count, capacity_bytes=capacity, threshold_bytes=threshold,
                   block_bytes=block, controls={}, fixed_timestamp=STAMP,
                   bulk_allocated_files=count // 2 if profile == 'files' else count // 4,
                   bulk_empty_files=count // 2 if profile == 'files' else count // 4,
                   bulk_allocation_bytes_per_file=4096, **receipt_shape(profile, count))
    # 'bulk' is kept only by the entries rule at 100k scale and by both rules at 1M; 'small' and the wide
    # target chunk stay below both limits, so a refresh below them must be answered from folded buckets.
    receipt['expected_controls'] = {
        'retained': ['large', 'large/above', 'large/deep', 'large/deep/deeper',
                     'large/deep/deeper/large', 'large/group', 'bulk'],
        'omitted': ['large/below', 'large/sparse', 'small', 'bulk/g000050'] +
                   [f'large/group/f{i:06}' for i in range(64)]}
    small = root / 'small'
    retained = root / 'large'
    small.mkdir(); retained.mkdir()
    for i in range(SMALL_FILES):
        (small / f'f{i:03}').touch()
    os.sync()
    before_backing = os.stat(os.environ['CLEANUP_BACKING_FILE']).st_blocks * 512
    for label, size in (('below', rounded - block), ('equalish', rounded), ('above', rounded + block)):
        receipt['controls'][label] = allocate(retained / label, size)
    deep = retained / 'deep' / 'deeper'
    deep.mkdir(parents=True)
    receipt['controls']['deep'] = allocate(deep / 'large', rounded + block)
    os.sync()
    after_backing = os.stat(os.environ['CLEANUP_BACKING_FILE']).st_blocks * 512
    inner_control_blocks = sum(receipt['controls'][name]['allocated'] for name in ('below','equalish','above','deep'))
    receipt['sparse_backing_capability'] = dict(before_outer_allocated=before_backing, after_outer_allocated=after_backing,
                                                outer_growth=after_backing-before_backing, inner_allocated=inner_control_blocks)
    if after_backing - before_backing >= inner_control_blocks:
        raise RuntimeError('sparse loop did not conserve outer allocation for unwritten inner extents')
    check_space(root)
    sparse = retained / 'sparse'
    with sparse.open('xb') as f:
        f.truncate(2 * threshold)
    ss = sparse.stat()
    receipt['controls']['sparse'] = dict(apparent=ss.st_size, allocated=ss.st_blocks * 512)
    group = retained / 'group'
    group.mkdir()
    group_count = GROUP_FILES
    group_child_size = ((threshold + group_count * block - 1) // (group_count * block)) * block
    if group_child_size >= threshold: raise ValueError('group child is not subthreshold')
    group_allocated = 0
    child_allocations = []
    for i in range(group_count):
        if i % 1000 == 0: check_space(root)
        allocated = allocate(group / f'f{i:06}', group_child_size)['allocated']
        child_allocations.append(allocated)
        group_allocated += allocated
    receipt['controls']['group'] = dict(files=group_count, child_size=group_child_size, child_allocated=group_allocated,
                                      minimum_child_allocated=min(child_allocations), maximum_child_allocated=max(child_allocations))
    if not (receipt['controls']['below']['allocated'] < threshold <= receipt['controls']['above']['allocated']
            and receipt['controls']['sparse']['allocated'] < threshold
            and max(child_allocations) < threshold and group_allocated >= threshold):
        raise RuntimeError('actual allocated controls do not straddle threshold')
    shapes = root / 'bulk'
    shapes.mkdir()
    bulk_allocated = 0
    for i in range(count):
        if i % CHUNK_CHILDREN == 0:
            check_space(root)
            chunk = shapes / f'g{i // CHUNK_CHILDREN:06}'
            chunk.mkdir()
        path = chunk / f'n{i:09}'
        if profile == 'directories' and i % 2 == 0:
            path.mkdir()
        elif (i % 2 == 0 if profile == 'files' else (i // 2) % 2 == 0):
            bulk_allocated += allocate(path, 4096)['allocated']
        else:
            path.touch()
    receipt['bulk_actual_allocated_bytes'] = bulk_allocated
    # Include a byte name that is not valid UTF-8 without changing locale.
    raw = os.fsencode(retained) + b'/raw-\xff'
    fd = os.open(raw, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
    os.close(fd)
    for directory, dirs, files in os.walk(root, topdown=False):
        for name in files:
            os.utime(os.path.join(directory, name), (STAMP, STAMP), follow_symlinks=False)
        os.utime(directory, (STAMP, STAMP), follow_symlinks=False)
    os.sync()
    check_space(root)
    after = os.statvfs(root)
    receipt.update(available_bytes=after.f_bavail * after.f_frsize,
                   inodes_available=after.f_favail, raw_name_hex=raw.hex())
    return receipt


if __name__ == '__main__':
    if len(sys.argv) != 5:
        raise SystemExit('usage: cleanup_probe_fixture.py ROOT PROFILE COUNT RECEIPT')
    Path(sys.argv[4]).write_text(json.dumps(build(sys.argv[1], sys.argv[2], int(sys.argv[3])), indent=2) + '\n')
