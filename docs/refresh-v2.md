# Local refresh with the opt-in v2 store

The v2 backend stores stable entry identities and versioned generations in
SQLite. A local refresh replaces the selected subtree and updates its ancestors
and affected hardlink groups. It does not load or rewrite unrelated entries.
Full scans support `--low-memory`, using disk staging and a depth stack.
Earlier revision-4 runs traded slower full scans and larger retained storage
for local updates; see the [historical CLI comparison](benchmarks.md#complete-cli-refresh-with-v2-2026-10-26).
The current revision-5 layout normalizes immutable paths and uses compact keys;
its storage and performance results are pending container verification. Enable
disk staging on machines with limited RAM and budget for database/WAL space as
well as the temporary staging file.

V1 remains the default. Select v2 explicitly and create a new full scan; v1
snapshots are not converted. V2 uses `<snapshot_dir>/v2/generations.db` with its
own state and locks. Existing v1 files remain available with `--backend v1`.
Both the CLI and HTTP server use the selected backend and existing query shapes.

## Start using v2

Build or install a binary containing the v2 backend, then select it for the
timer and grant the invoking user read access:

```sh
sudo freespace --backend v2 install-timer --no-start
sudo freespace scan --low-memory
freespace status
freespace ls /home
sudo freespace refresh /home/example/Downloads --json
```

`install-timer` saves `backend = "v2"` in the existing config. A healthy v2 store
does not trigger another scan when installation is repeated. To use disk staging
for scheduled scans, retain existing roots and schedule settings and add:

```toml
backend = "v2"
low_memory = true
keep_previous = true
```

For a temporary selection, pass `--backend v2` to each command. The override
also applies to `gui`, `serve`, `doctor`, and `install-timer`. To compare existing
v1 data, use `freespace --backend v1 status` or another query command. Changing
backend does not import or refresh the other backend's data.

An unreadable directory is recorded as incomplete. Scan and refresh JSON retain
the complete denied-path list; ordinary query/status headers show at most 100.
Disk-backed scans spool the complete report instead of retaining it in RAM.

## Upgrade an earlier experimental store

The current v2 format is experimental SQLite revision 5. It rejects revision 4
and earlier stores; running `scan` against the old file does not migrate it.
Use a new state directory so the previous database, WAL, generation state and
locks remain together for rollback.

Before installing the new binary, retain the old binary and a copy of your
existing configuration. Stop scheduled scans and servers, and let an active
scan finish before replacing the binary. Then select a new absolute `snapshot_dir` while keeping
your roots and schedule settings. For a standard system installation:

```sh
sudo freespace --backend v2 install-timer \
  --snapshot-dir /var/lib/freespace-v2-r5 --no-start
sudo freespace scan --low-memory
freespace status
```

This writes the new store at
`/var/lib/freespace-v2-r5/v2/generations.db`; the old state directory is retained.
The installer updates the configured location and service units together.
`--no-start` suppresses its immediate scan, so run the explicit full scan before
using the new store. The first scan has no previous generation for `diff`; a
second full scan establishes one. Do not copy the earlier database into the new
directory or edit its revision number.

For rollback, restore the retained binary and prior configuration, then run
that binary's `install-timer --no-start` to regenerate units for the old state
directory. Its queries can use the retained old store. Keep both directories
until you have chosen which installation to retain. `--backend v1` selects v1
snapshots in the configured directory; it does not select an earlier v2 format.

## Revision-5 storage tradeoffs

The store keeps immutable paths and compact traversal keys in the identity table
and its lookup indexes, removing copies from current and historical metadata.
Historical child indexes retain short names instead of full absolute
keys. Local refresh traverses only current descendants, so retained deleted
paths do not expand that traversal. Disk staging avoids empty child queries for
files and other leaves.

Historical metadata and child names still consume space. Queries join identity
data to metadata; current refresh sorts its selected subtree, and historical
queries can inspect obsolete retained records. Maintenance remains explicit.
See the [storage design](designs/refresh-v2-migration.md#storage-and-api-contract)
for access paths and remaining costs. Revision-5 measurements are pending;
no size, speed or peak-memory improvement is claimed yet.

## Generations and refresh

A full scan publishes a new current generation and makes the former current its
previous generation. Refresh changes current and preserves prev, so diff still
compares with the preceding full scan. With `keep_previous = false`, a full scan
removes the public comparison generation; obsolete stored versions become
eligible for explicit maintenance.

Queries pin a coherent current/prev pair. Readers already running can finish
against their old pair while another process publishes. If another writer
publishes during refresh's filesystem walk, the stale refresh fails with
`ROTATED`; retry the command against the new generation. Failed writes roll back
the entries, generation pair, metadata, and refresh history together.

V2 groups its timing stages differently from v1: `walk_seconds` covers filesystem
preparation, and `insert_seconds` covers the SQLite write transaction and
publication. Aggregation occurs during preparation or the transaction depending
on the scan profile. Zero values indicate phases without a separate timer.
Compare complete command time as well as the report. `snapshot_bytes`
includes the database and WAL; shared-memory and lease files are excluded.
The writer closes before the final size and `total_seconds` are measured, so a
last-writer checkpoint and state-file update are included in that total.

## Reclaim old versions

Maintenance is explicit; refresh does not perform a whole-store rewrite or hide
an automatic checkpoint. Run a bounded batch and optionally request a checkpoint:

```sh
sudo freespace maintain --budget 10000 --checkpoint --json
```

`--budget` limits examined records to 1–100,000 per batch. A checkpoint can perform
additional global WAL work. The report distinguishes active-reader deferral
(`readers_blocking`) from SQLite checkpoint contention (`checkpoint_busy`).
Close long-running readers and retry when necessary. Repeated batches reclaim
obsolete versions while preserving current, prev, and their required histories.
Deleting rows does not shrink the main database's allocated high-water space.

Admission defaults are 100 million logical retained records and a 512 MiB WAL
threshold. Logical records include identities and versions, so entry count is
not the retained-record count. The WAL threshold admits at most one additional
transaction; it is not a hard disk-space cap. Row/index sizes, staging, retained
generations, and filesystem cache require separate capacity planning.

Explicit policy changes persist even when maintenance is deferred or fails:

```sh
sudo freespace maintain --max-records 120000000 --max-wal-bytes 1073741824 --json
```

If protected history itself fills the record budget, the error reports numeric
temporary headroom. Close old readers and try maintenance first. If needed,
raise the budget explicitly, retry the failed operation, and run the reported
full scans. After a refresh retry, two full scans are required: the first retains
the old current as prev; the second retires it. Reclaim retired records before
restoring the lower budget. A growing tree can require a new estimate. Required
history is never silently truncated.

## Verification and measurements

The migration design and remaining measurement qualifications are recorded in
[refresh-v2-migration.md](designs/refresh-v2-migration.md). Reproduce a comparison
with the same release binary and a fixed small subtree:

```sh
BACKENDS="v1 v2" STEPS="1000 10000 100000" REPEATS=3 \
  SOURCE_REVISION=$(git rev-parse HEAD) \
  scripts/bench-refresh-memory.sh target/release/freespace ./bench-results
```

The harness checks report counts, alternates backend order, and records complete
process time, peak RSS, endpoint disk occupancy, and kernel block-operation
counters. These counters are not measurements of physical device bytes. Cache
state and other host activity affect latency. See [benchmarks.md](benchmarks.md)
for source hashes, measured results, and limits of the evidence.
