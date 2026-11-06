#!/usr/bin/env python3
"""Offline estimate: how many nodes/buckets the probe's retention rule keeps at other thresholds.

Reads the v1 snapshot of the whole-disk run (every entry, subtree totals) and applies the probe's rule:
a directory is retained if it is the root, or its subtree allocates at least T bytes, or its subtree holds more
than 4096 entries; a file is retained if it allocates at least T. A retained directory's remaining children are
folded into buckets of at most 4096 entries, so the number of buckets is at least ceil(folded / 4096) per retained
directory (>= 1). This is arithmetic on the v1 data, not a probe run: the probe's real bucket count at the 0.1%
threshold is printed next to the lower bound so the reader can see the gap.
"""
import json
import sqlite3
import sys
import time

DB = sys.argv[1]
CAPACITY = 473918423040
PPMS = [1000, 300, 100, 30, 10, 3]
BUCKET_CAP = 4096
con = sqlite3.connect(f'file:{DB}?mode=ro&immutable=1', uri=True)
con.execute('pragma temp_store=memory')  # the main database is opened read-only and immutable; only temp tables are written
t0 = time.time()
ROOT_BYTES = con.execute('select sum(allocated) from entries where parent_id is null').fetchone()[0]
kinds = dict(con.execute('select kind, count(*) from entries group by kind').fetchall())
print('root allocated bytes (all roots)', ROOT_BYTES)
print('entries by kind', kinds, f'{time.time() - t0:.0f}s', flush=True)
out = []
for ppm in PPMS:
    t = CAPACITY * ppm // 1_000_000 + 1
    cond = f'(kind = 1 AND (allocated >= {t} OR items > {BUCKET_CAP})) OR (kind = 0 AND allocated >= {t}) OR parent_id IS NULL'
    con.execute('drop table if exists temp.ret')
    con.execute(f'create temp table ret as select id, parent_id, kind, items from entries where {cond}')
    con.execute('create index temp.ret_parent on ret(parent_id)')
    dirs, files = (con.execute('select sum(kind = 1), sum(kind = 0) from ret').fetchone())
    folded = con.execute('''select p.id, p.items - coalesce(sum(c.items + 1), 0) from ret p left join ret c on c.parent_id = p.id
                            where p.kind = 1 group by p.id''').fetchall()
    folded_total = sum(max(0, f) for _, f in folded)
    buckets_lb = sum(max(1, -(-max(0, f) // BUCKET_CAP)) for _, f in folded)
    nodes = (dirs or 0) + (files or 0)
    file_bytes = con.execute('select coalesce(sum(allocated), 0) from entries where kind = 0 and allocated >= ?', (t,)).fetchone()[0]
    row = dict(ppm=ppm, threshold_bytes=t, retained_dirs=dirs, retained_files=files, nodes=nodes, folded_entries=folded_total,
               buckets_lower_bound=buckets_lb, retained_file_bytes=file_bytes, retained_file_share=file_bytes / ROOT_BYTES)
    out.append(row)
    print(json.dumps(row), f'{time.time() - t0:.0f}s', flush=True)
print(json.dumps(dict(kinds=kinds, rows=out)))
