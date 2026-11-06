# Whole-disk run on the author's host: bucketed coarse snapshot (probe) against v1

Date: 2026-11-06. Host: 8 cores, 15.6 GiB RAM with swap full, VMware disk, `/` 99% full (capacity 473,918,423,040 bytes), busy with other work throughout (load average 3-10, see "Conditions"). Probe source: research commit [`ba8281f`](https://github.com/mikhailsal/free-space/commit/ba8281f0a035509019f49459ff69b6a7b5bd2e11) on `perf/cleanup-bucket-probe`, with the local live-disk patch described below. v1 is the installed `/usr/local/bin/freespace` 0.2.0 (sha256 `32d948fb…271277a`), run as `--backend v1 scan --low-memory --workers 1`, the command the installed configuration implies and the one used for the 2026-10-31 calibration.

This is an exploratory measurement of a prototype on one live system: single runs, uncontrolled caches, no repeats of the full scans. It answers "what would this idea change on this disk", not "is the product qualified". It does not change the verdict of the [synthetic screen](cleanup-bucket-screen-2026-11-04.md); it adds what that screen could not show.

## Verdict

On this disk the idea buys **footprint, I/O and, where it applies, refresh speed. It does not buy scan time**, and it has costs the synthetic screen did not expose: the hardlink fallback and the loss of names.

* **Database size: 1.37 GiB (v1) to 3.01 MiB, 465 times smaller.** The installed v2 database (a scheduled scan on 2026-10-31) is 8.85 GiB, about 3,000 times the probe's, and larger than the free space this host had during the run (5.3-8.5 GB).
* **Scan peak disk: 3.03 GiB (v1) to 3.0 MiB; bytes written per scan: 15.6 GB to 3.3 MB.** With a 2.5 GiB safety reserve, v1 could not be run here until old scans were deleted to make room.
* **Memory: no gain on scans** (Max RSS 32.0 MiB against 23.9 MiB; that is the scanner process only, see [Memory](#memory-what-max-rss-means-and-how-little-ram-a-scan-needs)); **a large gain on refresh**: at most 14 MiB (10.7 MiB in a clean measurement) against the 1.78 GiB that a v1 refresh needed on 2026-10-31, and 350 MiB per million entries in the capped test below.
* **Full scan: 21.6% slower than v1** (3,265 s against 2,685 s). The probe walks the tree twice. Its CPU time is half of v1's (709 s against 1,484 s) because it writes no database; the wall time is the disk.
* **Refresh: 0.3-2.4 s (median per scenario) against 66.6 s for v1** when the planned region holds no hardlinked entry. When it does, the probe rescans the whole root (about as long as a full scan). On this disk that is **25.4% of the folded entries and 51.5% of the retained directories**, and it includes the folder the 2026-10-31 v1 calibration used.
* **What is lost: identity.** At the default threshold (0.1% of the disk, 474 MB) 70.8% of the bytes sit in anonymous bucket totals. A lower threshold buys more (table below).

## Full scan

Same disk, same host, run one after the other (probe 02:06-03:00, v1 03:10-03:55), both under `nice -n 10 ionice -c2 -n7`.

| | probe (patched research build) | v1 `--low-memory --workers 1` |
| --- | --- | --- |
| Wall time | 3,265.0 s (54 min 25 s) | 2,684.5 s (44 min 45 s) |
| What the time is | 2 passes, 19,028,920 visits (about 9.51 M each) | walk 1,650.7 s + insert 986.4 s + index 46.7 s |
| CPU, user + sys | 108.6 + 600.4 = 709 s (22% of one core) | 807.9 + 676.3 = 1,484 s (55%) |
| Max RSS (the scanner process only, see Memory) | 32.0 MiB | 23.9 MiB |
| Database | 3,158,016 B (3,689 nodes, 5,551 buckets) | 1,468,616,704 B (9,776,275 entries) |
| Peak snapshot-directory use | 3.0 MiB | 3,101 MiB (3.03 GiB) |
| Read, written (rusage blocks x 512 B) | 24.2 GB, 3.3 MB | 14.4 GB, 15.6 GB |
| Lowest MemAvailable | 1,163 MiB | 1,693 MiB |
| System I/O stall (PSI total) | 307 s | 168 s |
| Tolerated read errors | 1 (`digest_changed` true: the disk changed between the passes) | 1 unreadable directory (`incomplete` true) |

The walk is the same speed in both: v1 walked 9.78 M entries in 1,650.7 s (5.9 k/s), the probe made 19.03 M visits in 3,265 s (5.8 k/s). The probe's time is two walks; v1's time is one walk plus 1,033 s of database writing. The probe also read 9.8 GB more than v1, which fits a second pass that came from the disk and not from the page cache (the host's page cache was only a few GB for 9.8 M entries). The synthetic screen's 1 M-entry trees fit in cache, which is the likeliest reason the probe's full scans were faster there. Per-pass times were not recorded, so this is a reading of the numbers, not a measurement.

A one-pass build would, at the same walk speed, take about 1,630 s: roughly 39% under v1. That is an estimate from this table, not a run.

Reference points from the same host, installed binary, other days (not comparable run to run; the disk had 7.67 M entries on 2026-10-27 and has 9.78 M now):

| Run | Wall | Database | Notes |
| --- | --- | --- | --- |
| v1 scan, 2026-10-27 | 355.5 s | 1,028,276,224 B | flags and RSS not recorded (21.6 k entries/s, so several walkers); this is v1, not v2 as some earlier notes had it |
| v1 `--low-memory --workers 1`, 2026-10-31 | 1,779 s | 986,165,248 B | RSS 27.7 MiB; read 10.2 GB, written 6.65 GB |
| v2 scheduled scan, 2026-10-31 02:00 | 4,491 s | 9,500,950,528 B | RSS not recorded; 3 unreadable directories |

v2 was not re-run today: the host has no room for a second 9.5 GB database and the live one was not touched.

## Refresh

Probe refresh on the whole-disk snapshot, three runs each, in place, only on targets whose plan the stored snapshot predicts will not fall back. RSS 13.8-14.3 MiB in every run, which is a ceiling: the Python harness's own 13 MiB sets the floor of that reading (see Memory; a clean measurement of a comparable refresh gave 10.7 MiB).

| Scenario | Planned region | Visits (two passes) | Wall, three runs | Median |
| --- | --- | --- | --- | --- |
| retained folder | 8,001 entries | 16,002 | 0.39, 0.54, 0.95 s | 0.54 s |
| retained folder, checkout-sized | 7,402 entries | 14,804 | 0.37, 0.81, 0.83 s | 0.81 s |
| retained folder, large | 98,794 entries | 197,590 | 2.24, 2.42, 3.04 s | 2.42 s |
| omitted path, bucket window | 671 entries | 1,344 | 0.25, 0.33, 0.52 s | 0.33 s |
| omitted path, wide bucket window | 3,023 entries | 6,048 | 0.28, 0.29, 0.29 s | 0.29 s |

v1 baseline, measured 2026-10-31 on this host with the same binary (`refresh <small folder> --workers 1`): **66.6 s** (47.5 s user, 16.7 s sys), **max RSS 1.78 GiB**, 0.99 GB written. It loads and rewrites the whole database, so its cost does not depend on the target. It was not re-run today because 1.8 GiB with swap full could have made the kernel kill someone else's process. Against that, the probe is 28 to 230 times faster and its process uses at most 0.8% of that memory, in the regions above.

**Hardlink fallback.** The probe rescans the whole root when the planned region holds a hardlinked entry (hardlink owners are collected by a first pass over the whole root). The stored snapshot predicts this, and the prediction was checked on `/usr` against real runs: **6 of 6 matched**. Three targets in hardlinked regions rescanned the whole root (1,094,618 visits, 8.2-13.8 s on the warm `/usr`); three in plain regions refreshed 5,202-8,168 visits in 0.06-0.08 s.

Predicted on the whole-disk snapshot and **not run** (each would cost about a full scan, 54 min): the 2026-10-31 calibration folder (`core/src/refresh` in this repository; its bucket window has 4,085 entries with 2,401 hardlinked) and a `node_modules/.pnpm` folder (4,065 entries, 3,352 hardlinked). Exposure over the whole snapshot: 1,709,017 hardlinked entries (17.5% of all), 1,464 of 5,551 buckets hold hardlinks and cover 25.4% of the folded entries, and 1,851 of 3,592 retained folders (51.5%) hold them in their subtree.

For the installed default (v2), refresh was measured only on a 39,012-entry partial tree (see below), where v2 took 0.09-0.43 s. How v2 refresh behaves on the real 9.8 M-entry database is not known.

## Memory: what "Max RSS" means and how little RAM a scan needs

Added after the run to answer three questions: what the "memory" in the tables above is, why it reads in tens of megabytes when earlier work on this project needed gigabytes, and whether the mechanism could run on a machine with 0.5 GB of RAM.

**What "Max RSS" is.** The peak resident set of the scanner process itself (`ru_maxrss` from `wait4`): its heap, its stack and the pages of its own binary that it touched. It is not what a scan costs the machine. Three things are outside it: the page cache and the kernel's dentry and inode slab that every directory walk fills (charged to the scan's memory cgroup, but dropped by the kernel when it needs the space); data that v1 `--low-memory` writes to disk instead of keeping (3.03 GiB of scratch on this disk); and every other process.

**Why earlier figures were gigabytes.** They were other quantities, none of them the Max RSS of a `--low-memory` scan:

| Quantity | Size per 1 M entries unless noted | A floor on the RAM needed? |
| --- | --- | --- |
| Kernel caches charged to the scan (dentry/inode slab and page cache) | 1.6-1.8 GiB in total for the probe and v1, 2.4-2.6 GiB for v2 (its database writes add about 0.7 GiB of file cache); about 1.3 KiB of slab per entry, the same for every scanner | No. The kernel reclaims it: scans completed with the whole cgroup capped at 48 MiB (below). The 1.81 GiB of [`resource-causes-2026-11-02.md`](resource-causes-2026-11-02.md) is this quantity |
| Heap of v1 refresh and of scans without `--low-memory` | v1 refresh 350 MiB (below), and 1.78 GiB on this disk's database (2026-10-31); v1 default scan 217 MiB; v2 default scan 508 MiB | Yes |
| Installer policy constants (`docs/install-memory.md`) | 64 MiB + 384 B (v1) or 1,024 B (v2) per entry: 3.7 and 9.3 GiB for this disk | Planning numbers, not measurements; the heaps above are lower on this fixture |

The two v1 refresh figures do not scale in a straight line (350 MiB per million entries would give 3.4 GiB for 9.78 M entries; 1.78 GiB was measured), so any scale-up of the heap figures here is an upper bound of unknown looseness.

**Method.** An 8 GiB sparse ext4 image (1.4 M inodes, no journal) on a loop device, remounted before every run. Unmounting drops that filesystem's dentries and inodes, so each run starts with cold kernel caches. `drop_caches` would have done that for the whole host, and was not used because the host was busy with other work. On the live disk the first scan of a tree pays the slab and the next ones get it free, so arms cannot be compared there (the first two records of `memory.jsonl`, a smoke test on `/usr/include`, show it: 16.6 MB of slab for the first scan, 0.09 MB for the second). Fixture **A**: 1,004,501 entries (1,000,000 empty files in 4,500 folders), no hardlinks. Fixture **B**: 300,000 files and 300,000 hardlinks to them (300,000 distinct hardlinked inodes, 600,000 non-directory entries, paths of about 94 bytes). Every command ran in its own transient systemd scope with `MemoryMax` set and swap off, so exceeding the cap is an out-of-memory kill of that scope and nothing else. The scope's `memory.current` was sampled every 20 ms (kernel 5.15 has no `memory.peak`, so the cgroup peak is a sampled lower bound) beside `ru_maxrss`.

**A floor on small RSS values.** The harness is a Python process of 10.5-13 MiB, and Linux keeps the larger of the parent's peak and the exec'd program's. Probe readings at or below about 13 MiB are therefore ceilings, marked "floor" below. A clean measurement (`/usr/bin/time`, no Python parent, [`results/memory/clean-rss-usr.json`](local-host-2026-11-06/results/memory/clean-rss-usr.json)) gives 8.6 MiB for a probe scan of `/usr` (547 k entries), 10.7 MiB for a probe refresh inside that snapshot and 5.4 MiB for the idle binary. The 13.8-14.3 MiB quoted for the whole-disk refreshes has the same floor. Readings above 13 MiB (the whole-disk probe's 32.0 MiB, every v1 and v2 figure) are not affected, and neither is any "completed or killed" result.

**Uncapped reference** (cap 6 GiB, cold caches):

| Fixture | Arm | Visits | Max RSS | cgroup peak | anon | file cache | slab | Wall | Snapshot |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| A | probe | 2,009,002 | <= 12.2 MiB (floor) | 1,603 MiB | 6 | 263 | 1,334 | 36.1 s | 0.0 MiB |
| A | v1 `--low-memory --workers 1` | 1,004,501 | 35.5 MiB | 1,747 MiB | 29 | 443 | 1,272 | 73.7 s | 112.8 MiB |
| A | v1 default (8 workers) | 1,004,501 | 217.2 MiB | 1,858 MiB | 136 | 370 | 1,337 | 11.6 s | 106.1 MiB |
| A | v2 `--low-memory --workers 1` | 1,004,501 | 30.0 MiB | 2,434 MiB | 26 | 1,111 | 1,296 | 149.9 s | 426.7 MiB |
| A | v2 default | 1,004,501 | 508.5 MiB | 2,659 MiB | 374 | 1,064 | 1,202 | 104.7 s | 419.8 MiB |

The five scanners differ by a factor of at least 40 in process memory and by less than 1.7 in what the scan charges to the machine. The probe visits every entry twice (two passes). Wall times are not a speed comparison: v1 default used 8 threads, the others one, the backing file of the image sits in the host's page cache, and 1 M empty files are not a disk.

**With a cap** (the cap counts the process, its page cache and its slab; swap off; cold caches before every run; every run in [`results/memory/memory.jsonl`](local-host-2026-11-06/results/memory/memory.jsonl)):

| Arm | Fixture | Max RSS | Completed at every cap down to | Killed at | Wall, uncapped to lowest cap completed |
| --- | --- | --- | --- | --- | --- |
| probe | A, 1.0 M entries, no hardlinks | <= 12.7 MiB (floor; 8.6 clean) | **48 MiB** | 32 MiB | 36.1 s to 64.7 s |
| probe | B, 300 k hardlinked inodes | 58.0-59.4 MiB | **128 MiB** | 96 MiB | 17.3 s (1 GiB cap, no pressure) to 28.2 s |
| v1 `--low-memory --workers 1` | A | 34.6-35.5 MiB | 48 MiB (the lowest tried) | none | 73.7 s to 108.3 s |
| v1 default | A | 217-227 MiB | 256 MiB | 192 MiB | 11.6 s to 11.0 s |

What it shows:

* **A scan fits in very little.** On the plain 1 M-entry tree the probe completed with the whole cgroup (process, page cache and slab) held to 48 MiB, and was killed at 32 MiB. The cost of pressure is time, not failure: 36 s uncapped, 48-54 s from 1 GiB to 96 MiB, 65 s at 48 MiB (+32% to +79%), because the second pass finds fewer dentries cached.
* **A small machine does not need the probe to scan.** v1 `--low-memory` completed at every cap down to 48 MiB. Its price is the disk staging and the database described above, not RAM.
* **v1 in default mode needs its heap**: 217 MiB per million entries; killed at 192 MiB. v2 default is 508 MiB per million entries and v2 `--low-memory` 30 MiB; v2 was not run under caps.
* **Hardlinks are the probe's one memory cost that grows with the data.** The probe keeps one entry per distinct hardlinked inode (inode key and a copy of the owner path): 58.0 MiB on B against 8.6 MiB clean, **160-173 bytes per inode** at 94-byte paths (160 is the bound from the floor-clamped reading on A). On the whole disk the probe peaked at 32.0 MiB, which fits an owner table of the order of 100-150 k inodes (not counted directly; the unmodified probe's 100,000-owner cap rejected this disk). One million distinct hardlinked inodes would take about 170 MB and five million about 850 MB; the second figure is out of reach of 0.5 GB. A hash of the owner path in place of the path would cut the table to a few tens of bytes per inode; it was not built or measured.

**Refresh** of one folder of the 1,004,501-entry snapshot, on a copy of the snapshot per run, cold caches:

| Arm | Max RSS | Anonymous heap at the peak sample | Wall | Completed at | Killed at |
| --- | --- | --- | --- | --- | --- |
| v1 | 350.4-350.6 MiB (the cgroup shows about 265 MiB of it as anonymous heap; the rest is mostly file-backed pages that the scope is not charged for) | 265 MiB | 4.9-5.1 s | 512 MiB | 256 MiB (OOM kill) |
| probe | <= 13.1 MiB (floor; 10.7 clean) | 6-8 MiB | 0.2 s | every cap down to 48 MiB (the lowest tried) | none |

This is where the probe changes the memory picture. v1's refresh loads the database, so its memory follows the snapshot (350 MiB here, 1.78 GiB for this disk's), while the probe touches only the planned region.

