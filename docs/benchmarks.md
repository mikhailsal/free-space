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

Release build of `e89c298`, Linux 5.15 x86_64/glibc, ext4, one run per case, one
walker, empty files in a single wide directory. Each scanner and its measurement
helper ran in a fresh **128 MiB cgroup with swap disabled**. The snapshot directory
was outside the tree. [Machine-readable results](benchmark-results/disk-scan-2026-10-26.json)
include the binary SHA-256, reports, sampled cgroup/disk peaks and OOM counters.

| Entries | Engine | Peak process RSS | Elapsed | Result |
|---:|---|---:|---:|---|
| 100,002 | default | 37.7 MiB | 1.13 s | completed |
| 100,002 | low-memory | 24.5 MiB | 2.65 s | completed |
| 1,000,002 | default | 123.0 MiB | 12.33 s | OOM-killed |
| 1,000,002 | low-memory | 24.1 MiB | 73.07 s | completed |

The million-file low-memory run used approximately **180.3 MiB** of sampled peak
snapshot-directory disk space, including staging and output. Its cgroup reached
the 128 MiB limit while reclaiming filesystem cache, with zero OOM events and zero
swap use. The default engine recorded an OOM kill on the same fixture. Linux 5.15
on this host does not expose `memory.peak`, so cgroup peaks were sampled every
20 ms from `memory.current`; they are not exact high-water marks.

This verifies one practical constrained workload, including a directory whose
entire contents cannot fit in the old walker's budget. It does not establish a
universal 128 MiB requirement. Repeat on the target filesystem/hardware, especially
for deep paths, many hardlinks or slow storage. At 100,002 entries the disk engine
was about 2.3 times slower; the million-file default scan did not complete, so
there is no comparable full-scan time at that size. Phase-specific RSS profiling
and Raspberry Pi measurements remain follow-up work for #11.


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
