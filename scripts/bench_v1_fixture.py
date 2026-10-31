"""Deterministic mixed metadata fixture, created only on disposable GH runners."""
import os
from pathlib import Path

STAMP = 1700000000


def prepare(root: Path, files: int, subtree_entries: int = 100, scenario: str = 'unchanged'):
    if files < 100:
        raise ValueError("mixed fixture requires at least 100 files")
    if isinstance(subtree_entries, bool) or not isinstance(subtree_entries, int) or not 3 <= subtree_entries <= files:
        raise ValueError("subtree entries must be between 3 and files")
    if scenario not in ('unchanged', 'changed', 'changed-full') or (scenario == 'changed-full' and subtree_entries < 8):
        raise ValueError('invalid fixture scenario or subtree size')
    full = scenario == 'changed-full'
    directories = {root, root / "small", root / "unrelated"}
    for directory in directories:
        directory.mkdir(parents=True, exist_ok=True)
    for i in range(subtree_entries - (4 if full else 1)):
        path = root / "small" / f"f{i:05}"
        path.touch(exist_ok=False)
        os.utime(path, (STAMP, STAMP))
    if full:
        nested = root / 'small' / 'nested'
        nested.mkdir()
        directories.add(nested)
        (nested / 'owner').write_bytes(b'old owner')
        (nested / 'removed').write_bytes(b'nested payload')
        os.link(nested / 'owner', root / 'unrelated' / 'old-link')
        (root / 'unrelated' / 'outside-owner').write_bytes(b'outside owner')
        for path in (nested / 'owner', nested / 'removed', root / 'unrelated' / 'outside-owner'):
            os.utime(path, (STAMP, STAMP))
        owner = (nested / 'owner').stat()
        outside_owner = (root / 'unrelated' / 'outside-owner').stat()
        full_plan = dict(old_owner_path='small/nested/owner', old_outside_path='unrelated/old-link',
                         new_owner_path='unrelated/outside-owner', new_link_path='small/replacement/new-link',
                         old_owner_dev=owner.st_dev, old_owner_inode=owner.st_ino, old_owner_allocated=owner.st_blocks * 512, old_owner_size=owner.st_size, old_owner_mtime=owner.st_mtime_ns,
                         outside_owner_dev=outside_owner.st_dev, outside_owner_inode=outside_owner.st_ino, outside_owner_allocated=outside_owner.st_blocks * 512, outside_owner_size=outside_owner.st_size,
                         outside_owner_mtime=outside_owner.st_mtime_ns, removed_nested_entries=3,
                         added_nested_entries=3, expected_entry_delta=0, expected_file_delta=0)
    prior = None
    counts = dict(small_files=subtree_entries - (2 if full else 1), outer_files=2 if full else 0, payload_files=0,
                  sparse_files=0, hardlink_files=0, longname_files=0)
    if full:
        counts['hardlink_files'] = 1
    for i in range(files - (subtree_entries if full else subtree_entries - 1)):
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
        counts['outer_files'] += 1
        counts['longname_files'] += bool(suffix)
        path = directory / f"f{i:09}{suffix}"
        if i % 1000 == 999:
            os.link(prior, path)
            counts['hardlink_files'] += 1
        else:
            with path.open("xb") as stream:
                if i % 100 == 0:
                    stream.write(b"x" * 4096)
                    counts['payload_files'] += 1
                elif i % 4096 == 0:
                    stream.truncate(1024 * 1024)
                    counts['sparse_files'] += 1
        os.utime(path, (STAMP, STAMP))
        prior = path
        if (i + 1) % 100000 == 0:
            print(f"fixture: {i + subtree_entries} files", flush=True)
    for directory in directories:
        os.utime(directory, (STAMP, STAMP))
    os.sync()
    receipt = dict(files=files, expected_entries=files + len(directories),
                expected_subtree_entries=subtree_entries, directories=len(directories),
                shape=("mixed-full-mutation-subtree-v1" if full else "mixed-depth1-6-fanout1000-longnames-payload1pct-sparse-hardlinks-v1"
                       if subtree_entries == 100 else "mixed-parameterized-subtree-v1"),
                fixed_mtime=STAMP, payload_block_fraction=0.01, distribution=counts)
    if full:
        receipt['mutation_plan'] = full_plan
    return receipt


