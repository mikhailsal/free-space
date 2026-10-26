# Issue #10: experimental generation-store migration

Status: opt-in CLI/HTTP backend with timer lifecycle and explicit maintenance on
the implementation branch. V1 remains the default and keeps its existing
snapshot files. Final publication checks are pending. See
[the operator guide](../refresh-v2.md).

## Executable stages

1. **Implemented:** seed a dedicated SQLite store from the existing walker,
   retaining exact own metadata. Stable identities are `(configured root, raw
   path)`, independent of traversal rank. Component-encoded BLOB keys support
   ordered subtree ranges and root scopes disambiguate mount placeholders from
   independently configured nested roots.
2. **Implemented:** replace old/new subtree rows, version changed entries and
   propagate checked additive deltas over the union of affected ancestor paths.
   Indexed live-child extrema avoid enumerating wide sibling sets. Atomic
   control-row publication advances current; full scan rotates prev, while
   refresh preserves it.
3. **Implemented:** index live inode groups and examine only groups touched by
   the old/new subtree. Preserve v1's surviving-owner, returning-owner-path and
   OV-11 rules. Update changed external aliases and owner ancestors. Retain
   generation-scoped root/mount metadata and global denied-subtree reporting.
4. **Implemented:** pin readers to one generation pair; expose exact-path,
   children, subtree, root, mount, owner and refresh-history queries. Add opt-in
   public ls/top/find/diff/status adapters with persisted generation headers and
   configured root order; retain stable-ID-independent traversal ordering.
   Prepare filesystem work outside transactions; acquire the writer lock only
   for publication and reject a stale base generation before changing rows.
5. **Implemented experiments:** deterministic v1/v2 attribution comparisons,
   byte/order/extrema/rollback/pinned-reader tests, concurrent prepared writers,
   mount/root/denied fixtures and unrelated-entry/inode-group locality checks.
   An ignored release benchmark records full v2 scan and local refresh timings,
   logical SQL mutations, separately instrumented VM steps and WAL growth.
6. **Implemented:** conservative read-only reader leases, bounded cursor GC,
   persisted record accounting/admission limits and explicit checkpoint upkeep.
   Test public query parity while pinned, after GC, and through capacity recovery.
7. **Implemented:** disk-staged v2 seed with raw own values and exact aggregates,
   using the existing bounded-memory walker. Both seed profiles share atomic
   publication and honor `keep_previous`. Complete denied reports stream to disk.
8. **Implemented:** CLI scan options, timer install/uninstall, maintenance and
   policy recovery, complete write reports, process-kill, syscall-boundary and
   actual private-volume ENOSPC tests. Interrupted stages are recovered under
   lifetime locks.
9. **Measured:** complete CLI refresh scaling and constrained-memory full scans
   from source `575044b6`; raw manifests/reports and tradeoffs are archived in
   `docs/benchmarks.md`. Final CI and publication checks remain.

Validation commands:

```
cargo test -p freespace-core
cargo clippy -p freespace-core --all-targets -- -D warnings
cargo test --release -p freespace-core --test generation_store locality_benchmark -- --ignored --nocapture
```

## Storage and API contract

`Store::open(dedicated_path)`, `Store::scan(root, workers)`,
`Store::scan_roots(roots, one_file_system, workers)` and
`Store::refresh(existing_directory, workers)` exercise the complete
filesystem-to-store path. `Reader::open(dedicated_path)` pins a read transaction
and current/prev pair. `--backend v2` or config `backend = "v2"` selects the
separate store; the operator guide covers seeding and ongoing maintenance.

The `identities` table allocates a permanent integer key by collision-free raw
root/path equality. `live` contains one current row per scoped path; `versions`
records `[born, dead)` visibility with a partial index for each open version.
The subtree key encodes each byte as big-endian `byte + 1`, then terminates each
component with `0,0`. Thus `a`, `a/z`, `a-` retain component-wise DFS order. Scope
and key form an index: local replacement neither renumbers nor enumerates
unrelated entries. Root and mount rows use the same generation intervals.

