#!/usr/bin/env python3
"""Facts about a coarse.db: size, retained nodes, buckets and hardlink exposure of refreshes."""
import json
import os
import sqlite3
import sys

path = sys.argv[1]
con = sqlite3.connect(f'file:{path}?mode=ro&immutable=1', uri=True)
st = os.stat(path)
q = lambda sql, *a: con.execute(sql, a).fetchall()  # noqa: E731
out = {'file': path, 'logical_bytes': st.st_size, 'allocated_bytes': st.st_blocks * 512}
out['page_size'], out['page_count'], out['freelist'] = (q('pragma page_size')[0][0], q('pragma page_count')[0][0], q('pragma freelist_count')[0][0])
meta = q('select root,threshold,ppm,capacity,generation,scanned_at,has_hardlinks from meta')[0]
out['meta'] = dict(root=bytes(meta[0]).decode(errors='replace'), threshold=meta[1], ppm=meta[2], capacity=meta[3], generation=meta[4], has_hardlinks=meta[6])
nodes = q('select path,kind,allocated,apparent,files,dirs,others,links,errors from nodes')
buckets = q('select parent,lo,hi,allocated,files,dirs,others,links from buckets')
out['nodes'] = len(nodes)
out['nodes_by_kind'] = {'dir': sum(1 for n in nodes if n[1] == 1), 'file': sum(1 for n in nodes if n[1] == 0), 'other': sum(1 for n in nodes if n[1] == 2)}
out['buckets'] = len(buckets)
root = [n for n in nodes if bytes(n[0]) == bytes(meta[0])][0]
out['root_total'] = dict(allocated=root[2], apparent=root[3], files=root[4], dirs=root[5], others=root[6], entries=root[4] + root[5] + root[6], links=root[7], errors=root[8])
top = sorted(nodes, key=lambda n: -n[2])[:25]
out['top_nodes'] = [dict(path=bytes(n[0]).decode(errors='replace'), kind=n[1], allocated=n[2], entries=n[4] + n[5] + n[6], links=n[7]) for n in top]

# Hardlink exposure: a refresh whose planned region holds a hardlinked entry rescans the whole root.
b_entries = [(b[4] + b[5] + b[6], b[7]) for b in buckets]
total_b = sum(e for e, _ in b_entries)
linked_b = sum(e for e, l in b_entries if l > 0)
dir_nodes = [n for n in nodes if n[1] == 1 and bytes(n[0]) != bytes(meta[0])]
out['hardlink_exposure'] = dict(
    buckets_with_links=sum(1 for _, l in b_entries if l > 0), buckets_total=len(buckets),
    folded_entries_total=total_b, folded_entries_in_linked_buckets=linked_b,
    folded_entry_share_falling_back=(linked_b / total_b) if total_b else 0,
    retained_dirs=len(dir_nodes), retained_dirs_with_links=sum(1 for n in dir_nodes if n[7] > 0),
    retained_dir_share_falling_back=(sum(1 for n in dir_nodes if n[7] > 0) / len(dir_nodes)) if dir_nodes else 0,
    root_links=root[7])

# Typical refresh work when it does not fall back: entries in the bucket window or retained subtree.
def pct(values, ps=(0.5, 0.9, 0.99)):
    values = sorted(values)
    return {f'p{int(p * 100)}': values[min(len(values) - 1, int(p * len(values)))] for p in ps} if values else {}

out['bucket_window_entries'] = pct([e for e, _ in b_entries])
out['retained_dir_entries'] = pct([n[4] + n[5] + n[6] for n in dir_nodes])
out['avg_bytes_per_retained_node_and_bucket'] = st.st_size / max(1, len(nodes) + len(buckets))
print(json.dumps(out, indent=2, ensure_ascii=False))