def mutate_full(root):
    """Balanced shared refresh mutation; both snapshots start from the prechange tree."""
    small, outside = root / 'small', root / 'unrelated'
    def counts(directory):
        total, files = 1, 0
        for parent in (small, directory):
            with os.scandir(parent) as entries:
                for entry in entries:
                    total += 1
                    files += entry.is_file(follow_symlinks=False)
        return total, files
    before_entries, before_files = counts(small / 'nested')
    old = (small / 'nested' / 'owner').stat()
    outside_old = (outside / 'old-link').stat()
    outside_new = (outside / 'outside-owner').stat()
    if old.st_ino != outside_old.st_ino or old.st_dev != outside_old.st_dev or outside_new.st_ino == old.st_ino:
        raise ValueError('unexpected initial hardlink identities')
    renamed = (small / 'f00000').stat().st_ino
    deleted = (small / 'f00002').stat().st_ino
    nested_deleted = (small / 'nested' / 'removed').stat().st_ino
    (small / 'f00000').rename(small / 'renamed')
    (small / 'f00001').write_bytes(b'changed\n' * 1024)
    (small / 'added').write_bytes(b'added')
    replacement = small / 'replacement'
    replacement.mkdir()
    os.link(outside / 'outside-owner', replacement / 'new-link')
    (replacement / 'new-file').write_bytes(b'replacement')
    new_ids = {(small / 'added').stat().st_ino, replacement.stat().st_ino, (replacement / 'new-file').stat().st_ino}
    if len(new_ids) != 3 or new_ids & {deleted, nested_deleted, old.st_ino}:
        raise ValueError('mutation inode identities overlap')
    (small / 'f00002').unlink()
    (small / 'nested' / 'owner').unlink()
    (small / 'nested' / 'removed').unlink()
    (small / 'nested').rmdir()
    for path in (small / 'f00001', small / 'added', replacement / 'new-file', replacement, small):
        os.utime(path, (STAMP + 60, STAMP + 60))
    after_entries, after_files = counts(replacement)
    if before_entries != after_entries or before_files != after_files:
        raise ValueError('full mutation changed subtree entry count')
    os.sync()
    return dict(recipe='full-balanced-v1', renamed_inode=renamed, deleted_inode=deleted,
                nested_deleted_inode=nested_deleted, new_file_inode=(replacement / 'new-file').stat().st_ino,
                old_owner_inode=old.st_ino, old_outside_inode=(outside / 'old-link').stat().st_ino,
                outside_owner_inode=outside_new.st_ino, new_link_inode=(replacement / 'new-link').stat().st_ino,
                changed_size=(small / 'f00001').stat().st_size, changed_mtime=(small / 'f00001').stat().st_mtime_ns,
                target_mtime=small.stat().st_mtime_ns, added_inode=(small / 'added').stat().st_ino,
                added_size=(small / 'added').stat().st_size, added_mtime=(small / 'added').stat().st_mtime_ns,
                new_file_size=(replacement / 'new-file').stat().st_size, new_file_mtime=(replacement / 'new-file').stat().st_mtime_ns,
                replacement_mtime=replacement.stat().st_mtime_ns,
                old_outside_size=(outside / 'old-link').stat().st_size, old_outside_mtime=(outside / 'old-link').stat().st_mtime_ns,
                new_link_size=(replacement / 'new-link').stat().st_size, new_link_mtime=(replacement / 'new-link').stat().st_mtime_ns,
                replacement_inode=replacement.stat().st_ino, removed_nested_entries=3, added_nested_entries=3,
                subtree_entries_before=before_entries, subtree_entries_after=after_entries,
                entry_delta=after_entries - before_entries, file_delta=after_files - before_files)


def mutate(root):
    """A shared changed tree; never recreate it between compared arms."""
    small = root / "small"
    (small / "f00000").rename(small / "renamed")
    with (small / "f00001").open("wb") as stream:
        stream.write(b"changed\n" * 1024)
    os.utime(small / "f00001", (STAMP + 60, STAMP + 60))
    os.utime(small, (STAMP + 60, STAMP + 60))
    os.sync()
    return dict(recipe='rename-size-mtime', renamed_inode=(small / 'renamed').stat().st_ino,
                changed_size=(small / 'f00001').stat().st_size,
                changed_mtime=(small / 'f00001').stat().st_mtime_ns,
                target_mtime=small.stat().st_mtime_ns)
