#!/usr/bin/env python3
"""Check a coarse snapshot against an independent full v1 snapshot.

Run outside the measured writer process: cleanup_oracle.py V1_DB COARSE_DB.
Both snapshots must describe the same filesystem state and root. Memory is
bounded by tree depth; SQLite performs indexed lookups for retained nodes.
"""
import json
import sqlite3
import sys
from pathlib import Path

FIELDS = ('allocated', 'apparent', 'files', 'dirs', 'others', 'errors')


def connect(path):
    return sqlite3.connect(Path(path).resolve().as_uri() + '?mode=ro', uri=True)


def verify(baseline, coarse, v2=None):
    b, c = connect(baseline), connect(coarse)
    b.row_factory = c.row_factory = sqlite3.Row
    meta = c.execute('SELECT * FROM meta').fetchall()
    if len(meta) != 1:
        raise ValueError('expected one coarse metadata row')
    meta = meta[0]
    root, threshold = bytes(meta['root']), meta['threshold']
    if not 0 < meta['ppm'] <= 1_000_000 or meta['capacity'] <= 0 or threshold != (meta['capacity'] * meta['ppm'] + 999_999) // 1_000_000:
        raise ValueError('invalid capacity/ppm threshold')
    roots = b.execute('SELECT entry_id,path FROM roots').fetchall()
    if len(roots) != 1 or bytes(roots[0]['path']) != root:
        raise ValueError('baseline root differs or is not unique')
    root_id = roots[0]['entry_id']
    stack = []
    visited = retained = 0

    def finish():
        nonlocal retained
        n = stack.pop()
        row, path, children, rest = n['row'], n['path'], n['children'], n['rest']
        kind = min(row['kind'], 2)
        totals = [row['allocated'], row['apparent'], *n['counts'], row['errors']]
        totals[2 + kind] += 1
        own = [totals[0] - children[0], totals[1] - children[1], totals[5] - children[5]]
        if min(own) < 0 or row['items'] != sum(n['counts']):
            raise ValueError(f'invalid baseline aggregate at {path.hex()}')
        keep = row['id'] == root_id or totals[0] >= threshold
        if keep:
            actual = c.execute('SELECT * FROM nodes WHERE path=?', (path,)).fetchone()
            if actual is None:
                raise ValueError(f'missing retained node {path.hex()}')
            retained += 1
            parent = stack[-1]['path'] if stack else None
            expected = {'parent': parent, 'kind': kind, 'own_allocated': own[0], 'own_apparent': own[1], 'own_errors': own[2]}
            expected.update(dict(zip(FIELDS, totals)))
            expected.update(('rest_' + k, v) for k, v in zip(FIELDS, rest))
            for key, value in expected.items():
                if actual[key] != value:
                    raise ValueError(f'{path.hex()} {key}: expected {value!r}, got {actual[key]!r}')
        if stack:
            p = stack[-1]
            for i, value in enumerate(totals):
                p['children'][i] += value
                if not keep:
                    p['rest'][i] += value
            for i in range(3):
                p['counts'][i] += totals[2 + i]
        return keep

    for row in b.execute('SELECT id,parent_id,name,kind,allocated,apparent,items,errors,subtree_end FROM entries ORDER BY id'):
        while stack and row['id'] > stack[-1]['row']['subtree_end']:
            finish()
        if stack:
            if row['parent_id'] != stack[-1]['row']['id']:
                raise ValueError('baseline is not dense preorder')
            path = stack[-1]['path'].rstrip(b'/') + b'/' + bytes(row['name'])
        else:
            if visited or row['id'] != root_id or row['parent_id'] is not None:
                raise ValueError('invalid baseline root/preorder')
            path = root
        stack.append({'row': row, 'path': path, 'children': [0]*6, 'rest': [0]*6, 'counts': [0]*3})
        visited += 1
    while stack:
        finish()
    if not visited or c.execute('SELECT COUNT(*) FROM nodes').fetchone()[0] != retained:
        raise ValueError('extra coarse nodes or empty baseline')
    if v2 is not None:
        v = connect(v2)
        roots2 = v.execute('SELECT r.path,l.allocated,l.apparent,l.items,l.errors FROM live_roots r JOIN live_rows l ON l.id=r.entry_id').fetchall()
        cr = c.execute('SELECT allocated,apparent,files,dirs,others,errors FROM nodes WHERE path=?', (root,)).fetchone()
        if len(roots2) != 1 or bytes(roots2[0][0]) != root or tuple(roots2[0][1:]) != (cr[0], cr[1], cr[2] + cr[3] + cr[4] - 1, cr[5]):
            raise ValueError('v2 root differs from baseline/coarse totals')
    return {'success': True, 'visited': visited, 'retained': retained, 'threshold': threshold, 'v2_checked': v2 is not None}


def main():
    try:
        if len(sys.argv) not in (3, 4):
            raise ValueError('usage: cleanup_oracle.py V1_DB COARSE_DB [V2_DB]')
        print(json.dumps(verify(*sys.argv[1:])))
    except (ValueError, sqlite3.Error, OSError) as error:
        print(json.dumps({'success': False, 'error': str(error)}))
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
