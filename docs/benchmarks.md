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
CLI stage timers. `time.wall_seconds_monotonic` uses the harness's monotonic clock
and includes GNU time wrapper overhead; GNU time's `%e` has coarser resolution.
Its input/output counters are kernel block-operation counts,
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

## Complete CLI refresh with v2 (2026-10-26)

Release binary from `575044b6d2e9fba3aa7467489ed19afa9088e4e0`, SHA-256
`ee58e9af394599f86457b34debdd9309961f4e90573aefe8b387a39fcd295b7c`.
Linux 5.15 x86_64/glibc, Intel Core i7-4770S, ext4, two walker workers. No other
agent builds or benchmarks ran during this series; other host activity and
cache state were not controlled. The small refresh target contains 18 entries;
only the unrelated sibling grows. Three repetitions per size/backend alternate
backend order. Each repetition scans, then refreshes unchanged data. Timings
include process startup, writer close/checkpoint, state update and the GNU time
wrapper. These are warm synthetic measurements, not cold-disk promises.

```sh
SOURCE_REVISION=575044b6d2e9fba3aa7467489ed19afa9088e4e0 \
  BACKENDS="v1 v2" STEPS="1000 10000 100000" REPEATS=3 WORKERS=2 \
  SMALL_FILES=16 scripts/bench-refresh-memory.sh target/release/freespace ./bench-results
```

| Unrelated files | V1 refresh median | V2 refresh median | V1 refresh RSS | V2 refresh RSS |
|---:|---:|---:|---:|---:|
| 1,000 | 115.6 ms | 29.0 ms | 8,696 KiB | 8,840 KiB |
| 10,000 | 116.1 ms | 31.7 ms | 10,372 KiB | 8,880 KiB |
| 100,000 | 520.3 ms | 29.4 ms | 28,272 KiB | 8,804 KiB |

All 36 scan/refresh reports passed entry/error/incompleteness checks. At 100,000
unrelated files, v2 refresh is about **17.7 times faster** with **69% lower peak
process RSS** in this workload. Its wall-time range is 27.1–30.8 ms, versus
519.4–617.6 ms for v1. Median kernel output-operation counts are 1,512 versus
23,072. Those counters provide OS-level I/O evidence; they are not direct
physical-device byte measurements. The roughly constant v2 refresh time/RSS as
unrelated entries grow supports the locality tests' SQL evidence.

The tradeoff is substantial full-scan and storage overhead. At 100,000 unrelated
files, the ordinary memory-profile v2 scan took 14.79 s and 77,068 KiB RSS versus
0.72 s and 27,252 KiB for v1. Endpoint snapshot-directory allocation was about
300.8 MB for v2 versus 23.6 MB for v1 after repeated scans. V2 retains obsolete
versions until explicit maintenance; this series reused stores across all sizes
and repeats without running maintenance, while v1 retains only current/prev.
These endpoint sizes therefore include different physical retention states.
Use `--low-memory` for a bounded full-scan working set, plan additional disk
capacity, and run maintenance to reclaim eligible records. Local-refresh gains
do not imply faster full scans or smaller databases.

[Raw reports, stage timings and manifest](benchmark-results/refresh-v2-cli-2026-10-26.json)
include source/tree/binary hashes, all repetitions, monotonic/GNU wall times,
RSS, kernel I/O counters and endpoint disk occupancy. Connection-close costs are
included in complete command time; older experimental Store-only timings in the
migration design exclude them and must not be substituted for this result.

### V2 full scan under a 128 MiB cgroup

The same release source/binary above was also tested with `--low-memory`, one
walker, `keep_previous=false`, fresh stores and a fresh **128 MiB, no-swap**
systemd user cgroup per command. Each directory contains up to 1,000 empty files.
Fixture creation is outside the cgroup; the Python measurement helper and file
cache share the scanner's limit. These are separate single runs, with v1/v2 order
at 100k and v2/v1 order at one million. Cache state is uncontrolled. Full process
time includes writer close and checkpoint.

| Files / entries | Backend | Full command | Peak process RSS | Sampled peak DB/stage/WAL allocation |
|---:|---|---:|---:|---:|
| 100,000 / 100,101 | v1 | 2.32 s | 26,116 KiB | 17.9 MiB |
| 100,000 / 100,101 | v2 | 17.26 s | 26,756 KiB | 201.9 MiB |
| 1,000,000 / 1,001,001 | v1 | 65.77 s | 26,740 KiB | 178.3 MiB |
| 1,000,000 / 1,001,001 | v2 | 213.57 s | 29,104 KiB | 2,029.3 MiB |

