#!/usr/bin/env python3
"""Check a bucketed coarse snapshot against an independent full v1 snapshot.

Run outside the measured writer process: cleanup_oracle.py V1_DB COARSE_DB [V2_DB] [--fixture FILE].
Both snapshots must describe the same filesystem state and root. Memory is bounded by tree depth
and by the (at most 4096) name-hash slots of each open directory; SQLite performs indexed lookups
for retained nodes and buckets. The bucket layout is recomputed here from the v1 entries with the
independent reference in cleanup_buckets.py, not read back from the Rust implementation.
"""
import argparse
import json
import sqlite3
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cleanup_buckets as cb

FIELDS = ('allocated', 'apparent', 'files', 'dirs', 'others', 'errors')
BUCKET_SQL = ('SELECT lo,hi,allocated,apparent,files,dirs,others,errors,links,min_scanned_at,newest_mtime '
              'FROM buckets WHERE parent=? ORDER BY lo')


def connect(path):
    return sqlite3.connect(Path(path).resolve().as_uri() + '?mode=ro', uri=True)


def read_meta(c, fixture):
    meta = c.execute('SELECT * FROM meta').fetchall()
    if len(meta) != 1:
        raise ValueError('expected one coarse metadata row')
    meta = meta[0]
    threshold = meta['threshold']
    if not 0 < meta['ppm'] <= 1_000_000 or meta['capacity'] <= 0 or threshold != (meta['capacity'] * meta['ppm'] + 999_999) // 1_000_000:
        raise ValueError('invalid capacity/ppm threshold')
    if (meta['format'], meta['slot_bits'], meta['bucket_entries']) != (cb.FORMAT, cb.SLOT_BITS, cb.BUCKET_ENTRIES):
        raise ValueError('coarse snapshot format differs from the reference bucket layout')
    if fixture is not None:
        if meta['capacity'] != fixture['capacity_bytes'] or threshold != fixture['threshold_bytes'] or meta['ppm'] != 1000:
            raise ValueError('coarse capacity/threshold differs from fixture receipt')
        if meta['has_hardlinks']:
            raise ValueError('fixture trees contain no hardlinks')
    return meta


def read_controls(fixture, root):
    controls = {}
    if fixture is not None:
        for label, keep in (('retained', True), ('omitted', False)):
            for relative in fixture['expected_controls'][label]:
                path = root.rstrip(b'/') + b'/' + relative.encode('utf-8')
                if path in controls:
                    raise ValueError('duplicate fixture control')
                controls[path] = keep
    return controls