Raw own file sizes remain available even when an alias is charged zero bytes.
An indexed `(dev, ino)` lookup obtains old membership and ownership only for
inodes touched by old/new subtree entries. With a surviving known owner outside
refresh, that owner remains charged. A replaced owner retains its path when
possible; otherwise the remaining raw-path minimum wins. Without that owner,
fewer known aliases than recorded `nlink` zeros the group (OV-11). Observing the
missing aliases can later restore one owner. Full scans choose the smallest raw
full path, which is deliberately distinct from component-wise DFS order.

The changed subtree and changed external aliases contribute checked deltas.
Ancestor paths are deduplicated and updated deepest-first once per generation;
shared ancestors receive their net delta. Child extrema come from the first row
in covering indexes. Empty newest timestamps use SQL NULL, preserving real
negative file timestamps. Work includes S (old/new subtree), H (members of
changed inode groups), A (the union of affected ancestors), B-tree maintenance,
and system mount metadata, independently of unrelated snapshot entries.

## Roots, mounts and denied state

The existing walker supplies root normalization, duplicate-root handling,
same-filesystem overlap rejection, cross-device limits and bind-mount handling.
The v1 live mount-boundary checker is shared through crate-local visibility;
its implementation and v1 call behavior are unchanged. Configured nested roots
retain their own scope, while a mount placeholder at the same raw path remains
in its parent's scope. Exact-path lookup chooses the closest configured root;
subtree queries stay within that scope.

Refresh can update a configured root or remove an absent non-root directory.
Missing roots and directory-to-file/symlink replacements retain v1's rejection
behavior. New paths require refreshing an existing parent. A mount placeholder
cannot itself be refreshed as a directory. Refreshing its enclosing subtree can
add, update or retire mount records. Safe `statvfs` numbers on the enclosing root
are refreshed. Denied flags and exact own errors remain versioned with entries;
reader-level global incompleteness and capped non-nested denied paths use the
same pin. The opt-in public adapter consumes these headers and filters.

Root/mount identities and generation retention are tested with synthetic
cross-device/nested-root fixtures, and local mount replacement uses walker mount
fixtures. These are not claims of privileged live-mount/unmount stress coverage.

## Atomicity and contention

Preparation reads the target, enclosing root and base generation in a short
read transaction, then releases it. Boundary checks, filesystem traversal,
raw-row staging and root `statvfs` run with no database transaction held.
Publication starts `BEGIN IMMEDIATE`, verifies that current still equals the
base generation, and applies changes plus control/history updates atomically.
A competing publication causes `Rotated` before any mutations; callers must
retry with a new walk. There is no unbounded automatic retry. Tests prepare two
writers against one generation and publish concurrently: exactly one succeeds,
the other is rejected, and a fresh retry preserves both changes.

WAL and FULL synchronization supply SQLite transaction durability. Injected
publication-trigger failures after local and external-owner updates roll back
entries, root metadata, current, prev and history together. Separate subprocess
tests stop writers at selected WAL/staging writes and inject ENOSPC at the OS
write boundary. They verify both generation views, metadata, history, internal
accounting and retries. `generation_volume` separately passed actual ENOSPC for
staging, final WAL and checkpoint writes on a private 768 MiB tmpfs. Its barrier
resumes a real write after filling that isolated volume; it never substitutes an
errno. Published views/accounting survive and retries work after filler removal.
The preload barrier requires dynamically linked glibc on Linux, a C compiler,
passwordless sudo and private mount
namespaces; it explicitly skips when namespace privileges are unavailable.
This verifies the bounded tmpfs fixture, not every filesystem's failure modes.

The lifecycle suite repeats 160 refreshes and 16 full-scan rotations with pinned
readers, accounting checks and bounded GC. After reclamation the retained-record
count returns to 211 in each sampled cycle. A separate 8-versus-1,024 unrelated
entry fixture keeps 25 logical SQL changes and 1,732 VM instructions across each
of 12 refreshes. These are workload-specific correctness/locality checks, not
hardware latency budgets. An isolated private mount namespace also compares
v1/v2 against live tmpfs and same-device bind mounts: one-filesystem/cross-mount,
nested roots, both seed profiles, refresh, and late boundary changes. Namespace
privileges are required for those mount cases; unavailable privileges are
reported as a skip.

