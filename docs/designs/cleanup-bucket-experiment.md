# Cleanup threshold, revision 3: bucketed residual summaries (pre-registration)

Date: 2026-11-04. Prospective research experiment on a research example, not a product backend. No measurement of this
source exists when this file is committed. It extends [the threshold experiment](cleanup-threshold-experiment.md), whose
fixture, volume, I/O envelope, memory ceiling, cleanup, admission and forecast rules apply unchanged except where this
file says otherwise. No installed database, configuration, timer, master branch or release is changed. Benchmarks, scans and full builds
run only on disposable GitHub-hosted runners; local work was limited to formatting, the Python contract tests and a
small std-only Rust harness for the core algorithm.

## Why a new revision

The previous million-entry screen (run 41485277207, source `678e998`) showed that a coarse snapshot wins on disk
(32 KiB against 242 MiB for v1), process RSS (8.9 against 36.7 MiB) and full-scan time (-40% files, -8% directories),
and that refreshing a retained folder takes 0.02-0.03 s against 10.7 s. It also showed one material regression:
refreshing an *omitted* folder took 36.7 s (files) and 269 s (directories) against 10.7 s for v1, because the old
anonymous residual cannot say which part of it belongs to the target, so the only exact answer was to rescan the nearest
retained boundary, the root.

