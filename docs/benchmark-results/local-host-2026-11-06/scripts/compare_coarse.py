#!/usr/bin/env python3
"""Compare two coarse.db files row by row, ignoring the scan time stamps."""
import hashlib
import sqlite3
import sys

IGNORE = {'scanned_at', 'min_scanned_at', 'generation'}


def dump(path):
    con = sqlite3.connect(f'file:{path}?mode=ro&immutable=1', uri=True)
    out = {}
    for table in ('meta', 'nodes', 'buckets'):
        cols = [r[1] for r in con.execute(f'pragma table_info({table})') if r[1] not in IGNORE]
        rows = sorted(con.execute(f'select {",".join(cols)} from {table}').fetchall(), key=repr)
        out[table] = (len(rows), hashlib.sha256(repr(rows).encode()).hexdigest()[:16], cols)
    return out


a, b = dump(sys.argv[1]), dump(sys.argv[2])
same = True
for table in a:
    eq = a[table][:2] == b[table][:2]
    same &= eq
    print(f'{table:8} rows {a[table][0]:>7} vs {b[table][0]:>7}  hash {a[table][1]} vs {b[table][1]}  {"SAME" if eq else "DIFFERENT"}')
print('RESULT:', 'IDENTICAL' if same else 'DIFFERENT')
sys.exit(0 if same else 1)