## Retention, reader leases and recovery

DB, WAL, SHM and reader-lease files use v1's 0640 ownership/mode helper; a new
dedicated directory uses 0750. The database and lease are restricted at creation,
including under a permissive umask. SQLite inherits DB modes for sidecars, and
persistent WAL/SHM sidecars let group-read-only clients reopen the live store
without creating files in that directory. `journal_size_limit=0` trims the WAL
at checkpoint/reset; automatic refresh checkpoints remain disabled. Empty or
missing stores report NoSnapshot, while foreign/malformed databases report Schema.
These contracts have isolated process/umask and first-run regression coverage.

Each Reader acquires a shared `flock` on a persistent `.readers` sibling file,
then opens its read-only SQLite transaction. Maintenance requests the exclusive
lock without waiting. If any Reader is alive, it reports `readers_blocking` and
performs no pruning or checkpoint. Reader drop or process termination releases
the kernel lease. Canonical paths share the same lease; hardlinked database
filenames are rejected. This conservative local-filesystem protocol preserves
read-only client access and every pinned current/prev pair, including idle
readers, at the cost of deferred maintenance while any reader remains open.

`Store::maintain(budget, checkpoint)` examines at most `budget` candidate records
(1–100,000) through persisted cursors and ordered indexes. It preserves exact
current/prev version intervals and both generations' full refresh histories,
pruning obsolete entry/root/mount versions, generation headers, history and
unreferenced identities. IDs never recycle; a path recreated after its entire
history was reclaimed receives a new identity. Cursor/accounting changes and
pruning commit together. Failure after partial pruning rolls everything back.

Admission uses transactional O(1) record accounting. Defaults are 100,000,000
retained logical records and a 512 MiB WAL admission threshold; callers can set
explicit `RetentionPolicy` values. The published 7.8M-entry fixture needs about
15.6M records for its first seed (identities plus versions), and about 23.4M for
two full generations, before root/mount/header/history overhead. The 100M
logical default accommodates that baseline and recovery rotations; it is not
a measured disk-space or RSS guarantee. Production defaults still need sizing
against available disk and the bounded-RSS seed. Counted records are identities, all version
rows, generation headers and refresh history. A publication exceeding the record
budget rolls back. WAL admission is a threshold plus one admitted transaction,
**not a hard byte cap**. Row size, index overhead, uncommitted spill and database
high-water space require separate physical measurement. No automatic VACUUM or
full rewrite occurs during refresh.

Checkpoint is explicitly requested maintenance and can do global WAL work. It
uses a nonblocking TRUNCATE checkpoint and reports SQLite contention. Maintenance
without checkpoint also respects WAL admission; checkpoint requests clear an
already over-budget WAL before adding pruning writes, or return busy without
pruning. SQLite may additionally checkpoint on final connection close. Neither
explicit maintenance nor close/checkpoint cost is hidden in refresh timing.

A long-running current generation can fill the budget with history that cannot
legally be pruned. The error gives the exact failed-publication record count and
a numeric temporary budget/headroom for retrying the operation followed by two
full scans of the same indexed tree (one additional scan when the failed
operation was already a full scan). Recovery is:

1. Close old readers and attempt bounded maintenance/checkpoint.
2. If protected history itself fills capacity, explicitly set the reported
   temporary record budget and retry the failed operation.
3. Run two full scans after a refresh retry. The first preserves the old current
   and its complete history as prev; the second rotates it out. A changed/growing
   filesystem can require a newly reported budget; the estimate is conditional
   on the indexed tree remaining the same.
4. Run bounded maintenance/checkpoint, then restore the lower budget once the
   retained-record count fits. Required history is never silently truncated.

Tests cover this sequence with and without an old Reader: active leases defer
reclamation, the first scan preserves prev/diff semantics, and two rotations
allow history reclamation and resumed refresh under the lower budget.

## Experimental format and measurements

