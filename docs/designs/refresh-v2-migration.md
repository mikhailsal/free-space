# Issue #10: experimental generation-store migration

Status: opt-in Rust API on an implementation branch. The production CLI, HTTP
API, published snapshot files and v1 query contracts remain unchanged. Issue #10
is not complete until the remaining production gates below are satisfied.

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
   children, subtree, raw diff, root, mount, owner and refresh-history queries.
   Prepare filesystem work outside transactions; acquire the writer lock only
   for publication and reject a stale base generation before changing rows.
5. **Implemented experiments:** deterministic v1/v2 attribution comparisons,
   byte/order/extrema/rollback/pinned-reader tests, concurrent prepared writers,
   mount/root/denied fixtures and unrelated-entry/inode-group locality checks.
   An ignored release benchmark records full v2 scan and local refresh timings,
   logical SQL mutations, separately instrumented VM steps and WAL growth.
6. **Remaining:** implement production query adapters, complete retention/GC and
   checkpoint budgets, crash/ENOSPC coverage and production integration. Measure
   physical I/O, memory, long-run history costs and target-machine budgets before
   considering v2 the default.

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
and current/prev pair. No CLI switch is provided yet.

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
same pin. Production query headers/filtering still need adapters.

Root/mount identities and generation retention are tested with synthetic
cross-device/nested-root fixtures, and local mount replacement uses walker mount
fixtures. These are not claims of privileged live-mount/unmount stress coverage.

## Atomicity, contention and retention

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
entries, root metadata, current, prev and history together. Process termination
and ENOSPC remain separate tests; statement rollback is not evidence for them.

History retention is deliberately unbounded in this experiment. Old readers pin
WAL; all entry, root, mount, identity and history rows remain retained. Refresh
does not VACUUM or checkpoint. SQLite may checkpoint on final connection close;
that global work is outside local-refresh benchmark timing. Bounded leases,
reclamation and explicit checkpoint budgets are mandatory before adoption.

## Experimental format and measurements

The engine is v2, with experimental SQLite `user_version = 3` for the
root-scoped/inode metadata layout. Prior experimental revision 2 stores must be
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

Experimental staging constructs raw paths and order keys per entry, maps raw IDs
and inode metadata, and clones entries during its bottom-up fold. Stable ID
assignment builds a subtree-sized map. Refresh collects old subtree rows and
loads only touched inode groups; the ancestor map contains only the union of
affected paths. Readers materialize requested result sets; raw diff uses a
path-keyed map. Full scan materializes the full seed. These allocations do not
change the production v1 scanner.

## Remaining release dependencies

- All existing query adapters, headers, filters, top/find/pass ordering and
  limits, public diff semantics and invalidation. The raw v2 diff includes scan
  timestamps and is not API-compatible with v1 diff.
- Reader leases, bounded version/identity/history reclamation and explicit WAL
  checkpoint budgets, with long-lived readers and repeated tiny refreshes.
- Crash/process-kill/ENOSPC testing at staging, indexes, publication and cleanup;
  broader writer contention and live boundary-race stress.
- Full v1 semantic corpus parity, target root/mount filesystem fixtures and
  pathological H/A fanout timings.
- Long-run disk, RSS, reader latency, physical I/O and dirty-page measurements;
  comparisons against v1 and agreed numerical target-machine budgets.

The unsupported hardlink/mount restriction from the first slice has been removed.
These production gates still prevent marking issue #10 complete.

## Measured revision-3 evidence (2026-10-26)

Linux 5.15.0-136-generic, ext4 on `/`, Intel Core i7-4770S (8 visible logical
CPUs), one walker worker, release profile, warm fixtures without cache dropping.
Other host work was not controlled. Five repetitions per size each run a full
v2 scan, an uninstrumented v2 refresh, then a separate instrumented v2 refresh.
The target is one directory with one 32-byte file; unrelated files are direct
siblings, exercising a wide ancestor. Every operation publishes a generation.

| Unrelated files | v2 full scan median ms | v2 refresh median ms | SQL row changes | VM instructions | WAL growth median bytes |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 23.802 | 5.324 | 18 | 1,321 | 103,000 |
| 1,000 | 162.226 | 3.946 | 18 | 1,321 | 107,120 |
| 10,000 | 1,598.963 | 4.574 | 18 | 1,321 | 111,240 |

[Raw runs and stage timings](../benchmarks/refresh-v2-locality.csv) contain all
15 repetitions. Compared with the earlier experimental layout, mutation counts
now include versioned root metadata. WAL growth includes index/page maintenance
and is not constant. No cold-cache, RSS, device-write, dirty-page, v1 timing or
pathological fanout timing result is claimed. Checkpoint/connection-close cost
and unbounded retained history remain outside the benchmark window.

A separate deterministic locality test deletes an owner and transfers attribution
outside the refreshed subtree while growing unrelated inode groups from 10 to
1,000. It asserts one touched group, two known members, two deduplicated ancestors,
equal SQL mutation counts and no linear increase in VM instructions. This proves
only that fixture's unrelated-group isolation; H/A fanout measurements remain.

Validation for this slice: 134 core tests passed, strict clippy and formatting
passed, and the explicit release benchmark passed. Seven new real-filesystem
v1/v2 parity/locality tests cover cross-boundary and cross-root ownership,
unknown aliases, prior zeroing/recovery, ordering and rollback. Three new unit
tests cover scoped root/mount/denied generations, local mount replacement and
concurrent stale-base rejection. The broader production gates above remain open.