**Could it run on a machine with 0.5 GB of RAM?** For scanning and refreshing a snapshot, the measurements say yes, with a margin of about ten on a tree without many hardlinks (the whole cgroup fit in 48 MiB at 1 M entries, against 512 MiB) and about four on a tree with 300,000 distinct hardlinked inodes (128 MiB). Three things are not shown: millions of distinct hardlinked inodes (backup trees, container layer stores) break it unless the owner table is changed; the rest of a product (GUI, server, CLI queries) was not measured, because nothing reads the probe snapshot yet; and a whole 512 MB machine, where the kernel and other services take part of the memory, was not tried, only a cgroup. What the probe adds for small machines is therefore not scan memory, which v1 `--low-memory` already has, but the refresh and the database: no refresh heap that grows with the snapshot, and a small database (3.0 MiB for this whole disk, 40 KiB on these fixtures where nothing reaches the threshold, against 113 MiB for v1 and 427 MiB for v2 at 1 M entries on the same fixture).

**Limits of this measurement.**

* Synthetic trees: empty files with five-character names. Real names and deeper paths make the slab, the v1 and v2 heaps and the probe's owner table larger.
* The image's backing file is in the host's page cache, so "disk" reads here are fast. The extra time under a cap is a lower bound for a small machine with a slow disk, where the probe's second pass would reread metadata from it.
* One run per cell. A cgroup peak at a cap equals the cap by construction; the uncapped peaks are 20 ms samples and therefore lower bounds.
* Whole disk (9.78 M entries) under a cap was not run (about 54 minutes of load), so how the time under pressure scales from 1 M to 9.8 M entries is unknown. Only the 32.0 MiB uncapped peak is measured at that scale.
* The probe is the patched research build, with the record caps raised from 100,000 to 5,000,000. The unmodified build would refuse hardlink-heavy trees outright.
* The fixture scripts, the harnesses and the raw records are in `local-host-2026-11-06/` (see "Where to look"). The image and its scratch were deleted after the runs, and the transient systemd slice was stopped.

