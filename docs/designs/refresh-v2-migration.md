# Issue #10: experimental generation-store migration

Status: implementation branch, not a replacement for schema v1. The CLI, HTTP
API, published snapshot files and v1 query contracts remain unchanged.

## Executable stages

1. Add `snapshot::v2`, an isolated opt-in Rust API and separate SQLite file.
   Seed it from an unfinalized scanner tree; retain exact own metadata before
   folding. Stable entry identities are allocated by exact raw path, independent
   of traversal rank. A component-encoded BLOB key supplies DFS subtree ranges.
2. In one `BEGIN IMMEDIATE` transaction, replace only the selected subtree,
   version changed rows, update additive ancestor values by checked deltas, and
   read extrema through live child indexes. Publish current/prev in that same
   transaction. Full seed rotates prev; refresh preserves it.
3. Pin the generation pair in a read transaction. Provide exact-path lookup,
   children, subtree and path-based diff on versioned rows. Test stale readers,
   repeat refresh, deletion, negative timestamps, byte names, stable unrelated
   identities, rollback, and sibling-width independence.
4. Add a reproducible ignored locality benchmark: fixed refresh subtree and
   growing unrelated sibling population. Record scan/seed/refresh timings,
   affected row counts, SQLite VM steps and WAL growth. Do not infer physical
   write volume from row counts or call issue #10 complete.
5. Before CLI adoption, implement indexed touched-inode attribution including
   surviving owners and OV-11, root/mount metadata and boundary checks, all v1
   query headers/filter/tie/limit/denied/history semantics, bounded history GC
   with leases, ENOSPC/process-crash tests and measured numerical budgets.

## First slice contract and deliberate limits

The first slice supports one directory root, ordinary files, directories and
symlinks using the existing walker. Hardlinks and mounts are rejected before
publication, not silently miscounted. Exact own and aggregate values use checked
signed 64-bit arithmetic; overflows fail the transaction. Missing refreshed
non-root directories are removed. Root deletion and overlapping/multiple roots
are rejected. A directory replaced with a non-directory is rejected pending
filesystem boundary integration. Refresh history is generation-scoped.

No in-place conversion of a v1 file is allowed. A new full scan seeds a new v2
file. No automatic fallback that rewrites the whole snapshot is allowed. Stored
stable identity denotes an exact path, not an inode or a rename-tracking identity.

Version retention is unbounded in this experiment: old read transactions pin
SQLite WAL and old versions are retained in the database. There is no automatic
VACUUM, checkpoint or GC in refresh. History-dependent query and disk growth must
be addressed before production use. This is explicitly not the final retention
contract. Current live indexes bound refresh extrema work independently of
historical rows and unrelated sibling count (apart from B-tree maintenance).

## Validation commands

```
cargo test -p freespace-core --test generation_store
cargo test -p freespace-core --test generation_store locality_benchmark -- --ignored --nocapture
cargo test -p freespace-core
cargo clippy -p freespace-core --all-targets -- -D warnings
```

No latency target was supplied in issue #10. Report measured results and machine
conditions; acceptance budgets remain a dependency, not an invented guarantee.

## Implemented API and storage contract

`Store::open(dedicated_path)`, `Store::scan(root, workers)` and
`Store::refresh(existing_directory, workers)` exercise the filesystem-to-store
path. `Reader::open(dedicated_path)` pins a read transaction and generation pair;
`get`, `children`, `subtree`, `diff` and `refresh_history` operate within that pin.
The new module is exported independently of the production v1 readers/writers.
There is deliberately no CLI switch yet.

The `identities` table allocates a permanent key by collision-free raw path.
`live` contains one current row per path. `versions` records `[born, dead)`
visibility, with a partial index for each open version. `control` changes in the
same transaction as entries and history. A full scan sets prev to the former
current (including any refreshes); refresh advances current and preserves prev.

The subtree key encodes each raw byte as a big-endian 16-bit value `byte + 1`,
then terminates each component with `0,0`. Thus `a`, `a/z`, `a-` retain
component-wise DFS order. Exact subtree range replacement does not renumber
unrelated entries. New/old rows plus the ancestor chain are the mutation set.
Additive values use checked deltas; min/max recomputation reads only the first
indexed live-child contribution. No full child enumeration is needed even for
wide ancestors. Empty newest timestamps use SQL NULL, preserving negative file
timestamps correctly.

Filesystem work includes reading system mount metadata (independent of snapshot
entry count), validating ancestor directories, and walking the selected subtree.
New hardlinks/mounts, ancestor symlinks/device changes and unsupported target
replacements fail before publication. Mount/root race handling is not yet a full
production contract; it must be integrated with the existing v1 boundary rules.

