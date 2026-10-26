# Benchmarks

Measured on the machine freespace was built for (the design's primary target), 2026-10-23. Raw rows come from `scripts/bench-baseline.sh` (scans) and `scripts/bench-queries.sh` (queries); both print TSV.

## Machine

| | |
|---|---|
| CPU | Intel Core i7-4770S as 8 vCPUs (4 physical cores with Hyper-Threading), VMware guest |
| RAM | 15 GB |
| Disk | 450 GB VMware virtual disk, rotational per the kernel (`ROTA=1`, mq-deadline), ext4, 97 % full |
| Tree | `/`: 7.72 M entries, 407 G allocated (6.4 M entries under `/home`) |
| Kernel | Linux 5.15 (Ubuntu 22.04) |

"Cold" runs follow `sync; echo 3 > /proc/sys/vm/drop_caches`. "Warm" runs follow the cold run immediately, but 7.7 M inodes do not all fit in the dentry and inode caches next to everything else in 15 GB, so a warm full scan still reads much of the metadata from disk (see gdu's warm row).

## Scan

All tools scan `/` on one filesystem (`-x` or the default), as root, at normal priority. This first table uses the packages of the machine's Ubuntu 22.04 base; the next section repeats the comparison with current releases.

| Tool | Config | Cold | Warm | Peak RSS |
|---|---|---|---|---|
| du 8.32 | `-sx` | 1199 s | 891 s | 9 MB |
| duc 1.4.4 | `index -x` | 1206 s | 1046 s | 29 MB |
| ncdu 1.15.1 | `-x -0 -o` | 615 s | 847 s ¹ | 3 MB |
| gdu 5.13.2 | `-x -m 8` | 286 s | 318 s | 3.3 GB |
| gdu 5.13.2 | `-x -m 1` | 956 s | | 2.9 GB |
| **freespace** (glibc build) | `scan`, 8 workers (the default) | **239 s** ² | 434 s ¹ | 1.42 GB |
| freespace | `scan --workers 4` | 379 s, 657 s ³ | | 1.41 GB |
| freespace | `scan --workers 2` | 725 s | | 1.40 GB |
| freespace | `scan --workers 1` | 1234 s | | 1.40 GB |

¹ Overlapped a compile on the same machine.
² Idle rerun. The first 8-worker run, during a compile, took 576 s. Of the 239 s: walk 198 s, finalize 1.8 s, SQLite insert 11 s, indexes 28 s.
³ Two runs of the same binary on an idle machine: the variance of this disk.

freespace's time includes writing the snapshot that every later answer reads; du and gdu only print. On the default settings freespace is 5× faster than `du` cold and on par with this gdu, at under half of its memory. Current releases of ncdu and gdu walk as fast or faster with far less memory (below).

Snapshot: 1.05 GB for 7.77 M entries, 135 B per entry with indexes (before the first snapshot exists, the free-space precheck assumes 150 B per entry). Peak RSS of the scan is ~183 B per entry.

### Current releases

The same cold scan of `/` (7.78 M entries) with the latest release of each multi-threaded tool, from its project's own static build: ncdu 2.9.1 (signature checked), gdu 5.38.0 and dust 1.2.6 (sha256 checked), against the installed freespace 0.1.1 release (static musl with mimalloc). Two rounds on an idle machine, the second in reverse order (ncdu, gdu, dust, freespace, freespace, dust, gdu, ncdu), so a slow stretch of the disk does not land on one tool:

<!-- no-run -->
```sh
sudo env TOOLS=gdu WARM=0 GDU_CORES=8 GDU=./gdu scripts/bench-baseline.sh /   # one tool, one cold run
```

| Tool | Config | Cold, round 1 | Cold, round 2 | Peak RSS |
|---|---|---|---|---|
| ncdu 2.9.1 | `-x -0 -t 8 -O FILE` (binary export) | 287 s | 324 s | 9–11 MB |
| gdu 5.38.0 | `-n -p -s -x -m 8` | **214 s** | 303 s | 53–54 MB |
| dust 1.2.6 | `-x -T 8 -d 1` | 484 s | 534 s | 2.78 GB |
| freespace 0.1.1 | `scan --workers 8` | 288 s (walk 241 s) | 399 s (walk 354 s) | 1.49–1.52 GB |

- The walk is not where freespace wins. ncdu 2 and gdu are within this disk's noise of it, and gdu was the fastest in round 1. All of them wait on the same metadata reads.
- freespace's time includes writing the 1.05 GB snapshot with its indexes, 47 s and 45 s here. That is what makes every later answer take a second instead of a walk.
- Memory: ncdu streams its export and gdu `-s` evidently keeps only totals (53 MB for 7.8 M entries), so they need megabytes; their interactive browsers keep the tree in memory (not measured here). freespace holds the tree until the snapshot is written, ~190 B per entry; dust holds it at ~360 B.
- du and duc walk with one thread, so a newer release would not change their rows above.

### Workers (Open Question 3)

The default stays `min(CPUs, 8)`. One worker takes 1234 s and two 725 s: although the kernel reports the virtual disk as rotational, it serves parallel metadata reads. Between 4 and 8 workers the difference is inside this disk's noise (the same 4-worker binary took 379 s and 657 s), and the idle 8-worker run was the fastest of all. A slower disk throttles the walk on its own, so there is no reason to cap the default lower.

### Allocator (release builds)

The release archives are static musl builds. musl's malloc takes a global lock on every call, and the walker's threads allocate per directory. An 8-worker walk of a cached 580 k-entry `/usr`:

| Build | Walk |
|---|---|
| glibc | 0.95 s (1 worker: 3.8 s) |
| musl malloc | 6.9 s, slower than 1 worker (4.95 s) |
| musl + mimalloc | 2.6 s |
| musl + mimalloc replacing malloc for C code too (`override`: opendir, SQLite) | 1.2–1.35 s |

The release builds use the last row (`cli/Cargo.toml`); the full test suite also runs on that target in CI. Full scans of `/` with musl malloc before the fix: 287 s (8 workers), 382 s (4).

## Queries

`scripts/bench-queries.sh` on the 7.77 M-entry snapshot, every query with `--json`, cold then warm. "Before" is one thread reading the subtree in SQL (commit 864d3dc); "after" is the shared pass in `core/src/queries/pass.rs`: it splits the subtree's id range across up to 8 threads, each on its own read-only connection, and reads a selective `--min-size` through the size index.

| Query | Target | Before, cold / warm | After, cold / warm | Peak RSS after |
|---|---|---|---|---|
| `status` | ≤ 50 ms | 0.06 / 0.00 s | 0.09 / 0.00 s | 8 MB |
| `ls /` | ≤ 300 ms | 0.10 / 0.00 s | 0.04 / 0.00 s | 7 MB |
| `diff /` | ≤ 300 ms | 0.04 / 0.00 s | 0.04 / 0.00 s | 7 MB |
| `top /` | ≤ 300 ms warm | 4.78 / 2.27 s | 1.31 / **0.68 s** | 271 MB |
| `top / --min-size 1G` | ≤ 300 ms warm | 2.91 / 1.73 s | 0.12 / 0.00 s | 12 MB |
| `top / --kind dir --older-than 180d` | ≤ 300 ms warm | 2.48 / 1.34 s | 1.14 / **0.39 s** | 279 MB |
| `top / --kind dir --older-than 180d --min-size 100M` | ≤ 300 ms warm | 2.13 / 0.84 s | 0.48 / 0.02 s | 28 MB |
| `find / --name '*.iso'` | 1–3 s warm, ≤ 5 s cold | 2.93 / 1.71 s | 1.27 / 0.56 s | 271 MB |
| `find / --name '*'` | 1–3 s warm, ≤ 5 s cold | **6.38** / 4.94 s | 1.63 / 1.15 s | 273 MB |
| `find / --older-than 1y` | | 2.71 / 1.58 s | 1.20 / 0.50 s | 264 MB |
| `find / --kind dir --name node_modules` | | 2.39 / 0.96 s | 1.17 / 0.41 s | 283 MB |

Most of the peak RSS is the snapshot's first 256 MB mapped into memory (`mmap_size`), file pages shared with the page cache.

Decisions:

- **No name index (R8, D11).** The rule was: add the covering index `(id, name, kind, allocated, apparent, mtime, subtree_end)` if `find --name` takes more than 5 s cold. Before the change `--name '*'` did (6.38 s). But `'*.iso'` reads the same 7.77 M names and matches almost none of them in 2.93 s, so the extra time was spent per match, which a narrower index does not remove. After the change both are under 1.7 s cold, so schema v1 ships without the index and the per-entry estimate stays where it is.
- **No `(kind, newest_mtime)` index** (eng review run 2). Untouched folders take 1.14 s cold and 0.39 s warm; with `--min-size` the size index answers in 0.02 s.
- **`--min-size` through `entries_alloc` up to 8192 entries.** The index trades a sequential pass for random reads. Measured with the sqlite3 shell on the cold cache: 2.5 k entries over the size take 0.48 s, 12.6 k take 1.66 s, 70 k take 5.0 s; the sequential pass takes ~1.2 s. Above 8192 the pass is used.

**Missed: `top /` warm, 0.68 s against 300 ms** (untouched folders, 0.39 s, are close). The answer carries a total over all 5.9 M files, so every row of the subtree is read. On one core that is 2.0 s, the same as SQLite's own `count`/`sum` plus `ORDER BY … LIMIT` (1.12 s + 1.13 s), so the Rust side adds nothing to remove. Eight threads on four physical cores give 2.9×. Lock contention was ruled out: building SQLite without `SQLITE_ENABLE_MEMORY_MANAGEMENT` and memory statistics, which removes its global page-cache and malloc mutexes, changed nothing (0.69 s vs 0.71 s). The cold target (≤ 2 s) holds everywhere. Meeting the warm one needs per-folder file totals in the snapshot, a schema change (TODOS.md).

## Refresh (Open Question 5)

`refresh` writes a whole new snapshot with one subtree rescanned (dense preorder ids leave no in-place update, OV-1), so most of its cost does not depend on the subtree:

| Subtree | Entries in it | Load the old snapshot | Walk | Insert + indexes | Total | Peak RSS |
|---|---|---|---|---|---|---|
| `/etc` | 4.7 k | 51.5 s (cold) | 0.1 s | 10.6 s + 26.9 s | 91.3 s | 1.77 GB |
| `/usr` | 582 k | 6.7 s (warm) | 34.0 s | 10.4 s + 26.7 s | 80.4 s | 1.84 GB |

The design estimated tens of seconds for a 2 M subtree. On 7.77 M entries the floor is ~45 s warm, ~90 s cold, whatever the subtree; building the indexes is the largest part.

## Time to first answer (T48)

A clean start on this machine: freespace uninstalled, caches dropped, then the real install path, `curl -fsSL https://raw.githubusercontent.com/mikhailsal/free-space/master/install.sh | sh` with v0.1.1 from GitHub Releases, waiting for the first scan through the unit, and the first `freespace ls / --json`.

| Step | Time |
|---|---|
| Install: download, checksum, binary, completions, timer, ACL, skill, smoke checks | 4.4 s |
| First scan through the unit (`/` and `/boot/efi`, 7.8 M entries, idle CPU and I/O priority) | 271.6 s (walk 208 s, write 42 s) |
| First answer (`ls /`) | 0.01 s |
| **Total** | **276 s (4.6 min)** |

The release build (static musl with mimalloc) at idle priority took 272 s, against 239 s for the glibc build at normal priority in the scan table.

The first run of this measurement, on v0.1.0, found a bug. The unit's scan of `/` took 27 s because it found 1.4 M entries and skipped `/home`: `ProtectSystem=strict` mounts `/home` onto itself inside the unit, and the walk stopped at that mount. 0.1.1 fixes it, and CI now reproduces the sandbox.

## Caveats

- One machine, one run per row. Differences under ~10 % are noise; the earliest freespace rows overlapped with a compile on the same machine and were rerun.
- The warm column is not a fully cached run (see above).
- "Cold" means the guest's caches are empty. The VMware host may still cache the virtual disk, which is how a cold query reads the 1 GB snapshot in about a second while a cold scan's metadata reads take minutes.
- The nightly unit runs with `Nice=19` and `IOSchedulingClass=idle`, so on a busy machine it takes longer than these normal-priority rows; that is the point of idle priority.

## Synthetic refresh and memory scaling harness (#10, #11)

`scripts/bench-refresh-memory.sh` measures a full scan and a refresh of a fixed
small folder while only an unrelated sibling grows. It needs Bash, Python 3,
GNU `/usr/bin/time`, and Linux `stat`; it runs as an ordinary user and does not
change cache settings, mounts, memory limits, swap, or system services.

Build a release binary once, then run the default smoke workload:

```sh
cargo build --release -p freespace
scripts/bench-refresh-memory.sh ./target/release/freespace ./bench-results
```

For a larger series with three scan/refresh pairs per size:

```sh
STEPS="10_000 100_000 1_000_000" WORKERS=8 REPEATS=3 SMALL_FILES=16 \
  FILES_PER_DIR=1000 TMPDIR=/path/to/scratch \
  scripts/bench-refresh-memory.sh ./target/release/freespace ./bench-results
```

`BINARY` may also name an executable on `PATH`. `OUTPUT_PARENT` receives a new
unique `refresh-memory-*` directory per invocation; existing results are never
overwritten. `TMPDIR` selects the scratch filesystem (default: the Python
system temporary directory). The harness removes only its own newly created
scratch directory when it exits normally or encounters a Python exception.
Termination by SIGTERM or SIGKILL can leave that directory behind. Results persist.
Set `TMPDIR` to the disk being studied; `/tmp` may be tmpfs, which changes both
I/O and system memory use. Keep results on a filesystem with adequate free space.

Defaults are `STEPS="100 1000"`, `WORKERS=2`, `REPEATS=1`, `SMALL_FILES=16`, and
`FILES_PER_DIR=1000`. All are positive integers; steps must increase strictly.
Python-style underscores in step values are accepted for readability.
`BACKENDS` defaults to `v1`, so the default workload also accepts older binaries.
For a binary supporting v2, compare both backends on the same tree:

```sh
BACKENDS="v1 v2" STEPS="1000 10000 100000" REPEATS=3 \
  SOURCE_REVISION=$(git rev-parse HEAD) \
  scripts/bench-refresh-memory.sh ./target/release/freespace ./bench-results
```

Each backend has its own snapshot directory. Their order alternates between
repetitions; both see identical filesystem contents. The manifest retains the
supplied source revision and binary hash. GNU time covers the entire process,
including SQLite close and any close-time checkpoint, which can be outside the
CLI stage timers. Its input/output counters are kernel block-operation counts,
not measured physical device bytes. Endpoint disk occupancy includes WAL and
retained files; it is not peak temporary space. Stage timings use each backend's
documented grouping and should not be compared as identical implementations.

Steps count **unrelated files**, not total entries. Files are empty, with fixed
names and mtimes, grouped into numbered directories with at most
`FILES_PER_DIR` files each. The scanned root contains `small/` and `unrelated/`;
only `unrelated/` grows between steps. The small subtree stays unchanged across
all runs. Snapshot and config directories are outside the scanned tree. The
harness supplies its own config with `keep_previous=true` and overrides inherited
`FREESPACE_CONFIG` and `FREESPACE_SNAPSHOT_DIR`.

Each repetition runs a full scan immediately followed by a refresh of `small/`,
with identical workers. The snapshot directory is reused across steps and
repetitions, so later scans rotate a previous generation. Every successful row
is checked for the expected total entry count, a complete scan, and the fixed
refresh subtree count. A failed command or invalid report stops the series with
a nonzero exit; inspect the retained artifacts before analyzing partial results.

The result directory contains:

- `manifest.json`: binary path, version and SHA-256, platform, filesystem type,
  initial `/proc/meminfo`, GNU time version, workload settings, scratch paths,
  and cache/snapshot policy. Record the binary's source revision and build flags
  separately; its version string and hash do not identify those automatically.
- `results.jsonl`: one JSON object per operation with step, repetition, workers,
  exact argument vector, exit status, validation error (normally null), GNU time
  metrics, endpoint snapshot occupancy, and the original CLI JSON as `report`.
- `files-<step>-r<repeat>-<scan|refresh>.json`, `.stderr`, and `.time`: raw CLI
  report, diagnostics, and GNU time output, including failures.

For example, extract the core measurements without extra dependencies:

```sh
python3 - ./bench-results/refresh-memory-EXAMPLE/results.jsonl <<'PY'
import json, sys
for line in open(sys.argv[1]):
    row = json.loads(line)
    report = row['report'] or {}
    print(row['operation'], row['unrelated_files'], report.get('entries'),
          report.get('subtree_entries'), report.get('total_seconds'),
          row['time']['peak_rss_kib'])
PY
```

The nested CLI report retains `walk_seconds`, `finalize_seconds`,
`insert_seconds`, `index_seconds`, `publish_seconds`, `total_seconds`, and, for
refresh, `load_seconds`. GNU time adds process wall seconds and whole-command
peak RSS in **KiB**. It also records filesystem input/output counts (`%I`, `%O`),
which are kernel accounting units, **not byte counts**. Endpoint
`snapshot_file_bytes` and `snapshot_allocated_bytes` include retained generations
and bookkeeping files; they do not measure peak temporary disk occupancy.

These runs have uncontrolled caches: file creation warms metadata and refresh
follows scan. Compare distributions across repetitions and separate invocations
under comparable machine load; small smoke timings are dominated by overhead.
Empty files isolate entry-count costs but do not represent realistic payloads,
hardlinks, permission errors, deletion, or changing subtree contents. The harness
measures phase **timings**, not phase-specific RSS; GNU time cannot attribute its
single process peak to walk/finalize/write/index phases. RSS excludes page cache
and other system/cgroup memory and cannot demonstrate a memory budget or OOM
safety. A separately configured VM/cgroup and phase-aware profiling are needed
for those acceptance checks. This infrastructure supplies measurements for #10
and #11 and makes no speed or memory improvement claim.

### Releasing scan data before index creation

A local glibc release build on 2026-10-26 compared baseline `5a280da` with
ownership transferred to the snapshot writer. The writer drops `Tree` and
`Finalized` after inserting all tables, before `CREATE INDEX`. SQLite settings,
schema, transaction boundaries, and publication order are unchanged. Report
entry/error counts are captured before transferring ownership.

Two invocations per binary used the synthetic harness above with
`STEPS="10000 100000" REPEATS=3` and its default 2 workers, 16 small files,
and 1000 files per directory. Order was baseline, candidate, baseline,
candidate. The host ran Linux 5.15, glibc 2.35, on ext4, with uncontrolled
caches and other host activity. Each cell below is the median of six runs;
RSS is KiB and time is seconds. These are whole-command peaks, not stage RSS.

| Entries | Operation | Baseline peak RSS | Candidate peak RSS | Baseline wall | Candidate wall |
|---|---|---|---|---|---|
| 10,030 | scan | 11,242 | 10,294 | 0.110 | 0.110 |
| 10,030 | refresh | 11,648 | 10,478 | 0.105 | 0.115 |
| 100,120 | scan | 36,334 | 26,456 | 0.625 | 0.770 |
| 100,120 | refresh | 45,012 | 34,900 | 0.510 | 0.610 |

Median full-scan stage timings from the same runs. These measurements used
an earlier candidate whose insert timer included releasing the tree and
finalized arrays. The current writer records insert time before that release,
preserving the baseline timer boundary; deallocation is included only in total
time, between the insert and index timers. The historical candidate insert
numbers below and in the follow-up therefore include that extra work. RSS and
wall measurements cover the whole command and do not depend on timer placement.

| Entries | Build | Walk | Finalize | Insert | Index | Publish | Total |
|---|---|---|---|---|---|---|---|
| 10,030 | baseline | 0.029340 | 0.000696 | 0.013702 | 0.023299 | 0.001059 | 0.107021 |
| 10,030 | candidate | 0.029866 | 0.000581 | 0.011065 | 0.020194 | 0.001796 | 0.112108 |
| 100,120 | baseline | 0.246661 | 0.006558 | 0.121165 | 0.197558 | 0.001913 | 0.618723 |
| 100,120 | candidate | 0.276694 | 0.008566 | 0.158670 | 0.248005 | 0.002714 | 0.767135 |

The 100k scan peak falls 27.2%; refresh falls 22.5%. Timings in these separate
harness runs are slower for the candidate. A follow-up used one unchanged
100,101-entry tree (100 numbered directories, 1000 empty numbered files each),
2 workers, and alternating baseline/candidate/candidate/baseline order three
times, with separate snapshot directories. It confirmed the RSS reduction:
median 34,300 to 24,528 KiB (28.5%). Both binaries' wall times ranged from 0.61
to 0.71 s; medians were 0.61 and 0.66 s. Median insert time was 0.116979 vs
0.132916 s, index time 0.210195 vs 0.209731 s, and total time 0.608336 vs
0.657334 s. This is a memory improvement, with no speed improvement claim.
The results do not establish a memory budget or predict the musl/mimalloc
release build's peak; larger realistic trees and controlled hosts need their
own measurements.

### Releasing refresh-only data before writing

A second local glibc comparison used `3a8e80c` (which already releases the
written tree before indexes) as the baseline. After grafting and finalization,
refresh no longer needs the loaded snapshot-id map, old hardlink rows, or
hardlink policy. After copying refresh metadata, it also no longer needs the
old snapshot connection and its mapping/cache. Explicitly releasing all four
before calling the writer further reduces the refresh peak.

The same host and 2-worker configuration refreshed an unchanged 17-entry
`small/` subtree beside 100 directories containing 1000 empty files each:
100,119 total entries. Separate snapshot directories were initialized with the
baseline binary. Order was baseline/candidate/candidate/baseline repeated
three times, giving six refreshes per binary on the same tree. Every report
was checked for total and subtree entry counts. Medians follow (RSS in KiB,
times in seconds); caches and host load were uncontrolled.

| Build | Peak RSS | Wall | Load | Walk | Finalize | Insert | Index | Publish | Total |
|---|---|---|---|---|---|---|---|---|---|
| baseline | 31,630 | 0.725 | 0.115667 | 0.001581 | 0.008923 | 0.183436 | 0.318202 | 0.005315 | 0.720375 |
| candidate | 25,802 | 0.765 | 0.144132 | 0.004616 | 0.010552 | 0.197075 | 0.331633 | 0.005917 | 0.759816 |

Peak RSS falls another 18.4%, with nonoverlapping ranges: 31,484–31,784 vs
25,480–26,008 KiB. Wall ranges overlap (0.61–0.82 vs 0.71–0.83 s), with no
speed improvement claim. This retains the timing boundaries described above:
refresh-only releases happen outside load/finalize/insert/index timers and
are included in total time. This workload contains no hardlinks, so it does
not quantify the benefit from releasing old hardlink rows or policy maps.

D18 allocation inventory against master `47a76ac`: no new `clone()`,
`collect()`, or allocation inside a per-entry loop. The existing root-error
sum moves before ownership transfer without changing its iteration or
allocations. Writer insert loops and refresh graft/finalize loops are
unchanged. The new report counters are scalars; transferring `Tree` and
`Finalized` moves their existing buffers without copying them. Explicit drops
release existing allocations and the old snapshot connection.

## Disk-backed full scan

`scan --low-memory` (or config `low_memory = true`) streams individual entries and
pending directory jobs to SQLite in the snapshot directory. It never retains the
whole tree or an entire directory listing. An indexed DFS writes schema v1 rows,
with one aggregate frame per traversal depth. Hardlink ownership still follows
full byte-path order. Denied-path JSON is spooled and streamed without truncation.

The mode uses one walker, overriding `workers`. Each of the two SQLite connections
used during final writing has an 8 MiB cache target, with mmap disabled. Indexes
are maintained from empty tables so index construction does not require a large
sort. This is a working-set design, **not a hard process memory limit**. SQLite
cache targets, allocator overhead, path depth, root/mount configuration and kernel
page cache all affect memory. Use a cgroup when an operating-system limit matters.

Temporary space includes the staging database and final snapshot simultaneously,
plus any retained generations. Long directory paths and many hardlinks increase
staging size. The initial space check uses the existing snapshot size; ongoing
reserve checks cover staging and output. Interrupted temporary databases are
removed on the next scan. The previous snapshot remains published until the new
one is closed, fsynced and atomically renamed. Keep benchmark snapshots outside
the fixture tree, and use a disk-backed filesystem rather than tmpfs.

Finalization and final-row insertion are combined: `finalize_seconds` is zero in
this mode; `insert_seconds` includes DFS aggregation and incremental index
maintenance; `index_seconds` covers final analysis/closing. These phase timings
are not directly comparable with the fast engine's separate finalize/index stages.
`refresh` continues to use the existing in-memory engine.

Build once and compare increasing tree sizes:

```sh
cargo build --release -p freespace
STEPS="10000 100000 1000000" REPEATS=3 TMPDIR=/disk/scratch \
  scripts/bench-scan-memory.py target/release/freespace ./bench-results
```

On a Linux host with a systemd user manager and memory-controller delegation,
measure each run in a fresh cgroup with swap disabled:

```sh
STEPS="100000 1000000" MEMORY_MAX=128M TMPDIR=/disk/scratch \
  scripts/bench-scan-memory.py target/release/freespace ./bench-results
```

The harness creates empty files, scans each size with both engines, checks entry
counts and errors, then removes its own scratch tree. `FILES_PER_DIR` defaults to
1000; setting it above the largest step exercises one very wide directory. Each
run gets a fresh snapshot directory. A failed fast-engine run is retained so its
OOM can be compared with the disk engine; a disk-engine failure stops the run.

Results include a binary hash, host/configuration metadata, raw stdout/stderr,
JSON reports, elapsed time, process peak RSS (KiB), sampled peak snapshot-directory
disk occupancy, and constrained-run `memory.peak` when available; otherwise
sampled `memory.current`. Swap, OOM/limit-event counters and available memory
pressure statistics are also recorded. Set `SOURCE_REVISION` to the revision used
to build the supplied binary to retain it in the manifest.
The measurement helper shares the cgroup, so its memory is included in cgroup
usage; process RSS is the scanner's own peak. Sampling every 20 ms can miss brief
disk peaks. This fixture has no denied paths, so the unlinked report spool does
not contribute material disk usage. The harness opts into `FREESPACE_MEMORY_PROFILE`
markers for walk, finalize, write and index phases. Each boundary records RSS and
cumulative VmHWM; periodic 20 ms RSS samples are attributed to the active phase.
`phase_rss.observed_peak_rss_kib` is the largest boundary/periodic sample, a lower
bound on the true phase peak. Short phases can have no periodic samples; their
start/end RSS still supplies phase evidence. VmHWM remains cumulative and is never
presented as a phase peak. Low-memory finalization and incremental index updates
are fused into write; its index phase covers final analysis/closing. Failed runs
mark the last phase incomplete. Cache state is uncontrolled and fixture creation
warms metadata. Record source revision, filesystem, load and repetitions alongside
results before generalizing to a VPS or Raspberry Pi.

### Constrained wide-directory result (2026-10-26)

Release build of final integrated source `6755725` (including master `8dbb8d4`,
the hardlink/spool review fixes and phase tracing), Linux 5.15 x86_64/glibc, ext4,
one run per case, one walker, empty files in a single wide directory. Each scanner
and its measurement helper ran in a fresh **128 MiB cgroup with swap disabled**.
The snapshot directory and phase trace were outside the tree.
[Machine-readable results](benchmark-results/disk-scan-2026-10-26.json) include the
full source/tree hashes, final binary SHA-256, reports, phase boundaries, sampled
phase/cgroup/disk peaks and OOM/limit-pressure counters.

| Entries | Engine | Peak process RSS | Elapsed | Result |
|---:|---|---:|---:|---|
| 100,002 | default | 33.7 MiB | 1.36 s | completed |
| 100,002 | low-memory | 24.6 MiB | 3.07 s | completed |
| 1,000,002 | default | 121.7 MiB | 3.85 s | OOM-killed during walk |
| 1,000,002 | low-memory | 24.9 MiB | 90.10 s | completed |

Observed phase RSS below is the largest boundary/20 ms sample, **not an exact
phase peak**. It can be lower than the whole-command peak above. The JSON retains
boundary values, durations and periodic sample counts so short phases can be
identified. `finalize` is part of `write` in low-memory mode; that engine's `write`
also maintains indexes incrementally, while `index` performs final analysis and
closing. Default-mode phase boundaries include releasing the tree between insert
and index timing, so profile durations need not equal the CLI's phase timers.

| Entries | Engine | Walk | Finalize | Write | Index |
|---:|---|---:|---:|---:|---:|
| 100,002 | default | 29.5 MiB | 21.1 MiB | 28.5 MiB | 26.8 MiB |
| 100,002 | low-memory | 14.6 MiB | fused | 24.5 MiB | 24.5 MiB |
| 1,000,002 | default | 121.4 MiB, incomplete | not reached | not reached | not reached |
| 1,000,002 | low-memory | 15.6 MiB | fused | 24.9 MiB | 24.9 MiB |

The million-file low-memory run used approximately **180.3 MiB** of sampled peak
snapshot-directory disk space, including staging and output. **Its cgroup reached
the 128 MiB cap and recorded 1,139 `memory.events` max events**, with zero OOM
or OOM-kill events and zero sampled swap use. Low process RSS therefore does not
mean the cgroup had spare memory: the run completed under limit pressure, with
helper/kernel/filesystem-cache memory included in the cgroup. The default engine
recorded an OOM kill on the same fixture. Linux 5.15 on this host does not expose
`memory.peak`; cgroup peaks are 20 ms samples of `memory.current`, not exact
high-water marks. Available memory-pressure statistics are retained in the JSON.

This verifies one practical constrained workload, including a directory whose
entire contents cannot fit in the default walker's budget. It does not establish
a universal 128 MiB requirement. Repeat on the target filesystem/hardware,
especially for deep paths, many hardlinks or slow storage. At 100,002 entries the
disk engine was about 2.3 times slower; the million-file default scan did not
complete, so there is no comparable full-scan time at that size. Raspberry Pi
hardware measurements remain follow-up work.

### Small VPS and Raspberry Pi deployment

On a 512 MiB–2 GiB machine, enable `low_memory = true` for scheduled full scans.
The option uses a fixed small SQLite working set; it does not reserve memory or
enforce a process limit. Leave capacity for the OS and other services, and verify
completion with a cgroup limit appropriate to the RAM available on that machine.
The 128 MiB test above is one x86_64 workload, not a Raspberry Pi requirement or
a promise of spare memory within that limit. A successful run can still encounter
heavy cgroup memory pressure and reclaim, which affects elapsed time.

Use persistent disk space for `snapshot_dir` and staging, not a RAM-backed tmpfs.
Budget for staging and the new snapshot alongside retained generations; long paths
and many hardlinks increase the temporary space needed. If storage is tight,
`keep_previous = false` reduces retained generations at the cost of historical
comparison. Measure both elapsed time and disk occupancy on the target storage,
and allow scheduled scans to finish before the next interval. `refresh` still
uses the in-memory engine and does not inherit the full-scan memory behavior.