## What the small database costs in information

The probe keeps a node when it is the root, when its subtree allocates at least the threshold, or when it holds more than 4,096 entries. Everything else is folded into per-folder bucket totals with no names.

At the default 0.1% threshold on this disk: 3,593 folders and 96 files kept (the 96 files hold 129.7 GB, 29.2% of the disk), **9,772,914 entries and 314.7 GB (70.8% of the bytes) in 5,551 buckets**, and 551 buckets of 100 MiB or more hold 54.3% of the bytes. A file under 474 MB, or a folder under 474 MB with at most 4,096 entries, cannot be named, listed or ranked; the snapshot shows its parent's total and the bucket it falls in.

The threshold is a knob. These rows apply the probe's rule to the v1 snapshot taken in this run, which holds every entry (arithmetic, not probe runs; at 0.1% it gives 3,688 nodes and at least 5,352 buckets against the probe's real 3,689 and 5,551). The size column multiplies nodes plus buckets by the probe's measured 342 bytes per row, so it is a floor: deeper paths cost more per row.

| Threshold | Bytes | Nodes | Buckets (at least) | Size, about | Share of the disk in named files |
| --- | --- | --- | --- | --- | --- |
| 0.1% (default) | 474 MB | 3,688 | 5,352 | 3 MiB (real 3.01) | 28.9% |
| 0.03% | 142 MB | 4,391 | 5,862 | 3.3 MiB | 38.2% |
| 0.01% | 47 MB | 6,535 | 7,187 | 4.5 MiB | 51.2% |
| 0.003% | 14 MB | 12,311 | 10,690 | 7.5 MiB | 62.1% |
| 0.001% | 4.7 MB | 25,607 | 19,003 | 14.6 MiB | 69.8% |
| 0.0003% | 1.4 MB | 67,261 | 46,748 | 37 MiB | 77.2% |

