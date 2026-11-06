# Whole-disk run on the author's host: bucketed coarse snapshot (probe) against v1

Date: 2026-11-06. Host: 8 cores, 15.6 GiB RAM with swap full, VMware disk, `/` 99% full (capacity 473,918,423,040 bytes), busy with other work throughout (load average 3-10, see "Conditions"). Probe source: research commit [`ba8281f`](https://github.com/mikhailsal/free-space/commit/ba8281f0a035509019f49459ff69b6a7b5bd2e11) on `perf/cleanup-bucket-probe`, with the local live-disk patch described below. v1 is the installed `/usr/local/bin/freespace` 0.2.0 (sha256 `32d948fb…271277a`), run as `--backend v1 scan --low-memory --workers 1`, the command the installed configuration implies and the one used for the 2026-10-31 calibration.

This is an exploratory measurement of a prototype on one live system: single runs, uncontrolled caches, no repeats of the full scans. It answers "what would this idea change on this disk", not "is the product qualified". It does not change the verdict of the [synthetic screen](cleanup-bucket-screen-2026-11-04.md); it adds what that screen could not show.

## Verdict

On this disk the idea buys **footprint, I/O and, where it applies, refresh speed. It does not buy scan time**, and it has costs the synthetic screen did not expose: the hardlink fallback and the loss of names.

* **Database size: 1.37 GiB (v1) to 3.01 MiB, 465 times smaller.** The installed v2 database (a scheduled scan on 2026-10-31) is 8.85 GiB, about 3,000 times the probe's, and larger than the free space this host had during the run (5.3-8.5 GB).
* **Scan peak disk: 3.03 GiB (v1) to 3.0 MiB; bytes written per scan: 15.6 GB to 3.3 MB.** With a 2.5 GiB safety reserve, v1 could not be run here until old scans were deleted to make room.
* **Memory: no gain on scans** (32.0 MiB against 23.9 MiB); **14 MiB per refresh** against the 1.78 GiB that a v1 refresh needed on 2026-10-31.
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
| Max RSS | 32.0 MiB | 23.9 MiB |
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

Probe refresh on the whole-disk snapshot, three runs each, in place, only on targets whose plan the stored snapshot predicts will not fall back. RSS 13.8-14.3 MiB in every run.

| Scenario | Planned region | Visits (two passes) | Wall, three runs | Median |
| --- | --- | --- | --- | --- |
| retained folder | 8,001 entries | 16,002 | 0.39, 0.54, 0.95 s | 0.54 s |
| retained folder, checkout-sized | 7,402 entries | 14,804 | 0.37, 0.81, 0.83 s | 0.81 s |
| retained folder, large | 98,794 entries | 197,590 | 2.24, 2.42, 3.04 s | 2.42 s |
| omitted path, bucket window | 671 entries | 1,344 | 0.25, 0.33, 0.52 s | 0.33 s |
| omitted path, wide bucket window | 3,023 entries | 6,048 | 0.28, 0.29, 0.29 s | 0.29 s |

v1 baseline, measured 2026-10-31 on this host with the same binary (`refresh <small folder> --workers 1`): **66.6 s** (47.5 s user, 16.7 s sys), **max RSS 1.78 GiB**, 0.99 GB written. It loads and rewrites the whole database, so its cost does not depend on the target. It was not re-run today because 1.8 GiB with swap full could have made the kernel kill someone else's process. Against that, the probe is 28 to 230 times faster and uses 0.8% of the memory, in the regions above.

**Hardlink fallback.** The probe rescans the whole root when the planned region holds a hardlinked entry (hardlink owners are collected by a first pass over the whole root). The stored snapshot predicts this, and the prediction was checked on `/usr` against real runs: **6 of 6 matched**. Three targets in hardlinked regions rescanned the whole root (1,094,618 visits, 8.2-13.8 s on the warm `/usr`); three in plain regions refreshed 5,202-8,168 visits in 0.06-0.08 s.

Predicted on the whole-disk snapshot and **not run** (each would cost about a full scan, 54 min): the 2026-10-31 calibration folder (`core/src/refresh` in this repository; its bucket window has 4,085 entries with 2,401 hardlinked) and a `node_modules/.pnpm` folder (4,065 entries, 3,352 hardlinked). Exposure over the whole snapshot: 1,709,017 hardlinked entries (17.5% of all), 1,464 of 5,551 buckets hold hardlinks and cover 25.4% of the folded entries, and 1,851 of 3,592 retained folders (51.5%) hold them in their subtree.

For the installed default (v2), refresh was measured only on a 39,012-entry partial tree (see below), where v2 took 0.09-0.43 s. How v2 refresh behaves on the real 9.8 M-entry database is not known.

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
* **v1 refresh and v2 were not re-measured at scale** (memory risk, no disk room), so those comparisons rest on the 2026-10-31 figures.
* **Partial-tree campaign (`~/.cargo`, 39,012 entries, three runs per arm).** Scan: probe 0.83 s, v1 1.42 s, v2 4.17 s; database 0.04 MiB, 9.9 MiB, 36.5 MiB. Refresh (omitted/wide): probe 0.11/0.08 s, v1 0.42/0.72 s, v2 0.09/0.43 s. At this size the three are all fast; it is kept to show that v2's refresh is not slow on a small tree. The `/usr` and `~/.rustup` partial-tree campaigns were stopped before they measured anything, when this whole-disk run replaced the plan (`results/usr-aborted-seed-phase.jsonl` is the only record of the `/usr` start).

## What went wrong on the way

* The first whole-disk probe attempt was killed at 2,337 s by my own memory guard (system MemAvailable fell under 1 GiB while the probe itself held 29 MiB). The guard now acts only when the measured command is itself large; the attempt is kept in `results/aborted/`.
* The launch ran twice; the second copy exited at once (coarse snapshot locked) and left three junk records, the first three lines of `results/whole-disk.jsonl` (exit code 1, 0.06 s, plus a phase start and end at 02:06:56). They carry no measurement.
* Earlier notes called the 355 s scan "v2". It is v1 (2026-10-27, flags not recorded). The real v2 scheduled scan took 4,491 s.

## Changes made on the host

Old scans were deleted only to make room for the v1 run, with permission, least valuable first. Removed: `/var/lib/freespace/calibration-20261031/` (941 MiB, the 2026-10-31 v1 calibration snapshot; its `time.txt`, `refresh-time.txt` and `state.json` are kept in `results/baselines-oct31/`) and `/var/lib/freespace/snapshot.prev.db` (1,205,432,320 B, the v1 previous snapshot of 2026-10-25). Untouched: `snapshot.db` (2026-10-27), `state.json`, `v2/generations.db` (2026-10-31) and `/etc/freespace/config.toml`; no timer or service was started or stopped. Free space went from 5.7 GB to 8.5 GB. The temporary benchmark snapshots were removed.

## Where to look

Everything is under [`local-host-2026-11-06/`](local-host-2026-11-06/): `scripts/` (harnesses, the live patch, analysis, `redact.py`) and `results/` (`whole-disk.jsonl` holds every timed command; `v1-whole-scan/`, `probe-whole-scan/`, `usr-fallback-check.json`, `whole-targets-plans.json`, `threshold-estimate.out`, `whole-probe-coarse-analysis.json`, `whole-probe-folded-share.json`, `cargo-partial-tree.jsonl`, `baselines-oct31/`). The repository is public, so home-directory paths are replaced with `<home-path-redacted>` and the list of the biggest folders is dropped; numbers and structure are unchanged, and the unredacted files stay on the host in `/tmp/fsbench/results`. `python3 scripts/summarize.py whole results/whole-disk.jsonl` reprints the timing rows.

## What would make this worth integrating

Read as hypotheses, not results: a one-pass walk (the second pass is the whole scan-time loss); refresh that resolves hardlinks locally instead of rescanning the root (without it, a quarter to a half of refreshes cost a full scan here); a lower default threshold (0.01% costs about 4.5 MiB and names half the bytes); and a v2 refresh measurement at scale on a volume with room for the database.
