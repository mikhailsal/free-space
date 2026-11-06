#!/usr/bin/env python3
"""Read-only peek at the installed v1 snapshot (immutable open, query_only)."""
import sqlite3
import sys
import time

DB = '/var/lib/freespace/snapshot.db'
c = sqlite3.connect(f'file:{DB}?mode=ro&immutable=1', uri=True)
c.execute('PRAGMA query_only=1')
t0 = time.monotonic()


def q(sql, *a):
    return c.execute(sql, a).fetchall()


print('meta', q('select * from meta'))
cols = ['entry_id', 'path', 'fs_type', 'fs_used', 'fs_total', 'fs_avail', 'inodes_used']
for r in q('select * from roots'):
    print('root', dict(zip(cols, [x.decode('utf-8', 'replace') if isinstance(x, bytes) else x for x in r])))
print('mounts', q('select count(*) from mounts'), 'denied', q('select count(*) from denied'))
for r in q('select entry_id,path,fs_type,fs_used,fs_total from mounts limit 40'):
    print('  mount', r[0], r[1].decode('utf-8', 'replace'), r[2], r[3], r[4])
print('hardlink rows', q('select count(*) from hardlinks'),
      'distinct (dev,ino)', q('select count(*) from (select 1 from hardlinks group by dev,ino)'))
print('elapsed so far %.1fs' % (time.monotonic() - t0))
print('kinds', q('select kind, count(*), sum(allocated) from entries group by kind'))
print('elapsed so far %.1fs' % (time.monotonic() - t0))
# Find '/' root entry and the /usr row to learn what allocated/items/subtree_end mean.
root_ids = [r[0] for r in q('select entry_id from roots')]
print('root ids', root_ids)
for rid in root_ids:
    print('root row', q('select id,parent_id,name,kind,flags,allocated,apparent,items,errors,subtree_end from entries where id=?', rid))
for rid in root_ids[:1]:
    for r in q("select id,parent_id,name,kind,allocated,items,subtree_end from entries where parent_id=? order by allocated desc limit 12", rid):
        print('  child', r[0], r[2], 'kind', r[3], 'alloc', r[4], 'items', r[5], 'subtree_end', r[6], 'end-id', r[6] - r[0])
usr = q("select id,parent_id,name,kind,allocated,items,subtree_end from entries where parent_id=? and name=cast('usr' as blob)", root_ids[0])
print('usr', usr)
print('done in %.1fs' % (time.monotonic() - t0))
sys.exit(0)