Not integrated: no GUI, CLI query or scheduler reads this snapshot. The numbers above say what the format could hold, not what the product shows.

## Conditions and limits

* **Single runs of both full scans**, one after the other, not interleaved. Load average at start and end (1, 5, 15 min): probe 6.1/8.1/7.2 to 9.6/8.0/6.8; v1 9.6/6.4/6.0 to 4.5/4.1/4.1. Caches were not flushed and were mostly cold because the host had little to spare. Read the 22% as "about 20% either way".
* **Live disk.** Other tasks created and deleted files during both scans, including a 3.5 M-entry job folder. The probe's two passes saw different states (`digest_changed` true) and it tolerated one error; v1 reported one unreadable directory. Entry counts differ slightly for that reason (9,776,602 against 9,776,275).
* **The probe was patched to run here.** The unmodified build refuses this disk: it rejects nested mounts and other devices, requires the snapshot directory to be outside the root, fails on any vanished or unreadable entry or a digest mismatch between passes, and caps nodes and hardlink owners at 100,000 (this host has 106,581+ distinct hardlinked inodes and 1.7 M hardlinked entries). The patch (`scripts/apply_live_patch.py`, every replacement asserted) raises the caps to 5 M records, 4 M buckets and 100 M digest entries, skips mounts, the snapshot directory and other devices, and counts instead of failing on vanished or unreadable entries and digest changes. It produced an identical `coarse.db` to the original build on two static trees, `~/.rustup` and `/usr`: every row of `meta`, `nodes` and `buckets` matches, ignoring scan timestamps and generation (`scripts/compare_coarse.py`). Binary sha256 `972c9d6e…e29067b`.
* **v1 refresh and v2 were not re-measured at whole-disk scale** (memory risk, no disk room), so those comparisons rest on the 2026-10-31 figures. The Memory section adds measurements of both on a 1 M-entry fixture, with its own method and limits (synthetic trees on a loop image, one run per cell, a cgroup cap and not a 512 MB machine).
* **Partial-tree campaign (`~/.cargo`, 39,012 entries, three runs per arm).** Scan: probe 0.83 s, v1 1.42 s, v2 4.17 s; database 0.04 MiB, 9.9 MiB, 36.5 MiB. Refresh (omitted/wide): probe 0.11/0.08 s, v1 0.42/0.72 s, v2 0.09/0.43 s. At this size the three are all fast; it is kept to show that v2's refresh is not slow on a small tree. The `/usr` and `~/.rustup` partial-tree campaigns were stopped before they measured anything, when this whole-disk run replaced the plan (`results/usr-aborted-seed-phase.jsonl` is the only record of the `/usr` start).