Refresh runs under a SQLite immediate writer transaction, so a failed statement
rolls back row retirement, inserts, history and publication together. WAL and
FULL synchronization supply SQLite's transaction durability. Tests inject a
publication-trigger failure after staged row changes and verify current, prev,
entries and history are unchanged. Process termination/ENOSPC and retention
failure tests remain necessary; rollback testing is not evidence for those cases.

The `walk` report includes in-memory staging/folding. The `transaction` report
measures store mutation and commit after walking, not the entire duration for
which the writer lock is held. Benchmark total time includes preflight work.
`sql_changes` counts logical SQLite row changes; it does not count page writes.
`observe_vm_steps` is diagnostic instrumentation with per-instruction overhead,
so latency is measured separately without it.

### Allocation review

New per-entry allocation sites are confined to the experimental module: staging
constructs full raw paths and encoded order keys; the bottom-up fold clones an
entry; ID assignment builds a subtree-sized map; replacement collects old subtree
IDs; ancestor propagation clones two entries per level. Readers materialize their
requested result and diff builds path-keyed maps. None of these collections are
sized to unrelated live entries during refresh. Full scan still materializes the
whole scanned tree and seed rows. Optimizing these allocations is separate from
the storage-locality proof and the production scan path is untouched.

## Remaining release dependencies

- Touched-inode indexes, owner persistence, alias/owner ancestor propagation,
  unknown aliases and OV-11 parity, with cross-boundary fixtures.
- Roots/mounts/denied auxiliary metadata and complete boundary semantics.
- All existing query adapters, headers, filters, top/find/pass ordering and
  limits, diff's public semantics and invalidation. The new raw diff includes
  scan timestamps and is deliberately not API-compatible with v1 diff.
- Reader leases and bounded version/identity/history reclamation; explicit WAL
  checkpoint budgets. SQLite may checkpoint on the final connection close;
  no close/checkpoint cost is included in the local refresh benchmark.
- Root replacement/deletion, creation of an absent target, multiple roots and
  directory-to-file replacement behavior.
- Crash/process-kill/ENOSPC/concurrent-writer stress, long-run disk and reader
  latency bounds, physical I/O and RSS measurements, agreed numerical budgets.

Until these are complete, issue #10 remains open and v1 remains the default.

## Measured first-slice evidence (2026-10-26)

Command:

```
cargo test --release -p freespace-core --test generation_store locality_benchmark -- --ignored --nocapture
```

Linux 5.15.0-136-generic, ext4 on `/`, Intel Core i7-4770S (8 visible logical
CPUs), one walker worker, release profile, warm fixtures with no cache dropping.
Other host work was not controlled. Five repetitions per size, each doing a full
v2 scan, an uninstrumented v2 refresh, then a separate instrumented v2 refresh.
The fixed target is one directory containing one 32-byte file; unrelated files
are direct siblings to exercise a wide ancestor. Every scan/refresh publishes a
new generation. The measured refresh rescans unchanged data; add/delete semantics
are covered by tests but their latency distributions remain unmeasured.

| Unrelated files | v2 full scan median ms | v2 refresh median ms | SQL row changes | VM instructions | WAL growth median bytes |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 9.008 | 3.626 | 14 | 934 | 61,800 |
| 1,000 | 104.400 | 4.110 | 14 | 934 | 82,400 |
| 10,000 | 1,305.366 | 4.593 | 14 | 934 | 90,640 |

[Raw runs and stage timings](../benchmarks/refresh-v2-locality.csv) include all
15 repetitions. No cold-cache result, peak RSS, process physical-write counter,
page-dirtiness counter, v1 timing comparison or pathological hardlink result is
claimed. WAL length growth is appended SQLite page frames, not device I/O; its
increase with database size shows the expected index/page maintenance cost.
Instruction counting does not measure B-tree page traversal. Final connection
close/checkpoint is outside the timing window and retention is unbounded.

This supports local logical work for the supported two-entry fixture. It does
not establish a production end-to-end speedup, satisfy issue #10 in full, or
replace numerical budgets on the intended target filesystem.

Validation: `cargo test -p freespace-core` passed 124 tests (9 new v2 tests),
with the benchmark intentionally ignored in the ordinary suite; the explicit
release benchmark passed. `cargo clippy -p freespace-core --all-targets -- -D
warnings`, formatting and diff-whitespace checks passed. Tests cover byte paths
and DFS order, aggregate parity, extrema deletion including negative mtimes,
pinned generation pairs, prev preservation and scan rotation, transaction
rollback after staged changes, unsupported-feature rejection, reopening,
stable unrelated identities and indexed wide-parent updates.
