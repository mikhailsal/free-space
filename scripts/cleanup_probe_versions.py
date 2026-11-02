#!/usr/bin/env python3
"""Capture the runtime versions from the frozen benchmark image."""
import json
import platform
import sqlite3
import subprocess


def capture(command):
    return subprocess.check_output(command, text=True, timeout=10).strip()


def main():
    record = {
        'python': platform.python_version(),
        'sqlite': sqlite3.sqlite_version,
        'fio': capture(['fio', '--version']),
        'time': capture(['/usr/bin/time', '--version']).splitlines()[0],
        'packages': capture(['dpkg-query', '-W', '-f=${Package}=${Version}\n']).splitlines(),
    }
    print(json.dumps(record, sort_keys=True))


if __name__ == '__main__':
    main()
