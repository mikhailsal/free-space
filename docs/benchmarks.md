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

All tools scan `/` on one filesystem (`-x` or the default), as root, at normal priority.

| Tool | Config | Cold | Warm | Peak RSS |
|---|---|---|---|---|
| du | `-sx` | 1199 s | 891 s | 9 MB |
| duc | `index -x` | 1206 s | 1046 s | 29 MB |
| ncdu | `-x -0 -o` | 615 s | 847 s ¹ | 3 MB |
| gdu | `-x -m 8` | 286 s | 318 s | 3.3 GB |
| gdu | `-x -m 1` | 956 s | | 2.9 GB |
| **freespace** | `scan`, 8 workers (the default) | **239 s** ² | 434 s ¹ | 1.42 GB |
| freespace | `scan --workers 4` | 379 s, 657 s ³ | | 1.41 GB |
| freespace | `scan --workers 2` | 725 s | | 1.40 GB |
| freespace | `scan --workers 1` | 1234 s | | 1.40 GB |

¹ Overlapped a compile on the same machine.
² Idle rerun. The first 8-worker run, during a compile, took 576 s. Of the 239 s: walk 198 s, finalize 1.8 s, SQLite insert 11 s, indexes 28 s.
³ Two runs of the same binary on an idle machine: the variance of this disk.

freespace's time includes writing the snapshot that every later answer reads; du and gdu only print. On the default settings freespace is 5× faster than `du` cold and a little faster than gdu, at under half of gdu's memory.

Snapshot: 1.05 GB for 7.77 M entries, 135 B per entry with indexes (before the first snapshot exists, the free-space precheck assumes 150 B per entry). Peak RSS of the scan is ~183 B per entry.

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
