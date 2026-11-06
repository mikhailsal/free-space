#!/usr/bin/env python3
"""Build the two memory fixtures on a mounted scratch volume (run as the owner of the mount point).

  A: 1,000,000 empty files in 4,500 directories + 1,000 at the top: the shape of the earlier "1m" fixture.
     No hardlinks, so a scanner's own heap is not influenced by link bookkeeping.
  B: 300,000 files plus 300,000 hardlinks to them in other directories (300,000 distinct hardlinked inodes,
     600,000 non-directory entries), with ~85-character paths like those under a real home directory.
"""
import os
import sys
import time

root = sys.argv[1]
t0 = time.monotonic()


def touch(path):
    os.close(os.open(path, os.O_CREAT | os.O_WRONLY, 0o644))


def make_a():
    base = os.path.join(root, 'A')
    os.makedirs(base)
    for i in range(1000):
        touch(os.path.join(base, f'top{i:04d}'))
    for d in range(4500):
        sub = os.path.join(base, f'd{d:04d}')
        os.mkdir(sub)
        for f in range(222):
            touch(os.path.join(sub, f'f{f:04d}'))
        if d % 500 == 499:
            print(f'A dirs {d + 1}/4500 {time.monotonic() - t0:.0f}s', flush=True)


def make_b():
    src, lnk = os.path.join(root, 'B', 'src'), os.path.join(root, 'B', 'lnk')
    os.makedirs(src)
    os.makedirs(lnk)
    for d in range(300):
        s = os.path.join(src, f's{d:03d}_application_data_store_component')
        l = os.path.join(lnk, f'l{d:03d}_application_data_store_mirror')
        os.mkdir(s)
        os.mkdir(l)
        for f in range(1000):
            name = f'f{f:04d}_resource_object_entry.bin'
            touch(os.path.join(s, name))
            os.link(os.path.join(s, name), os.path.join(l, name))
        if d % 50 == 49:
            print(f'B dirs {d + 1}/300 {time.monotonic() - t0:.0f}s', flush=True)


make_a()
make_b()
print(f'done in {time.monotonic() - t0:.0f}s', flush=True)
