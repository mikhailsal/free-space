# Cleanup threshold, revision 3: bucketed residual summaries (pre-registration)

Date: 2026-11-04. Prospective research experiment on a research example, not a product backend. No measurement of this
source exists when this file is committed. It extends [the threshold experiment](cleanup-threshold-experiment.md), whose
fixture, volume, I/O envelope, memory ceiling, cleanup, admission and forecast rules apply unchanged except where this
file says otherwise. No installed database, configuration, timer, master branch or release is changed. Benchmarks, scans and full builds
run only on disposable GitHub-hosted runners; local work was limited to formatting, lints, the Python contract tests and the
probe's own unit tests (a few seconds each, at lowest CPU priority).

## Why a new revision

The previous million-entry screen (run 41485277207, source `678e998`) showed that a coarse snapshot wins on disk
(32 KiB against 242 MiB for v1), process RSS (8.9 against 36.7 MiB) and full-scan time (-40% files, -8% directories),
and that refreshing a retained folder takes 0.02-0.03 s against 10.7 s. It also showed one material regression:
refreshing an *omitted* folder took 36.7 s (files) and 269 s (directories) against 10.7 s for v1, because the old
anonymous residual cannot say which part of it belongs to the target, so the only exact answer was to rescan the nearest
retained boundary, the root.