The engine is v2, with experimental SQLite `user_version = 4` for the
root-scoped/inode, header and retention layout. Prior experimental stores must be
reseeded into a new file. No v1 file or earlier published inode is mutated or
silently migrated, and no whole-snapshot rewrite fallback is introduced.

`walk` includes filesystem preparation and raw in-memory staging; full scans
also fold and attribute their seed there. Refresh attribution/folding and
ancestor updates occur in `transaction`, which now includes writer-lock
acquisition through commit. Total benchmark time includes initial metadata and
boundary checks. `sql_changes` counts logical rows, not page writes.
`observe_vm_steps` adds per-instruction overhead, so latency is measured
separately without it. Neither instruction counts nor WAL file growth equal
physical device I/O.

### Allocation review

The memory scan and local-refresh profile construct one raw path and encoded
order key per staged entry, raw-ID and inode maps, and a cloned entry during each
bottom-up fold. Stable-ID assignment retains a subtree-sized map. Refresh loads
only the old subtree and touched inode groups; ancestor state contains only the
union of affected paths. Experimental `Reader::diff` materializes path maps;
the public query adapter streams and uses bounded result heaps.

The disk scan profile has no entry-count-sized Rust collection. Its allocations
and lifetimes are:

| Allocation | Bound and lifetime |
| --- | --- |
| Raw staging SQLite pages | 8 MiB target cache; disk B-trees contain raw rows, jobs, links and mounts throughout the scan |
| V2 SQLite pages | 8 MiB target cache, mmap disabled, temp storage on disk; retained for the Store connection |
| Root metadata | One record per configured root, through the staged scan |
| DFS path buffer | One growing raw path, reused during the visitor |
| Raw row/name and link/mount metadata | One row at a time; owner lookup uses `(dev,ino,path,root,id)` index |
| Visitor stack | One cursor/name and Entry per active depth; each Entry holds ancestor path, scope and encoded key until its children finish |
| Stable identity | Indexed lookup/insert per row; parent ID lives in the depth stack, with no full raw-to-stable map |
| Entry SQL | Cached fixed-size statements; bound path/key blobs live for one call; live/version rows are inserted once on subtree exit |
| Denied report | One raw path/string at a time plus five warning examples; complete records go to an unlinked disk spool |

SQLite cache values are targets, not process RSS caps. Ancestor path/key storage
can grow quadratically with extreme depth, but not with unrelated entry count.
Per-row SQL formatting currently allocates short fixed-size statement strings.
Staging and publication check the existing filesystem reserve at least every
1,024 visitor steps and once after visiting. Staging uses a secure 0640 file named
`scan-<16 lowercase hex digits>.stage.tmp` beside the v2 database; normal success
and error paths unlink it. Each stage holds an exclusive advisory lock from
before SQLite opens it through publication and removal. Before a new disk scan,
recovery opens only exact-pattern names with `O_NOFOLLOW`, accepts only regular
single-link files, acquires a nonblocking exclusive lock, and rechecks inode
identity before unlinking. Active concurrent scanners, symlinks, hardlinks,
directories and unknown names are preserved. A killed scanner releases its lock
so the next disk scan reclaims the abandoned file. A creation/recovery race is
detected before SQLite opens the stage and retried with another name. The v2
transaction rolls back all generation rows on
visitor, reserve, admission or SQLite failure. Full scan writer locks span the
staged database traversal, but never its earlier filesystem walk.

`Store::scan_roots_with_options(roots, one_file_system, workers, low_memory,
keep_previous)` selects the profile. Existing `scan`/`scan_roots` keep memory/prev
defaults. `keep_previous=false` atomically clears the public prev pointer and
header; already pinned readers retain both old generations until dropped, then
bounded maintenance may reclaim their rows. It does not synchronously delete
retained history. `Reader::denied_report` and `visit_denied` emit every denied
entry, including nested ones, without loading refresh history or imposing the
status response's 100-item cap.

## Public query adapter and remaining release dependencies

