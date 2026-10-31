"""Deterministic mixed metadata fixture, created only on disposable GH runners."""
import os
from pathlib import Path

STAMP = 1700000000


def prepare(root: Path, files: int):
    if files < 100:
        raise ValueError("mixed fixture requires at least 100 files")
    directories = {root, root / "small", root / "unrelated"}
    for directory in directories:
        directory.mkdir(parents=True, exist_ok=True)
    for i in range(99):
        path = root / "small" / f"f{i:05}"
        path.touch(exist_ok=False)
        os.utime(path, (STAMP, STAMP))
    prior = None
    for i in range(files - 99):
        if i % 1000 == 0:
            chunk = i // 1000
            depth = 1 + chunk % 6
            directory = root / "unrelated" / f"g{chunk:06}"
            for level in range(depth):
                directory /= f"depth{level}"
            current = directory
            while current not in directories:
                directories.add(current)
                current = current.parent
            directory.mkdir(parents=True, exist_ok=True)
        suffix = "x" * (80 if i % 31 == 0 else 0)
        path = directory / f"f{i:09}{suffix}"
        if i % 1000 == 999:
            os.link(prior, path)
        else:
            with path.open("xb") as stream:
                if i % 100 == 0:
                    stream.write(b"x" * 4096)
                elif i % 4096 == 0:
                    stream.truncate(1024 * 1024)
        os.utime(path, (STAMP, STAMP))
        prior = path
        if (i + 1) % 100000 == 0:
            print(f"fixture: {i + 100} files", flush=True)
    for directory in directories:
        os.utime(directory, (STAMP, STAMP))
    os.sync()
    return dict(files=files, expected_entries=files + len(directories),
                expected_subtree_entries=100, directories=len(directories),
                shape="mixed-depth1-6-fanout1000-longnames-payload1pct-sparse-hardlinks-v1",
                fixed_mtime=STAMP, payload_block_fraction=0.01)


def mutate(root):
    """A shared changed tree; never recreate it between compared arms."""
    small = root / "small"
    (small / "f00000").rename(small / "renamed")
    with (small / "f00001").open("wb") as stream:
        stream.write(b"changed\n" * 1024)
    os.utime(small / "f00001", (STAMP + 60, STAMP + 60))
    os.utime(small, (STAMP + 60, STAMP + 60))
    os.sync()