This revision tests the grouping idea from that discussion: do not keep one anonymous residual per retained directory,
but fold the omitted children into a bounded number of **buckets selected by a hash of the child name**. A refresh of
an omitted path then rescans the bucket that holds its first folded component. The old contribution is known (the
bucket's stored totals) and is replaced by a recomputation from a rescan of exactly the members of that bucket and of
the directory's own size. Siblings that share the bucket are refreshed as a side effect. What lies outside the rescanned
range (other buckets, retained siblings) is not revisited, as with any partial refresh; see the limitations.

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
  records the format, slot bits and bucket cap; a reader rejects other layouts, gaps, overlaps, a snapshot without
  exactly one root, nodes that are not direct children of a retained directory, and inconsistent totals.
* Refresh planning: a retained directory target rescans that directory; a retained file target rescans its parent
  directory; any other target resolves to the bucket of its first component below the nearest retained directory and
  rescans only the admitted children of that range (name in range and not a retained child), re-reads the directory's
  own size and scan time, re-tiles the range, and promotes children that became retained. Demoted retained nodes fold
  into the bucket of their own name. A target that no longer exists is planned from the snapshot in the same way (its
  retained directory and bucket are still known); only when that retained directory is gone as well does the nearest
  surviving ancestor take over. A region that held, or now shows, hardlinks falls back to a full root rescan and
  reports `fallback_full_root: true`, as does any plan that ends at the root for a narrower request.
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
`[945, 956)` at 1M; the Rust tests pin the same two windows. It is added so the result is not flattered by the
single-bucket `small` case: the root's lone bucket holds only `small`, whereas `g000050` shares a bucket with three
other chunk directories, so a refresh visits four times the entries v1 visits for the same path. v1 reports `entries`
for a scan and `subtree_entries` (74 / 100 / 1001) for the refreshes. The plan derives the chunk size and the entry
counts from the fixture builder's own constants and arithmetic, and its contract tests cover both profiles.

Run counts (frozen in `scripts/cleanup_probe_plan.py` and checked by the forecast): smoke has 8 seed commands, 16 timed
commands and 8 semantic oracles; screen has 8 seed commands, 48 timed commands (4 scenarios x 3 repeats x 2 arms x
2 profiles) and 24 semantic oracles. The arm that runs first alternates with scenario, repeat and profile, so each arm
leads half of all comparisons and half of those of every scenario over the two profiles (smoke 4:4, screen 12:12);
caches are flushed before each command. No automatic retries.

## Independent semantic check

The oracle replays the complete v1 snapshot of the same cycle in dense preorder and requires the probe snapshot to
be exactly its folded image: every retained node, every own/total statistic, and **every bucket row** of every
retained directory (slot range, entry counts, bytes, errors, freshness extrema), recomputed in Python from raw names
with a hash and tiling implementation that is independent of the Rust code. Cross-language agreement is pinned by
shared hash vectors, by published FNV-1a vectors in both languages and by the pinned wide windows. Missing or extra
nodes, extra or missing buckets, wrong windows, double counting, a directory over 4096 entries that was folded,
hardlinks in the fixture, or a layout that differs from format 2 all fail the cycle. The hosted cycles refresh
unchanged trees, where a range refresh re-tiles to the layout of the seed; the oracle therefore requires the
from-scratch greedy tiling, and the merge on changed trees is covered by the Rust tests only: promotion (by size and by
entry count), demotion, skipping retained siblings, hardlink fallback, range refresh against a full rescan, deleted
targets, a re-read parent directory, inconsistent snapshots and structural damage.

## Decision bars (unchanged from the v1 plan, relative to v1)

Owner decisions D2-D14 of [the v1 plan](v1-performance-plan.md) apply as written; no bar is moved after seeing results.

* Peak disk: no increase. Process RSS: no increase.
* Full scan: at most +15% time, and then only with at least 2x that percentage as a reduction in another metric
  (a RAM exchange also needs at least 256 MiB absolute savings).
* Folder refresh: no slowdown, separately for each scenario and profile (retained, omitted, wide).
* A meaningful benefit needs a confirmed speed gain of at least 15%, or a disk gain of at least 25%, or an RSS gain of
  at least 25%.

**How a bar is judged, fixed here before measurement** (the review of this file made the evidence rule for the
non-inferiority bars stricter than the plan's "no confirmed increase" wording, which three noisy repeats could satisfy
despite a real regression):

* A difference is noise unless it exceeds **both** the relative and the absolute floor: elapsed time 5% and 0.05 s
  (the timers resolve 0.01 s and the worker polls up to 50 ms), memory 2% and 1 MiB, disk 5% and 4 KiB.
* A benefit bar (speed 15%, RSS 25%, disk 25%) is passed only when all three paired repeats clear the bar and the median
  does too (a confirmed benefit).
* A no-increase or no-slowdown bar is passed only when affirmatively shown: in every paired repeat the probe is at most
  the v1 value plus the noise floor. One repeat above that floor, or a median above it, fails the bar. A bar with its
  own explicit tolerance (the +15% full-scan limit) is judged against that tolerance without adding a floor.
* Any cap, reclaim, OOM or swap flag makes that memory comparison inconclusive, and **inconclusive is not a pass for any
  bar**, benefit or non-inferiority. Each bar is reported as passed, failed or inconclusive.
* Total cgroup memory (cache and slab included) is reported separately and is not a substitute for the RSS bar.

**Not measured, hence not claimed:** the minimum tested successful memory budget. Every writer runs under the same
8 GiB ceiling, and a low-limit ladder would be dominated by the Python measurement wrapper at small limits. The
budget clause of D3 and the capacity alternative D11 therefore stay open, as in the previous screen, and any report
says so. This also means no claim of lower required RAM follows from a lower RSS.

## Predictions, recorded before any run

* Omitted and wide refresh of the probe take well under 1 s and under 10% of v1's roughly 10.7 s.
* Retained refresh stays below 0.1 s. Full scans stay faster than v1.
* Process RSS stays under v1's (36.7 MiB earlier). Snapshot pair stays under 1 MiB; bucket rows add tens of KiB.
* Any of these failing is reported as a failure of the idea, not explained away.
* Absolute times and sizes of the earlier screen are a different runner and run; they are context for the numbers, not
  a prediction and not a gate.

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
* A range refresh skips retained siblings by name: a retained sibling that was deleted, shrunk or replaced stays
  stale until its own directory is refreshed. It re-reads the rescanned directory's own size but no other ancestor's.
* A retained file target rescans its whole parent directory. A target below a deleted retained directory is answered
  by the nearest surviving ancestor, which is the root (a full rescan) when nothing deeper survives.
* Regions with hardlinks always fall back to the full root; the timed fixtures have none, so this path is only covered
  by tests. The example still rejects inaccessible entries and nested mounts, limits depth to 256, retained nodes to
  100,000 and buckets to 400,000 (the bucket cap is checked when the snapshot is written), needs its output outside
  the scan root, and keeps the two-name current/previous publish gap. There is no GUI, no complete-catalog query and
  no live drill cache.
* Not covered by any test: a non-UTF-8 name through scan, store and refresh end to end, a retained target replaced by
  a file, a target outside the root, and demotion into an over-cap bucket followed by a refresh of that bucket.
* Synthetic fixture, three repeats, unchanged filesystems between seed and refresh (changed-tree behaviour is covered
  by unit tests only, never by a hosted run), and results about a prototype, not a measurement of the installed system.

## Review

Formal independent review of this source must use the actual `claude-opus-5-5` model. Its outcome, or the fact that it
could not be run, is recorded in the final report; a substitute or a simulated review is not offered as one.

Record, 2026-11-04: the commit `b141a3e` was reviewed by `claude-opus-5-5` (verified from the CLI's init event and
`modelUsage`, which listed no other model), tool-less, on the complete diff. It answered NO-GO with 2 P1, 7 P2 and
11 P3 findings. Disposition before any dispatch:

* P1 "clippy `manual_range_contains`": not reproduced. `cargo clippy -D warnings` passed on that exact commit locally
  and in the pre-push hook (the lint only fires for constant bounds). The comparisons now use `contains` anyway.
* P1 "`fixture['scale']` is never produced": the key has been in the receipt since the fixture's first version
  (`scale=count`) and was outside the diff. The plan's contract tests now build receipts with the fixture's own
  arithmetic for both profiles instead of an invented shape.
* P2 deleted targets lost bucket targeting and misreported the fallback, and P2 the parent's own size was never
  re-read: both fixed with regression tests, each shown to fail when its fix is removed.
* P2 non-inferiority bars were judged more leniently than benefits, with an ambiguous noise rule, and P3 a cross-run
  prediction: the evidence rules above and the reworded prediction.
* P2 wide windows pinned only in Python: pinned in Rust as well. P3 hash spec pinned only by mutual agreement: published
  FNV-1a vectors in both languages.
* P2 lints skipped test code and P3 CI coverage: the example is a test target, so the repository's clippy gate (hook
  and CI) lints its tests, proven with a planted violation; the dev CI runs the repository's own `scripts/check.sh rust
  size`.
* P2 `directories` profile assumptions and P3 arm order identical in both profiles: chunk size derived from the
  fixture constant, both profiles covered, order now depends on the profile.
* P2 timed refreshes never exercise the merge, P3 retained siblings not revisited, P3 untested paths and P3
  overclaiming wording: disclosed above and reworded. P3 structural checks in the store reader were added.
* P3 actions pinned by tag: left as is, consistent with every workflow of this repository (read-only token, no secrets).
* P3 bucket cap reported late: left, listed above.
