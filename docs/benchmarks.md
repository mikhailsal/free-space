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