`queries::v2` returns the existing public response types for ls/top/find/diff,
status/status_page and the filtered pass. It streams indexed scoped generations,
uses component ordering for ties/nesting and bounded result heaps, and pairs diff
rows by exact path. It does not synthesize DFS IDs, materialize compatibility
snapshots, or copy full stores. Both generations use the same leased Reader.
Persisted headers include full-scan timestamps, host/OS/version, entry counts and
fresh snapshot IDs; root ordinals preserve configured-root order.

Eight cross-backend parity fixtures exercise hundreds of parameter combinations,
including filters, limits, byte paths, ties after stable IDs diverge, denied
suppression, overlapping mount/root paths, missing snapshots, invalidation,
status and pinned-pair behavior across publication and GC. Deep diff currently
streams the scoped subtree below the requested depth; memory is bounded, but
extra scoped reads remain to be benchmarked/optimized. The experimental raw
`Reader::diff` remains separate from the public adapter.

CLI/HTTP routing, maintenance/policy recovery and timer lifecycle have integrated
tests, including malformed/empty stores, complete denied reports, report timing
groups, read-only access and isolated root install/uninstall. V1 remains default.

Acceptance measurements use the exact integrated CLI binary: three warm
refresh/scan repetitions at each unrelated-tree size, plus four no-swap128MiB
full-seed runs up to a million files. Complete command time includes close and
checkpoint. Reported RSS and kernel process I/O counters complement the logical
SQL tests; raw data and full-scan/storage tradeoffs are in `docs/benchmarks.md`.
Differential tests cover raw own values beyond u32 saturation, scoped hardlink
ties, mount/denied metadata, negative timestamps, byte paths, refresh and both
keep-prev modes.

Evidence limits remain explicit: no cold-cache or target Raspberry Pi run,
direct device-write/dirty-page attribution, pathological H/A latency bound or
deep-query performance guarantee is claimed. No target-machine numerical budget
was supplied; the128MiB fixture demonstrates one working limit. Admission limits
are not physical-size guarantees. These qualifications constrain performance
claims; final CI/publication is required before closing issue #10.

## Measured revision-4 evidence (2026-10-26)

Linux 5.15.0-136-generic, ext4 on `/`, Intel Core i7-4770S (8 visible logical
CPUs), one walker worker, release profile, warm fixtures without cache dropping.
Other host work was not controlled. Five repetitions per size each run a full
v2 scan, an uninstrumented v2 refresh, then a separate instrumented v2 refresh.
The target is one directory with one 32-byte file; unrelated files are direct
siblings, exercising a wide ancestor. Every operation publishes a generation.

| Unrelated files | v2 full scan median ms | v2 refresh median ms | SQL row changes | VM instructions | WAL growth median bytes |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 59.023 | 17.587 | 25 | 1,732 | 123,600 |
| 1,000 | 385.534 | 17.465 | 25 | 1,732 | 127,720 |
| 10,000 | 2,740.527 | 5.935 | 25 | 1,732 | 131,840 |

[Raw runs and stage timings](../benchmarks/refresh-v2-locality.csv) contain all
15 repetitions. Compared with the earlier experimental layout, mutation counts
now include versioned root/header metadata and accounting triggers. Host
contention makes these timings noisy; the instruction/mutation counts are the
locality evidence. WAL growth includes index/page maintenance
and is not constant. No cold-cache, RSS, device-write, dirty-page, v1 timing or
pathological fanout timing result is claimed. Checkpoint/connection-close cost
and explicit maintenance remain outside the benchmark window.

A separate deterministic locality test deletes an owner and transfers attribution
outside the refreshed subtree while growing unrelated inode groups from 10 to
1,000. It asserts one touched group, two known members, two deduplicated ancestors,
equal SQL mutation counts and no linear increase in VM instructions. This proves
only that fixture's unrelated-group isolation; H/A fanout measurements remain.

This historical benchmark predates the integrated disk seed, CLI report path,
timer/maintenance commands and expanded crash tests. It demonstrates logical
locality for the supported fixture and is not the final complete-command timing.
The current validation commands above rerun strict formatting, Clippy, size,
query/retention parity and fault tests. Final CLI/cgroup evidence is recorded in
`docs/benchmarks.md` with its exact measured source and binary hashes.