## What went wrong on the way

* The first whole-disk probe attempt was killed at 2,337 s by my own memory guard (system MemAvailable fell under 1 GiB while the probe itself held 29 MiB). The guard now acts only when the measured command is itself large; the attempt is kept in `results/aborted/`.
* The launch ran twice; the second copy exited at once (coarse snapshot locked) and left three junk records, the first three lines of `results/whole-disk.jsonl` (exit code 1, 0.06 s, plus a phase start and end at 02:06:56). They carry no measurement.
* Earlier notes called the 355 s scan "v2". It is v1 (2026-10-27, flags not recorded). The real v2 scheduled scan took 4,491 s.

## Changes made on the host

Old scans were deleted only to make room for the v1 run, with permission, least valuable first. Removed: `/var/lib/freespace/calibration-20261031/` (941 MiB, the 2026-10-31 v1 calibration snapshot; its `time.txt`, `refresh-time.txt` and `state.json` are kept in `results/baselines-oct31/`) and `/var/lib/freespace/snapshot.prev.db` (1,205,432,320 B, the v1 previous snapshot of 2026-10-25). Untouched: `snapshot.db` (2026-10-27), `state.json`, `v2/generations.db` (2026-10-31) and `/etc/freespace/config.toml`; no timer or service was started or stopped. Free space went from 5.7 GB to 8.5 GB. The temporary benchmark snapshots were removed.