All four commands completed with exact entry counts, no errors, no OOM/OOM-kill
events and swap disabled. V2 peak process RSS grew from 26.1 to 28.4 MiB across a
tenfold fixture increase. The million-file v2 cgroup recorded 25,685 memory-limit
pressure events and sampled current memory reached the cap with a one-page
transient overshoot. This is completion under cache-reclaim pressure, not spare
memory headroom or a universal process/cgroup-size guarantee.

The kernel lacks `memory.peak`; cgroup current memory and allocated disk space
were sampled every 20 ms and can miss short peaks. On the million-file fixture,
v2 took about 3.25 times as long as v1 and used about 11.4 times the sampled peak
disk space. Kernel input/output counters were 4,413,328 / 6,446,872 for v2 versus
270,512 / 517,312 for v1; these are process-accounting counters, not device-byte
measurements. The storage/time tradeoff is part of selecting v2.

Reproduce each size into a new output directory on a disk-backed scratch volume:

```sh
SOURCE_REVISION=575044b6d2e9fba3aa7467489ed19afa9088e4e0 \
  STEPS=1000000 BACKENDS="v2 v1" MEMORY_MAX=128M \
  scripts/bench-v2-seed.py target/release/freespace ./new-seed-results
```

[Raw cgroup/CLI/time records and exact archived runners](benchmark-results/v2-seed-memory-2026-10-26.json)
include the four runs, source/tree/binary and runner hashes, memory limits,
pressure counters, complete reports and measurement qualifications. Generated
fixtures/stores were removed after the run; the results remain available.

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

## GitHub container storage balance (2026-10-27)

Revision 5 stores immutable scope/path/compact traversal keys in
`identities` and its lookup indexes, numeric metadata in `live_rows` and `version_rows`, and raw
basenames in the historical child index. History uses `WITHOUT ROWID`. Current
refresh traverses current parent links, preserving its locality after deletion;
child/diff streams retain indexed byte ordering. The shared staged visitor also
skips empty child probes for non-directory entries. This comparison measures the
combined changes, not the isolated contribution of each change.

### Paired 100k pilot