class Walk:
    """Replay the v1 preorder and require the coarse snapshot to be exactly its folded image."""

    def __init__(self, c, root, root_id, threshold, controls):
        self.c, self.root, self.root_id, self.threshold, self.controls = c, root, root_id, threshold, controls
        self.stack = []
        self.visited = self.retained = self.buckets = self.max_bucket = 0
        self.seen_controls = set()
        self.root_totals = None

    def push(self, row):
        if self.stack:
            if row['parent_id'] != self.stack[-1]['row']['id']:
                raise ValueError('baseline is not dense preorder')
            path = self.stack[-1]['path'].rstrip(b'/') + b'/' + bytes(row['name'])
        else:
            if self.visited or row['id'] != self.root_id or row['parent_id'] is not None:
                raise ValueError('invalid baseline root/preorder')
            path = self.root
        self.stack.append({'row': row, 'path': path, 'children': [0] * 6, 'counts': [0] * 3, 'child_min': None, 'slots': {}})
        self.visited += 1

    def check_buckets(self, n, kind):
        rows = self.c.execute(BUCKET_SQL, (n['path'],)).fetchall()
        if kind != 1:
            if rows:
                raise ValueError(f"{n['path'].hex()} non-directory node has buckets")
            return []
        expected = cb.tile(n['slots'])
        if len(rows) != len(expected):
            raise ValueError(f"{n['path'].hex()} bucket count: expected {len(expected)}, got {len(rows)}")
        mins = []
        for (lo, hi, stats), row in zip(expected, rows):
            want = {'lo': lo, 'hi': hi, 'links': 0, 'newest_mtime': stats.newest, **{f: getattr(stats, f) for f in FIELDS}}
            for key, value in want.items():
                if row[key] != value:
                    raise ValueError(f"{n['path'].hex()} bucket {lo}-{hi} {key}: expected {value!r}, got {row[key]!r}")
            if (row['min_scanned_at'] is not None) != (stats.entries > 0):
                raise ValueError(f"{n['path'].hex()} bucket {lo}-{hi} invalid freshness presence")
            mins.append(row['min_scanned_at'])
            self.max_bucket = max(self.max_bucket, stats.entries)
        self.buckets += len(rows)
        return mins

    def check_node(self, n, kind, totals, own, newest):
        path = n['path']
        actual = self.c.execute('SELECT * FROM nodes WHERE path=?', (path,)).fetchone()
        if actual is None:
            raise ValueError(f'missing retained node {path.hex()}')
        self.retained += 1
        parent = self.stack[-1]['path'] if self.stack else None
        expected = {'parent': parent, 'kind': kind, 'own_allocated': own[0], 'own_apparent': own[1], 'own_errors': own[2],
                    'links': 0, 'newest_mtime': newest}
        expected.update(zip(FIELDS, totals))
        times = [actual['scanned_at'], n['child_min'], *self.check_buckets(n, kind)]
        expected['min_scanned_at'] = min(t for t in times if t is not None)
        for key, value in expected.items():
            if actual[key] != value:
                raise ValueError(f'{path.hex()} {key}: expected {value!r}, got {actual[key]!r}')
        return actual

    def finish(self):
        n = self.stack.pop()
        row, path = n['row'], n['path']
        kind = min(row['kind'], 2)
        totals = [row['allocated'], row['apparent'], *n['counts'], row['errors']]
        totals[2 + kind] += 1
        own = [totals[0] - n['children'][0], totals[1] - n['children'][1], totals[5] - n['children'][5]]
        if min(own) < 0 or row['items'] != sum(n['counts']):
            raise ValueError(f'invalid baseline aggregate at {path.hex()}')
        # Retained for size, or because a folded directory this large would make a refresh as
        # expensive as rescanning everything below it.
        keep = row['id'] == self.root_id or totals[0] >= self.threshold or sum(totals[2:5]) > cb.BUCKET_ENTRIES
        newest = row['newest_mtime'] if totals[2] else None
        if path in self.controls:
            self.seen_controls.add(path)
            if keep != self.controls[path]:
                raise ValueError(f'fixture control threshold classification differs: {path.hex()}')
        if row['id'] == self.root_id:
            self.root_totals = totals
        actual = self.check_node(n, kind, totals, own, newest) if keep else None
        if self.stack:
            p = self.stack[-1]
            for i, value in enumerate(totals):
                p['children'][i] += value
            for i in range(3):
                p['counts'][i] += totals[2 + i]
            if not keep:
                p['slots'].setdefault(cb.slot_of(bytes(row['name'])), cb.Stats()).add_totals(totals, newest)
            else:
                child_min = actual['min_scanned_at']
                p['child_min'] = min(p['child_min'], child_min) if p['child_min'] is not None else child_min


def check_v2(c, v2, root, fixture, counts):
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


def verify(baseline, coarse, v2=None, fixture=None):
    b, c = connect(baseline), connect(coarse)
    b.row_factory = c.row_factory = sqlite3.Row
    meta = read_meta(c, fixture)
    root, threshold = bytes(meta['root']), meta['threshold']
    roots = b.execute('SELECT entry_id,path,fs_total FROM roots').fetchall()
    if len(roots) != 1 or bytes(roots[0]['path']) != root:
        raise ValueError('baseline root differs or is not unique')
    if fixture is not None and roots[0]['fs_total'] != fixture['capacity_bytes']:
        raise ValueError('v1 capacity differs from fixture receipt')
    controls = read_controls(fixture, root)
    walk = Walk(c, root, roots[0]['entry_id'], threshold, controls)
    for row in b.execute('SELECT id,parent_id,name,kind,allocated,apparent,items,errors,subtree_end,newest_mtime FROM entries ORDER BY id'):
        while walk.stack and row['id'] > walk.stack[-1]['row']['subtree_end']:
            walk.finish()
        walk.push(row)
    while walk.stack:
        walk.finish()
    if not walk.visited or c.execute('SELECT COUNT(*) FROM nodes').fetchone()[0] != walk.retained:
        raise ValueError('extra coarse nodes or empty baseline')
    if c.execute('SELECT COUNT(*) FROM buckets').fetchone()[0] != walk.buckets:
        raise ValueError('extra coarse buckets')
    counts = fixture['expected_counts'] if fixture is not None else None
    if fixture is not None:
        if walk.seen_controls != controls.keys() or walk.visited != counts['entries'] or tuple(walk.root_totals[2:5]) != (counts['files'], counts['dirs'], counts['others']):
            raise ValueError('fixture controls or expected entry counts differ')
    if v2 is not None:
        check_v2(c, v2, root, fixture, counts)
    return {'success': True, 'visited': walk.visited, 'retained': walk.retained, 'buckets': walk.buckets,
            'max_bucket_entries': walk.max_bucket, 'threshold': threshold, 'v2_checked': v2 is not None,
            'fixture_checked': fixture is not None}


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