## Where to look

Everything is under [`local-host-2026-11-06/`](local-host-2026-11-06/): `scripts/` (harnesses, the live patch, analysis, `redact.py`) and `results/` (`whole-disk.jsonl` holds every timed command; `v1-whole-scan/`, `probe-whole-scan/`, `usr-fallback-check.json`, `whole-targets-plans.json`, `threshold-estimate.out`, `whole-probe-coarse-analysis.json`, `whole-probe-folded-share.json`, `cargo-partial-tree.jsonl`, `baselines-oct31/`). The memory runs are `scripts/make_fixture.py`, `mem_ladder.py`, `mem_refresh.py` and `summarize_memory.py`, with their records and logs in `results/memory/` (`memory.jsonl`, `memory-refresh.jsonl`, `clean-rss-usr.json`); `python3 scripts/summarize_memory.py results/memory/memory.jsonl results/memory/memory-refresh.jsonl` reprints the memory tables. The repository is public, so home-directory paths are replaced with `<home-path-redacted>` and the list of the biggest folders is dropped; numbers and structure are unchanged, and the unredacted files stay on the host in `/tmp/fsbench/results`. `python3 scripts/summarize.py whole results/whole-disk.jsonl` reprints the timing rows.

## What would make this worth integrating

Read as hypotheses, not results: a one-pass walk (the second pass is the whole scan-time loss); refresh that resolves hardlinks locally instead of rescanning the root (without it, a quarter to a half of refreshes cost a full scan here); a lower default threshold (0.01% costs about 4.5 MiB and names half the bytes); an owner table that does not grow with the number of hardlinked inodes (a hash of the owner path instead of the path), if machines with 0.5 GB matter; and a v2 refresh measurement at scale on a volume with room for the database.
