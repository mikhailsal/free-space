#!/usr/bin/env python3
"""Check a coarse snapshot against an independent full v1 snapshot.

Run outside the measured writer process: cleanup_oracle.py V1_DB COARSE_DB.
Both snapshots must describe the same filesystem state and root. Memory is
bounded by tree depth; SQLite performs indexed lookups for retained nodes.
"""
import argparse
import json
import sqlite3
import sys
from pathlib import Path

FIELDS = ('allocated', 'apparent', 'files', 'dirs', 'others', 'errors')


def connect(path):
    return sqlite3.connect(Path(path).resolve().as_uri() + '?mode=ro', uri=True)


def verify(baseline, coarse, v2=None, fixture=None):
    b, c = connect(baseline), connect(coarse)
    b.row_factory = c.row_factory = sqlite3.Row
    meta = c.execute('SELECT * FROM meta').fetchall()
    if len(meta) != 1:
        raise ValueError('expected one coarse metadata row')
    meta = meta[0]
    root, threshold = bytes(meta['root']), meta['threshold']
    if not 0 < meta['ppm'] <= 1_000_000 or meta['capacity'] <= 0 or threshold != (meta['capacity'] * meta['ppm'] + 999_999) // 1_000_000:
        raise ValueError('invalid capacity/ppm threshold')
    if fixture is not None:
        if meta['capacity'] != fixture['capacity_bytes'] or threshold != fixture['threshold_bytes'] or meta['ppm'] != 1000:
            raise ValueError('coarse capacity/threshold differs from fixture receipt')
    roots = b.execute('SELECT entry_id,path,fs_total FROM roots').fetchall()
    if len(roots) != 1 or bytes(roots[0]['path']) != root:
        raise ValueError('baseline root differs or is not unique')
    if fixture is not None and roots[0]['fs_total'] != fixture['capacity_bytes']:
        raise ValueError('v1 capacity differs from fixture receipt')
    root_id = roots[0]['entry_id']
    controls = {}
    if fixture is not None:
        for label, keep in (('retained', True), ('omitted', False)):
            for relative in fixture['expected_controls'][label]:
                path = root.rstrip(b'/') + b'/' + relative.encode('utf-8')
                if path in controls:
                    raise ValueError('duplicate fixture control')
                controls[path] = keep
    seen_controls = set()
    root_totals = None
    stack = []
    visited = retained = 0

    def finish():
        nonlocal retained, root_totals
        n = stack.pop()
        row, path, children, rest = n['row'], n['path'], n['children'], n['rest']
        kind = min(row['kind'], 2)
        totals = [row['allocated'], row['apparent'], *n['counts'], row['errors']]
        totals[2 + kind] += 1
        own = [totals[0] - children[0], totals[1] - children[1], totals[5] - children[5]]
        if min(own) < 0 or row['items'] != sum(n['counts']):
            raise ValueError(f'invalid baseline aggregate at {path.hex()}')
        keep = row['id'] == root_id or totals[0] >= threshold
        newest = row['newest_mtime'] if totals[2] else None
        if path in controls:
            seen_controls.add(path)
            if keep != controls[path]:
                raise ValueError(f'fixture control threshold classification differs: {path.hex()}')
        if row['id'] == root_id:
            root_totals = totals
        if keep:
            actual = c.execute('SELECT * FROM nodes WHERE path=?', (path,)).fetchone()
            if actual is None:
                raise ValueError(f'missing retained node {path.hex()}')
            retained += 1
            parent = stack[-1]['path'] if stack else None
            expected = {'parent': parent, 'kind': kind, 'own_allocated': own[0], 'own_apparent': own[1], 'own_errors': own[2]}
            expected.update(dict(zip(FIELDS, totals)))
            expected.update(('rest_' + k, v) for k, v in zip(FIELDS, rest))
            expected['newest_mtime'] = newest
            expected['rest_newest_mtime'] = n['rest_newest']
            times = [actual['scanned_at'], n['child_min'], actual['rest_min_scanned_at']]
            times = [t for t in times if t is not None]
            expected['min_scanned_at'] = min(times)
            if bool(sum(rest[2:5])) != (actual['rest_min_scanned_at'] is not None):
                raise ValueError(f'{path.hex()} invalid rest freshness presence')
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
            if not keep and newest is not None:
                p['rest_newest'] = max(p['rest_newest'], newest) if p['rest_newest'] is not None else newest
            if keep:
                child_min = actual['min_scanned_at']
                p['child_min'] = min(p['child_min'], child_min) if p['child_min'] is not None else child_min
        return keep

    for row in b.execute('SELECT id,parent_id,name,kind,allocated,apparent,items,errors,subtree_end,newest_mtime FROM entries ORDER BY id'):
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
        stack.append({'row': row, 'path': path, 'children': [0]*6, 'rest': [0]*6, 'counts': [0]*3, 'rest_newest': None, 'child_min': None})
        visited += 1
    while stack:
        finish()
    if not visited or c.execute('SELECT COUNT(*) FROM nodes').fetchone()[0] != retained:
        raise ValueError('extra coarse nodes or empty baseline')
    if fixture is not None:
        counts = fixture['expected_counts']
        if seen_controls != controls.keys() or visited != counts['entries'] or tuple(root_totals[2:5]) != (counts['files'], counts['dirs'], counts['others']):
            raise ValueError('fixture controls or expected entry counts differ')
    if v2 is not None:
        v = connect(v2)
        roots2 = v.execute('SELECT r.path,l.allocated,l.apparent,l.items,l.errors,r.total FROM live_roots r JOIN live_rows l ON l.id=r.entry_id').fetchall()
        cr = c.execute('SELECT allocated,apparent,files,dirs,others,errors FROM nodes WHERE path=?', (root,)).fetchone()
        if len(roots2) != 1 or bytes(roots2[0][0]) != root or tuple(roots2[0][1:5]) != (cr[0], cr[1], cr[2] + cr[3] + cr[4] - 1, cr[5]) or (fixture is not None and roots2[0][5] != fixture['capacity_bytes']):
            raise ValueError('v2 root differs from baseline/coarse totals')
        if fixture is not None:
            kinds = [0, 0, 0]
            for kind, count in v.execute('SELECT kind,COUNT(*) FROM live_rows GROUP BY kind'):
                kinds[min(kind, 2)] += count
            if tuple(kinds) != (counts['files'], counts['dirs'], counts['others']):
                raise ValueError('v2 fixture entry counts differ')
    return {'success': True, 'visited': visited, 'retained': retained, 'threshold': threshold, 'v2_checked': v2 is not None, 'fixture_checked': fixture is not None}


def main():
    try:
        parser = argparse.ArgumentParser()
        parser.add_argument('baseline')
        parser.add_argument('coarse')
        parser.add_argument('v2', nargs='?')
        parser.add_argument('--fixture', type=Path)
        args = parser.parse_args()
        fixture = json.loads(args.fixture.read_text()) if args.fixture else None
        print(json.dumps(verify(args.baseline, args.coarse, args.v2, fixture)))
    except (ValueError, KeyError, TypeError, sqlite3.Error, OSError) as error:
        print(json.dumps({'success': False, 'error': str(error)}))
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
