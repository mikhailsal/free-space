#!/usr/bin/env python3
"""Streaming metadata intervention; Linux x86_64 only, no payload reads."""
import argparse
import ctypes
import hashlib
import json
import os
import platform
import stat
import struct
import sys
import time


class Dirent(ctypes.Structure):
    _fields_ = [('ino', ctypes.c_ulong), ('offset', ctypes.c_long),
                ('reclen', ctypes.c_ushort), ('type', ctypes.c_ubyte),
                ('name', ctypes.c_char * 256)]


LIBC = None


def libc():
    global LIBC
    if LIBC is None:
        library = ctypes.CDLL(None, use_errno=True)
        library.opendir.argtypes = [ctypes.c_char_p]
        library.opendir.restype = ctypes.c_void_p
        library.readdir.argtypes = [ctypes.c_void_p]
        library.readdir.restype = ctypes.POINTER(Dirent)
        library.closedir.argtypes = [ctypes.c_void_p]
        library.closedir.restype = ctypes.c_int
        LIBC = library
    return LIBC


class Directory:
    def __init__(self, path):
        if (platform.system(), platform.machine()) != ('Linux', 'x86_64'):
            raise RuntimeError('requires Linux x86_64 dirent ABI')
        if (ctypes.sizeof(ctypes.c_void_p), Dirent.name.offset,
                ctypes.sizeof(Dirent)) != (8, 19, 280):
            raise RuntimeError('unsupported dirent ABI')
        self.lib = libc()
        self.handle = self.lib.opendir(path)
        if not self.handle:
            raise OSError(ctypes.get_errno(), 'opendir failed', path)

    def __iter__(self):
        return self

    def __next__(self):
        while True:
            ctypes.set_errno(0)
            item = self.lib.readdir(self.handle)
            if not item:
                error = ctypes.get_errno()
                if error:
                    raise OSError(error, 'readdir failed')
                raise StopIteration
            record = item.contents
            size = record.reclen
            if not 24 <= size <= 280 or size % 8:
                raise ValueError('invalid dirent record length')
            raw = ctypes.string_at(ctypes.addressof(record) + 19, size - 19)
            end = raw.find(b'\0')
            if end < 0 or end > 255:
                raise ValueError('unterminated dirent name')
            name = raw[:end]
            if not name or b'/' in name or not record.ino:
                raise ValueError('invalid dirent name/inode')
            if name in (b'.', b'..'):
                continue
            if record.type == 0:
                raise ValueError('DT_UNKNOWN; stat fallback prohibited')
            if record.type not in (4, 8):
                raise ValueError('unexpected file type in fixture')
            return name, record.type, record.ino

    def close(self):
        if self.handle:
            handle, self.handle = self.handle, None
            if self.lib.closedir(handle):
                raise OSError(ctypes.get_errno(), 'closedir failed')


def probe(root, mode, directory=Directory):
    result = dict(protocol='resource-cause-v1', mode=mode, entries=0,
                  regular_files=0, directories=0, directory_opens=0,
                  root_lstat_calls=0, entry_stat_calls=0,
                  path_kind_digest=None, digest_algorithm='sha256-sum256-path-length-kind-v1',
                  validated=False)
    if mode == 'idle':
        time.sleep(1)
        result.update(validated=True, traversal=False)
        return result
    if mode not in ('names', 'stat'):
        raise ValueError('invalid probe mode')
    root = os.fsencode(root)
    metadata = os.lstat(root)
    result['root_lstat_calls'] = 1
    if not stat.S_ISDIR(metadata.st_mode):
        raise ValueError('root is not a directory')
    digest = 0

    def add(relative, kind):
        nonlocal digest
        value = struct.pack('>Q', len(relative)) + relative + bytes([kind])
        digest = (digest + int.from_bytes(hashlib.sha256(value).digest(), 'big')) % (1 << 256)
        result['entries'] += 1
        result['directories' if kind == 4 else 'regular_files'] += 1

    add(b'', 4)
    stack = []
    try:
        stack.append((b'', directory(root)))
        result['directory_opens'] += 1
        while stack:
            relative, iterator = stack[-1]
            try:
                name, kind, inode = next(iterator)
            except StopIteration:
                iterator.close()
                stack.pop()
                continue
            child = relative + (b'/' if relative else b'') + name
            absolute = root + b'/' + child
            if mode == 'stat':
                metadata = os.stat(absolute, follow_symlinks=False)
                result['entry_stat_calls'] += 1
                expected = stat.S_IFDIR if kind == 4 else stat.S_IFREG
                if stat.S_IFMT(metadata.st_mode) != expected or metadata.st_ino != inode:
                    raise ValueError('dirent/stat inode or type mismatch')
            add(child, kind)
            if kind == 4:
                if len(stack) >= 64 or len(absolute) > 4095:
                    raise ValueError('fixture depth/path bound exceeded')
                stack.append((child, directory(absolute)))
                result['directory_opens'] += 1
    finally:
        for _, iterator in reversed(stack):
            iterator.close()
    result.update(path_kind_digest=f'{digest:064x}', traversal=True, validated=True)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--backend', choices=['v1'], required=True)
    parser.add_argument('operation', choices=['scan'])
    parser.add_argument('root')
    parser.add_argument('--workers', type=int, choices=[1], required=True)
    parser.add_argument('--json', action='store_true', required=True)
    parser.add_argument('--low-memory', action='store_true', required=True)
    args = parser.parse_args(argv)
    try:
        result = probe(args.root, os.environ.get('FREESPACE_CAUSE_PROBE_MODE'))
    except Exception as error:
        print(json.dumps(dict(protocol='resource-cause-v1', validated=False,
                              error=repr(error))), flush=True)
        return 1
    print(json.dumps(result), flush=True)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