[GitHub run 40414910069](https://github.com/mikhailsal/free-space/actions/runs/40414910069)
compares baseline `036fd2b7cb223a212dc3c68d78d4400d74ab793a` against
`7871961c5b06ed73b4191eb84858c2a97d0549ca`. Both GNU release builds use Rust/Cargo
1.99.1, Bun 1.4.2 and `cargo build --release --locked -p freespace`.
The candidate binary SHA-256 is
`ef5d5e9b773ff1e427c59a8a79e1d052ff23e2f89db7251b9e5171d7b06608b7`;
full provenance is in the [raw pilot records](benchmark-results/v2-storage-pilot-2026-10-27.json.gz).
The archived records are gzip-compressed JSON; decompress with `gzip -dc FILE.json.gz`.
These are GNU/glibc builds, not measurements of the distributed static musl binary.

All comparison arms run sequentially on the same GitHub-hosted VM, each in a
fresh Docker container with a 2-CPU quota, 256 MiB memory limit, zero swap,
network disabled, and bind-mounted fixture/snapshot storage. The fixture has
100,000 unrelated files and a fixed 18-entry refresh target: 100,120 entries in
total. There are two paired repeats with alternating arm order, three complete
`scan --low-memory --workers 1` commands, then a full CLI refresh. Both
`keep_previous` settings are measured. All 48 commands validate with no errors,
incompleteness, timeout, sampler failure or OOM; all 48 immutable post-command
SQLite observations leave file occupancy unchanged. Candidate verification
passed 291 Rust tests, with zero failures and one ignored test.

This pilot generates fixtures inside each arm cgroup and retains cache across
fixture creation, scans and immutable SQLite observations. It makes no cold-cache
claim. GNU time reports each CLI's peak RSS; cgroup peak includes preparation and
all earlier activity in that arm. Complete wall time includes process start and
last-writer close/checkpoint, while observations run outside timed spans. Disk
peaks sampled every 20 ms are lower bounds for named files under `snapshot_dir`;
temporary files elsewhere and already-unlinked files are outside this metric.
It is not a hard bound on all process disk use. Class maxima must not be added to
reconstruct a simultaneous total; the reported total is sampled concurrently.
Short refresh WAL activity can be missed entirely by this sampler.

For `keep_previous=true`, medians of the two repeats are:

| Phase | Baseline v2 snapshot allocation | Revision 5 snapshot allocation | Baseline sampled total peak | Revision 5 sampled total peak |
| --- | ---: | ---: | ---: | ---: |
| First full scan | 111.18 MB | 44.95 MB | 223.10 MB | 90.14 MB |
| Second full scan | 165.11 MB | 55.41 MB | 315.44 MB | 84.87 MB |
| Third full scan | 220.47 MB | 65.81 MB | 406.16 MB | 104.53 MB |

MB means 1,000,000 bytes. Endpoint values include the snapshot directory's main
DB, sidecars and state files. Exact logical DB size falls from 111,136,768 to
44,912,640 bytes after the first scan (**59.6% smaller**), and from 220,430,336 to
65,769,472 after the third (**70.2% smaller**). Both layouts retain 100,120
identities/current rows and 300,360 history rows after three scans: the saving
does not discard history. The initial baseline has roughly 111.7 MB of sampled
WAL alongside its 111.1 MB DB; staging peaks around 7.1 MB. The large peak is
primarily DB/WAL overlap, not a separate multi-gigabyte cache.

| Command (`keep_previous=true`) | Baseline v1 | Baseline v2 | Revision 5 v2 |
| --- | ---: | ---: | ---: |
| First full scan, seconds | 1.663 | 15.897 | 7.129 |
| Second full scan, seconds | 1.708 | 23.012 | 6.787 |
| Third full scan, seconds | 1.658 | 27.981 | 7.714 |
| 18-entry refresh, milliseconds | 306.30 | 15.35 | 9.53 |

Revision-5 fresh scan ranges are 6.140–8.119 seconds versus baseline v2
13.838–17.955; third scan ranges are 7.700–7.729 versus 27.927–28.035. Observed
refresh medians are 32.1 times faster than baseline v1 with previous history,
and 38.3 times faster without it (9.30 versus 356.07 ms). The new full scan still
takes about four to five times as long as baseline v1 on this fixture. Scan RSS
remains roughly 25–26 MiB; refresh RSS is about 8.4 MiB, versus about 31 MiB for
baseline v1. Cgroup peaks approach the 256 MiB limit because they include helper
and filesystem cache; RSS is not total container memory.

The [identical-binary control](https://github.com/mikhailsal/free-space/actions/runs/40412597121)
validated all 48 commands with the same executable in both v2 arms. Individual
paired fresh-scan timings differed by approximately −42% to +48%, showing that
this resource-constrained setup is noisy. That control used Rust 1.94, so it is
method evidence, not the optimized candidate's timing baseline. The candidate
pilot builds its own paired baseline with Rust 1.99.1. Its disk reductions are
consistent across both repeats/settings, and measured timing ranges improve
substantially, but two repeats do not establish tight confidence intervals or
predict another machine's latency. [Raw control records](benchmark-results/v2-storage-control-2026-10-27.json.gz)
preserve the unchanged-arm evidence.

### Million-file command evidence

[Run 40418037469](https://github.com/mikhailsal/free-space/actions/runs/40418037469)
uses source `770730769b9a66986b25fff63236730e7a20d22d`, whose scanner binaries
have the same hashes as the accepted pilot. Each arm uses a fresh 2-CPU,
128 MiB no-swap container, one repeat per `keep_previous` setting, two low-memory
full scans and an 18-entry refresh. Each report covers 1,001,020 entries,
including one million unrelated files. Fixtures are prepared outside the
measured container on the dedicated GitHub-hosted VM; all six sync/cache-drop
receipts succeeded. Cache then remains in the arm cgroup across commands and
immutable observations. Active pages and hardware caches are not reset.
The cgroup peak here excludes host fixture preparation, while including the
CLI, helper and observations. This differs from the warm 256 MiB pilot;
latencies must not be pooled across these protocols.

**The overall comparison run failed.** It contains 15 of 18 expected command
records: 13 valid CLI results, two failed v1 refreshes and three old-v2 commands
not reached after observer failures. All six candidate-v2 CLI commands completed
with valid counts, zero errors/incompleteness and zero OOM events. The two v1
refreshes were killed with OOM; one also reached the watchdog timeout. Their
elapsed times are failure diagnostics, never completed latency samples.

Four post-command observer guards rejected a decrease in allocated SHM blocks
from 36,864 to 32,768 bytes. In each case the only changed occupancy field was
`generations.db-shm` allocation; its logical size stayed 32,768, and every other
file name, logical size and allocation stayed the same. This is evidence of an
overstrict allocation-equality guard, not retrospective proof of byte identity:
this protocol captured no pre/post content hashes. The failed status is retained
in the [complete records](benchmark-results/v2-storage-confirmation-2026-10-27.json.gz).
The following values come only from independently validated CLI records,
collected before their subsequent observation; they do not assert an all-green
comparison workflow.

| Million-file command | Baseline v2 logical DB | Revision-5 logical DB | Baseline sampled directory peak | Revision-5 sampled directory peak |
| --- | ---: | ---: | ---: | ---: |
| First full scan, `keep_previous=true` | 1,116.29 MB | 452.03 MB | 2,241.26 MB | 907.55 MB |
| Second full scan, `keep_previous=true` | 1,658.00 MB | 558.32 MB | 3,186.63 MB | 852.41 MB |

The logical DB is **59.5% smaller** after seeding and **66.3% smaller** after the
second scan. These values preserve the same current/history cardinalities.
Observed directory peaks fall about 59.5% and 73.3%, respectively; they remain
sampled lower bounds for named snapshot-directory files. No hard process disk
cap or maintenance steady state was established.

| Complete command, `keep_previous=true` | Baseline v1 | Baseline v2 | Revision-5 v2 |
| --- | ---: | ---: | ---: |
| First low-memory scan | 42.20 s | 169.65 s | 122.95 s |
| Second low-memory scan | 40.99 s | 250.21 s | 96.83 s |
| 18-entry refresh | Failed: OOM plus timeout | Not reached after observer guard | 30.82 ms |

With `keep_previous=false`, candidate scans complete in 101.75 and 96.13 seconds,
and refresh in 23.36 ms. Candidate scan RSS spans 22.18–25.63 MiB, refresh
8.00–8.33 MiB; the configured cgroup limit remains 128 MiB and its peak reaches
that limit. Completion under this budget does not imply spare memory or another
machine's latency. A single repeat per setting establishes no tight latency
confidence interval. V1's valid scans remain faster and its DB remains smaller;
no finite million-file v1 refresh speedup is derived from its failed executions.

Earlier 128 MiB attempts failed during fixture generation before any freespace
command; they provide no scanner performance or memory-limit result. The host
fixture/cold-cache protocol above separates that preparation cost from the CLI.
The observer repair now checks names, logical sizes and bounded SHA-256 content
inventories, recording allocation drift independently. Integrity reads occur
outside CLI timing but affect subsequent cache state, so the repaired protocol
is separately versioned. [Diagnostic run 40426917945](https://github.com/mikhailsal/free-space/actions/runs/40426917945)
passed both baseline/candidate observers on OS-enforced read-only mounts, with
identical pre/post SHA-256 inventories and EROFS write/create probes. Four
regressions verify allocation-only drift acceptance and rejection of same-size
byte, name and logical-size mutations. [Diagnostic records](benchmark-results/v2-observer-check-2026-10-27.json.gz)
are correctness evidence, not another million-file performance sample. This does
not retroactively supply hashes or a passing status for the earlier comparison.

History still grows on repeated full scans, including `keep_previous=false`:
that option removes the public comparison generation, while obsolete physical
versions await explicit bounded maintenance. This experiment does not measure
maintenance steady state or promise a fixed-size cache. Revision 4 and 5 stores
are incompatible; seed a new state directory following the
[upgrade guide](refresh-v2.md#upgrade-an-earlier-experimental-store).

## Ordinary-memory current-backend comparison (2026-10-27)

[Run 40474178463](https://github.com/mikhailsal/free-space/actions/runs/40474178463)
answers how the candidate behaves when the memory ceiling no longer constrains
this fixture. Both backends use the **same current candidate binary**, SHA-256
`ef5d5e9b773ff1e427c59a8a79e1d052ff23e2f89db7251b9e5171d7b06608b7`, built
from source `e370279a161c235a0381c00f92b9d5f97f4502ab`. Scanner implementation
and build inputs are unchanged from the prior candidate; this revision adds
benchmark controls. The historical v1 binary is not used as the comparator.

Each arm gets a fresh Docker cgroup with a **4 GiB memory limit, no swap,
2-CPU quota and one scan worker**. The fixture contains one million unrelated
files, a fixed 18-entry refresh target, and 1,001,020 total entries.
`keep_previous=true`. Two paired repeats alternate backend order. Default
in-memory staging and `--low-memory` disk staging are measured separately,
sequentially on the same GitHub-hosted VM: eight arm containers and 24 commands.

All 24 commands validate, all eight Docker states report no OOM, all eight
sync/cache-drop receipts succeed, and all 24 observer hash inventories match.
Every recorded cgroup `memory.events` counter remains zero, including `max` and
`oom`; the largest arm peak is 2,978,213,888 bytes (2,840.25 MiB), below the
4,096 MiB limit. Thus these timings are not measurements at the memory ceiling.
All complete [records are archived as compressed JSON](benchmark-results/v2-normal-memory-2026-10-27.json.gz).

The initial scan follows the requested clean OS cache drop for each arm;
subsequent commands retain cache. SHA-256 integrity reads and immutable SQLite
observations run outside timed spans but can warm cache. Active pages and
hardware caches are not reset. Complete CLI wall time includes process start,
publication and close/checkpoint. These are GNU/glibc release builds with
Rust/Cargo 1.99.1 and Bun 1.4.2; they are not timings of the distributed musl
binary or a prediction of another machine's filesystem. Two repeats provide
observed ranges, not tight statistical confidence intervals.

### Default in-memory staging

Seconds except the refresh row; values are median [minimum–maximum].

| Complete command | Current v1 | Current v2 |
| --- | ---: | ---: |
| Initial full scan, s | 12.186 [12.061–12.312] | 51.831 [51.091–52.571] |
| Repeat full scan, s | 5.363 [5.352–5.374] | 31.922 [31.534–32.310] |
| 18-entry refresh, ms | 2,543.57 [2,505.81–2,581.32] | 8.254 [8.135–8.372] |

At these medians, v2 takes **4.25 times** as long for the initial full scan and
**5.95 times** for the repeat, while its small refresh is **308 times faster**.
Median scan RSS is approximately 184 MiB for v1 and 535 MiB for v2; refresh RSS
is approximately 199 MiB and 8 MiB, respectively. RSS excludes filesystem cache
and the helper. The largest cgroup peaks across the arm lifecycle are about
2,057 MiB for v1 and 2,840 MiB for v2, including cache and observations.

The report groups filesystem preparation/aggregation in `walk_seconds` and
publication work in the other stages. Initial walk medians are about 9.90 s for
v1 and 10.96 s for v2; repeat walks are about 3.05 s and 3.35 s. V2's publication
transaction (`insert_seconds`) takes about 38.29 s initially and 27.09 s on
repeat. This identifies the expensive reported phase; it is not a causal
CPU-versus-device-I/O breakdown. Additional RAM does not eliminate the measured
versioned-publication cost.

### Disk staging with the same 4 GiB budget

| Complete command | Current v1 | Current v2 |
| --- | ---: | ---: |
| Initial full scan, s | 25.720 [25.671–25.768] | 57.703 [57.295–58.111] |
| Repeat full scan, s | 18.345 [18.242–18.447] | 39.847 [39.761–39.934] |
| 18-entry refresh, ms | 2,617.36 [2,516.99–2,717.74] | 8.523 [8.399–8.647] |

V2's scan RSS is about 26–27 MiB in this mode, compared with about 535 MiB for
its default in-memory scan. Its observed full scans take about 58/40 s instead
of 52/32 s; this is an explicit time/process-RSS tradeoff. The whole cgroup
still peaks around 2,508 MiB because filesystem cache remains charged to it.
V1's disk-scan RSS is about 25 MiB, and refresh still uses its in-memory engine.

The harness exposes `--current-backends`, `--scan-profile memory|disk|both`,
`--keep-previous true|false|both`, and `--memory 4096` for this comparison. These
flags select a benchmark profile; they do not alter application defaults or
reserve 4 GiB on the user's computer. Existing 128 MiB records remain a separate
constrained-memory experiment, with their original failure qualifications.
