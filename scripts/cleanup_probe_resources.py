#!/usr/bin/env python3
"""Record host headroom for the 8 GiB benchmark cgroup ceiling."""
import json
import os
from pathlib import Path
import sys
import time

GIB = 1024 ** 3
MIN_BYTES = 10 * GIB
MIN_CPUS = 2


def parse_meminfo(text):
    fields = {}
    for line in text.splitlines():
        if ':' not in line:
            continue
        name, raw = line.split(':', 1)
        if name not in ('MemTotal', 'MemAvailable'):
            continue
        parts = raw.split()
        if len(parts) != 2 or parts[1] != 'kB' or not parts[0].isdigit() or name in fields:
            raise ValueError('invalid or duplicate meminfo field: ' + name)
        fields[name] = int(parts[0]) * 1024
    if set(fields) != {'MemTotal', 'MemAvailable'}:
        raise ValueError('missing host meminfo fields')
    return fields


def validate(receipt):
    return (isinstance(receipt, dict) and receipt.get('success') is True
            and type(receipt.get('mem_total_bytes')) is int and receipt['mem_total_bytes'] >= MIN_BYTES
            and type(receipt.get('mem_available_bytes')) is int and receipt['mem_available_bytes'] >= MIN_BYTES
            and type(receipt.get('cpu_count')) is int and receipt['cpu_count'] >= MIN_CPUS)


def capture(meminfo=None, cpu_count=None):
    if meminfo is None:
        meminfo = Path('/proc/meminfo').read_text()
    if cpu_count is None:
        cpu_count = os.cpu_count()
    values = parse_meminfo(meminfo)
    receipt = dict(mem_total_bytes=values['MemTotal'], mem_available_bytes=values['MemAvailable'],
                   cpu_count=cpu_count, minimum_memory_bytes=MIN_BYTES, minimum_cpus=MIN_CPUS,
                   writer_ceiling_bytes=8 * GIB, host_overhead_bytes=2 * GIB,
                   captured_unix_seconds=time.time(), success=True,
                   limitation='Host headroom is a point-in-time observation; global pressure can still occur.')
    receipt['success'] = validate(receipt)
    return receipt


def require(receipt):
    if not validate(receipt):
        raise ValueError('host memory/CPU headroom below required 10 GiB and 2 CPUs')
    return receipt


def main():
    if len(sys.argv) != 2:
        raise SystemExit('usage: cleanup_probe_resources.py OUTPUT')
    destination = Path(sys.argv[1])
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        receipt = capture()
    except (OSError, ValueError) as error:
        receipt = {'success': False, 'error': str(error)}
    destination.write_text(json.dumps(receipt, indent=2) + '\n')
    if not validate(receipt):
        raise SystemExit('host resource admission failed')


if __name__ == '__main__':
    main()