This revision tests the grouping idea from that discussion: do not keep one anonymous residual per retained directory,
but fold the omitted children into a bounded number of **buckets selected by a hash of the child name**. A refresh of
an omitted path then rescans exactly the bucket that holds its first folded component. The old contribution is known
(the bucket's stored totals), the new one is recomputed from a full rescan of exactly the members of that bucket, and
both are exact. Siblings that share the bucket are refreshed as a side effect, never left wrong.

## Information contract of the probe

* Retain a node when it is the root, or its subtree allocated bytes reach the threshold (0.1% of `statvfs`
  capacity, integer ceiling, unchanged), or its subtree holds more than 4096 entries (files, directories and others).
  The last rule bounds the child list a range rescan must read; it is the only reason `bulk` is retained at 100k scale.
* Fold every other child of a retained directory into the bucket of its name slot. `slot = top 12 bits of
  (FNV-1a-64(name bytes) * 0x9e3779b97f4a7c15)`; 4096 slots. Buckets tile `[0, 4096)` greedily: a bucket is closed
  before the slot that would push it over 4096 entries, a single slot may exceed the cap, and there is always at least
  one bucket. The constant 4096 was chosen before any measurement and is not tuned to the fixture.
* A bucket stores allocated and apparent bytes, file/directory/other counts, errors, hardlink count, oldest scan time
  and newest file mtime. Totals of a node are derived: own bytes + buckets + retained children. SQLite schema 2
  records the format, slot bits and bucket cap; a reader rejects other layouts, gaps, overlaps and inconsistent totals.
* Refresh planning: a retained directory target rescans that directory; a retained file target rescans its parent
  directory; any other target resolves to the bucket of its first component below the nearest retained directory and
  rescans only the admitted children of that range (name in range and not a retained child), re-tiles the range, and
  promotes children that became retained. Demoted retained nodes fold into the bucket of their own name. A region that
  held, or now shows, hardlinks falls back to a full root rescan and reports `fallback_full_root: true`.
* Discarded per-path identity of small files and small directories stays discarded; the contract is unchanged from
  revision 2 and is not a lossless or drop-in replacement for the complete catalog.

## Comparison and fixture

Two arms on identical fixtures from the same source and toolchain: v1 (unchanged master
`afce6fdfc007a2c6914cab61aab4d84daccbb99b`) and the bucketed probe. **v2 is not run.** It is not part of any decision
gate; numbers from the earlier screen (v2 full scans 129.7 / 363.8 s, refresh 0.165 s, snapshot about 627 MiB) may be
quoted only as a different-run reference, never pooled.

The fixture content is unchanged (two profiles, 100k smoke and 1M screen, 500 GiB sparse ext4 volume, threshold
near 512 MiB, controls). Control classifications are extended, not the fixture: `bulk` retained, and `small` and
`bulk/g000050` omitted. Four scenarios per profile, each measured separately from its own two-scan seed:

| Scenario | Command | Probe must report (each pass visits the scan root plus every admitted subtree, two passes) |
| --- | --- | --- |
| full | scan root | `visited = 2 x total entries`, scope `full`, no fallback |
| retained | refresh `large` (74 entries) | `visited = 148`, boundary `large`, scope `retained` |
| omitted | refresh `small` (100 entries) | `visited = 2 x (1 + 100) = 202`, boundary root, scope `range`, window `[0,4096)` holding 100 entries |
| wide | refresh `bulk/g000050` (1001 entries) | `visited = 2 x (1 + 4004) = 8010`, boundary `bulk`, scope `range`, window holding 4004 entries (four chunk directories) |

The wide window is computed before any run with the independent Python reference: slots `[804, 971)` at 100k and
`[945, 956)` at 1M. It is added so the result is not flattered by the single-bucket `small` case: the root's lone
bucket holds only `small`, whereas `g000050` shares a bucket with three other chunk directories, so a refresh visits
four times the entries v1 visits for the same path. v1 reports `entries` for a scan and `subtree_entries`
(74 / 100 / 1001) for the refreshes.

Run counts (frozen in `scripts/cleanup_probe_plan.py` and checked by the forecast): smoke has 8 seed commands, 16 timed
commands and 8 semantic oracles; screen has 8 seed commands, 48 timed commands (4 scenarios x 3 repeats x 2 arms x
2 profiles) and 24 semantic oracles. The two arms alternate which runs first; each arm leads half of all comparisons
(per scenario 2:1 at screen scale, accepted; caches are flushed before each command). No automatic retries.

## Independent semantic check

The oracle replays the complete v1 snapshot of the same cycle in dense preorder and requires the probe snapshot to
be exactly its folded image: every retained node, every own/total statistic, and **every bucket row** of every
retained directory (slot range, entry counts, bytes, errors, freshness extrema), recomputed in Python from raw names
with a hash and tiling implementation that is independent of the Rust code. Cross-language agreement is pinned by
shared hash vectors and by the pinned wide window. Missing or extra nodes, extra or missing buckets, wrong windows,
double counting, a directory over 4096 entries that was folded, hardlinks in the fixture, or a layout that differs from
format 2 all fail the cycle. Rust tests cover promotion, demotion, skipping retained siblings, hardlink fallback,
range refresh against a full rescan, and inconsistent snapshots.

## Decision bars (unchanged from the v1 plan, relative to v1)

Owner decisions D2-D14 of [the v1 plan](v1-performance-plan.md) apply as written; no bar is moved after seeing results.

* Peak disk: no confirmed increase. Process RSS: no confirmed increase.
* Full scan: at most +15% time, and then only with at least 2x that percentage as a reduction in another metric
  (a RAM exchange also needs at least 256 MiB absolute savings).
* Folder refresh: no confirmed slowdown, separately for each scenario and profile (retained, omitted, wide).
* A meaningful benefit needs a confirmed speed gain of at least 15%, or a disk gain of at least 25%, or an RSS gain of
  at least 25%. Inconclusive is not a pass.

**"Confirmed" is fixed here, before measurement:** a change counts only if all three paired repeats point the same way
and the median paired change passes the bar. Noise floors below which a change is treated as no change: elapsed time
5% and 0.05 s (the timers resolve 0.01 s and the worker polls up to 50 ms), memory 2% and 1 MiB, disk 5% and 4 KiB.
Regressions use the same floors; benefits must clear the 15% / 25% / 25% bars on all three repeats. Any cap, reclaim,
OOM or swap flag makes that memory comparison inconclusive. Total cgroup memory (cache and slab included) is reported
separately and is not a substitute for the RSS bar.

**Not measured, hence not claimed:** the minimum tested successful memory budget. Every writer runs under the same
8 GiB ceiling, and a low-limit ladder would be dominated by the Python measurement wrapper at small limits. The
budget clause of D3 and the capacity alternative D11 therefore stay open, as in the previous screen, and any report
says so. This also means no claim of lower required RAM follows from a lower RSS.

## Predictions, recorded before any run

* Omitted and wide refresh of the probe take well under 1 s and under 10% of v1's roughly 10.7 s.
* Retained refresh stays below 0.1 s. Full scans stay faster than v1 and within +-10% of the previous probe.
* Process RSS stays under v1's (36.7 MiB earlier). Snapshot pair stays under 1 MiB; bucket rows add tens of KiB.
* Any of these failing is reported as a failure of the idea, not explained away.

## Admission and stops

Unchanged: fresh same-source smoke, then the forecast (10x smoke host time plus 20x cycle time, 30% margin, 12,600 s
step and 14,400 s job caps, observed setup and storage), then the conditional screen. The campaign cap stays 216,000
runner-seconds; this experiment reserves 3,600 for the smoke and 14,400 for the screen. A failed smoke or admission is
recorded and not rerun unchanged; a source change needs a new smoke. Source hashes of everything measured are archived
by the workflow, which also requires the contract tests, the probe's own unit tests and the same binaries in smoke
and screen.

## Known limitations

* A demotion merges a node into the bucket of its name without splitting, so a bucket can exceed 4096 entries until
  its range is next re-tiled by a refresh. The bound on work is therefore soft after demotions.
* A refresh also rescans every sibling of the target that shares its bucket (collateral work, at most about 4096
  entries unless one slot alone exceeds the cap). Renames that move a name to another bucket leave the other bucket
  stale until it is refreshed, exactly like any partial refresh.
* A retained file target rescans its whole parent directory; a deleted target resolves to its nearest live ancestor.
* Regions with hardlinks always fall back to the full root; the timed fixtures have none, so this path is only covered
  by tests. The example still rejects inaccessible entries and nested mounts, limits depth to 256, retained nodes to
  100,000 and buckets to 400,000, needs its output outside the scan root, and keeps the two-name current/previous
  publish gap. There is no GUI, no complete-catalog query and no live drill cache.
* Synthetic fixture, three repeats, unchanged filesystems between seed and refresh (changed-tree performance is
  covered by correctness tests only), and results about a prototype, not a measurement of the installed system.

## Review

Formal independent review of this source must use the actual `claude-opus-5-5` model. Its outcome, or the fact that it
could not be run, is recorded in the final report; a substitute or a simulated review is not offered as one.
