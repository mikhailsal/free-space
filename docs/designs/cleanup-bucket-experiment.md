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
  exactly one root, a root that is not a directory or differs from the root in the metadata, nodes that are not direct
  children of a retained directory, and inconsistent totals.
* Refresh planning: a retained directory target rescans that directory; a retained file target rescans its parent
  directory; any other target resolves to the bucket of its first component below the nearest retained directory and
  rescans only the admitted children of that range (name in range and not a retained child), re-reads the directory's
  own size and scan time, re-tiles the range, and promotes children that became retained. Demoted retained nodes fold
  into the bucket of their own name. A target that no longer exists is planned from the snapshot in the same way (its
  retained directory and bucket are still known); only when that retained directory is gone as well, or has been replaced
  by a file, does the nearest surviving ancestor take over. A relative target, or one below a path that became a file,
  is resolved from its nearest live ancestor. A target containing `..` is refused: once part of the path is missing it
  cannot be resolved lexically, and it must neither pick a bucket nor slip past the root check. A region that held, or
  now shows, hardlinks falls back to a full root rescan and reports `fallback_full_root: true`, as does any plan that
  ends at the root for a narrower request.
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
retained directory (slot range, entry counts, bytes, errors, newest mtime), recomputed in Python from raw names
with a hash and tiling implementation that is independent of the Rust code. Cross-language agreement is pinned by
shared hash vectors, by published FNV-1a vectors in both languages and by the pinned wide windows. Missing or extra
nodes, extra or missing buckets, wrong windows, double counting, a directory over 4096 entries that was folded,
hardlinks in the fixture, or a layout that differs from format 2 all fail the cycle.

That replay checks scan stamps only for internal consistency (a node's oldest stamp equals the oldest of its own, its
retained children's and its buckets' stamps; a bucket carries a stamp exactly when it holds entries). This cannot show
that anything was rescanned: every hosted refresh runs on an unchanged tree, so a refresh that republished its seed
untouched would still equal v1's folded image, which the round-2 review pointed out. **For every refresh cycle the oracle
therefore also receives the seed snapshot the operation started from** (mounted read-only) and the scope the frozen plan
expects (`--seed`, `--expect-scope`, `--expect-boundary`, `--expect-window`), and requires: the same root; a generation
one higher and a newer scan time than the seed's; the same retained nodes and the same bucket layout (an unchanged tree
re-tiles to the seed's layout); a scan stamp strictly newer than the seed's on exactly the items the scope must have
rewritten and identical to the seed's on every other node and bucket; and at least one fresh item. The rewritten set is
everything for a full scan; the boundary and everything below it for a retained refresh; and for a range refresh the
directory that owns the buckets (its own size and time are re-read) and the buckets inside the window. Stamps are whole
seconds, so the host pauses 1.05 s after each seed scan, which makes a later stamp strictly newer.

The merge on changed trees is covered by the Rust tests only: promotion (by size and by entry count), demotion,
skipping retained siblings, hardlink fallback, range refresh against a full rescan, deleted targets, a directory
replaced by a file, targets with `..` or outside the root, a re-read parent directory, inconsistent snapshots and
structural damage.

## Decision bars

Owner decisions D2-D14 of [the v1 plan](v1-performance-plan.md) bind this experiment. Those it uses, quoted verbatim
from the plan's table of current accepted rules:

| Decision | Owner answer / binding result |
| --- | --- |
| D2 peak disk | No confirmed increase beyond calibrated uncertainty |
| D3 memory | No confirmed peak-RSS increase or higher minimum tested successful budget |
| D4 full scan | At most 15% slowdown, conditionally |
| D5 exchange | Other-metric percentage reduction at least twice full-scan slowdown percentage |
| D6 folder refresh | No confirmed slowdown per scenario |
| D8 speed-only benefit | At least 15% confirmed improvement |
| D9 RSS benefit | At least 25% confirmed reduction |
| D10 disk benefit | At least 25% confirmed peak-allocation reduction |
| D11 capacity benefit | Alternative: at least 25% lower tested successful budget, with repeated success and baseline OOM proof |
| D14 RSS exchange floor | RAM savings justify full-scan slowdown only with both 2:1 percentage gain and at least 256 MiB absolute savings |

The plan does not say what "confirmed" or "calibrated uncertainty" mean for three noisy repeats. This file fixes it
**before any measurement of this source**, and more strictly than a reading of "no confirmed increase" that three noisy
repeats could satisfy despite a real regression. The rules below are an explicit pre-registered addition to the plan,
not a quotation of it. They are implemented, with contract tests that were checked by mutation, in
`scripts/cleanup_probe_analysis.py`, which the workflow captures and hashes with the rest of the source. No bar is moved
after seeing results.

* **What is judged.** Only the screen. The smoke has one pair per cell; it decides nothing except that counts, scopes and
  oracles are right. The screen is judged separately for each profile and scenario (eight cells of three paired repeats,
  paired by repeat number).
* **Metrics.** Time: wall seconds of the timed command as GNU `time` reports them (resolution 0.01 s). RSS: the command's
  peak resident set. Peak disk: see below. Total cgroup memory (cache and slab included) is reported, but it holds
  reclaimable page cache: it is neither a bar nor an exchange metric and is not a substitute for the RSS bar.
* **Noise.** In one pair a difference is a change only if it exceeds **both** floors, with v1 as the base: time 5% and
  0.05 s, memory 2% and 1 MiB, disk 5% and 4 KiB. The allowance is `max(relative x v1, absolute)`; anything inside it is
  noise. The floors exist because the timers and samplers have finite resolution.
* **No increase, no slowdown (D2, D3, D6).** Passed only when shown: in every one of the three repeats the probe is at
  most the v1 value plus the allowance. One repeat above that, or a median above it, fails the bar.
* **Benefit (D8 time 15%, D9 RSS 25%, D10 disk 25%).** Passed only when every repeat is better than v1 beyond the noise
  floors by at least the bar and the median is too.
* **Full scan (D4, D5, D14), per profile.** Every repeat must be at most 15% slower than v1; that tolerance gets no noise
  allowance. A repeat is a slowdown only beyond the noise floors; when none is, the bar passes without an exchange.
  Otherwise it needs an exchange: peak disk or process RSS better than v1 beyond noise in every repeat by at least twice
  the **worst** repeat's slowdown, and for RSS also by at least 256 MiB absolute in every repeat. D5 also names
  folder-refresh time as an exchange metric; that exchange is deliberately not used, which can only make the bar harder
  to pass.
* **Flags.** Any cap, reclaim, OOM, swap or worker flag in either arm of a cell makes that cell's memory comparisons
  (D3, D9, the RSS exchange) inconclusive; time and disk stay judged. **Inconclusive is not a pass for any bar**, benefit
  or non-inferiority. Each bar is reported as passed, failed or inconclusive.
* **Overall.** The experiment succeeds only if D2 and D3 pass in all eight cells, D6 in all six refresh cells, the
  full-scan bar in both full cells, and at least one benefit bar (D8, D9 or D10) is confirmed in some cell. A gate that no
  cell reported is not passed.

**Peak disk.** The metric is the output directory's allocated bytes at the start of the command plus the sampled rise of
its filesystem's used bytes (every 50 ms, and once at the end), or the final allocated bytes if larger; v1's value
therefore contains its whole existing database. v1 runs for seconds and is sampled many times. A probe refresh can finish
within one sampling interval, and `store::publish` can hold four snapshot-sized files at once (the current file, the
previous file, a temporary copy of the previous one and the new temporary file), so the sampled value is a lower bound
that may show only the end state. **Every probe disk value is therefore multiplied by 4 before any disk comparison** (D2,
D10 and the disk exchange). v1's sampled value is used as measured, which is a lower bound on its own peak and so
conservative against the probe. The reported tables keep the observed values (sampled peak, final allocation and their
maximum); only the bars use the 4x value.

**Not measured, hence not claimed:** the minimum tested successful memory budget. Every writer runs under the same
8 GiB ceiling, and a low-limit ladder would be dominated by the Python measurement wrapper at small limits. The
budget clause of D3 and the capacity alternative D11 therefore stay open, as in the previous screen, and any report
says so. This also means no claim of lower required RAM follows from a lower RSS.

## Predictions, recorded before any run

Every comparison is against v1's median in the same profile and scenario of the same run; a number from the earlier
screen is context, never a prediction or a gate.

* Omitted and wide refresh of the probe: median under 10% of v1's and well under 1 s, in both profiles.
* Retained refresh: median under 10% of v1's and under 0.1 s. Full scans: faster than v1 in both profiles.
* Process RSS: lower than v1's in every cell. Peak disk: the probe stays inside v1 at four times its observed value in
  every cell, with a wide margin (v1's value contains its whole database).
* The probe's snapshot pair stays under 1 MiB; bucket rows add tens of KiB.
* Any of these failing is reported as a failure of the idea, not explained away.
* The earlier screen (v1 refresh about 10.7 s, RSS 36.7 MiB, probe snapshot 32 KiB) ran on another runner at another
  time; it is the reason these predictions were made, not evidence for them.

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
* Not covered by any test: a non-UTF-8 name through scan, store and refresh end to end, and demotion into an over-cap
  bucket followed by a refresh of that bucket.
* Synthetic fixture, three repeats, unchanged filesystems between seed and refresh (changed-tree behaviour is covered
  by unit tests only, never by a hosted run), and results about a prototype, not a measurement of the installed system.
* The arm that runs first alternates so that each arm leads half of all comparisons and half of each scenario over the
  two profiles. Inside one judged cell (a profile and a scenario) the three screen repeats are ordered 2:1. Caches are
  flushed before every command and the arms run in separate containers, but an order effect is not estimated separately.
* The Python harness (plan, host, worker, oracle, verdict) has contract tests, mutation-checked, and a compile check, but no
  formatter or linter: the repository has none for Python, and its size check counts only languages that have one.
  Adding one is a repository decision, not part of this experiment.
* The hosted flow with the oracle's seed mount and the verdict's artifact reading has not run on real artifacts when this
  file is committed; the smoke is its first run. A layout mistake it exposes in the artifact plumbing is corrected and the
  smoke repeated; the bar functions are never touched after a result is seen.
* The minimum-budget clause of D3 and the capacity alternative D11 are not measured (see above).

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

Record, round 2, 2026-11-04: the commit `59a977f`, which carried the fixes above, was reviewed again by `claude-opus-5-5`
(the CLI's init event, every assistant message and `modelUsage` name only that model; tool-less, one turn, complete diff).
It conceded both disputed round-1 P1 findings, found nothing that would make the smoke fail, mis-report a scope or let a
failed result pass, and answered **GO for the hosted smoke**, with 4 P2 and 8 P3 findings that must be fixed or disclosed
before a screen result is reported. It added that any change of captured source needs a new smoke, so all of them were
fixed before the first smoke was dispatched. Disposition:

* P2 the oracle could not tell a refresh from a no-op (a refresh that republished its seed untouched would have passed,
  freshness was derived from the probe's own stamps, and the text overclaimed): the oracle now proves exactly which nodes
  and buckets were rewritten against the seed, as described above, and the text is corrected.
* P2 the judgment rules were implemented nowhere: `scripts/cleanup_probe_analysis.py` implements them with contract tests,
  and every rule was checked by mutating the code (thresholds, floors, median against every repeat, exchange sizing, flags,
  empty gates, the disk factor, the oracle fields) until each mutation failed a test.
* P2 the full-scan bar was ambiguous: it is now specified point by point, per profile. While freezing it, total cgroup
  memory was removed from the exchange metrics, which D5 and D14 do not name.
* P2 peak disk is a sampled lower bound that favours the faster arm: the 4x rule above, and the disclosure.
* P3 the bullets were presented as the plan's wording, the smoke was not excluded, the allowance was not a formula and the
  timer justification was wrong: the plan is quoted verbatim, and the three points are stated or corrected.
* P3 the predictions leaned on another run's absolutes: restated relative to this run.
* P3 arm order is 2:1 inside a judged cell: accepted and disclosed.
* P3 `..` in the missing part of a target, relative and not-a-directory targets, and a root not tied to the metadata:
  fixed with tests, each shown to fail without its fix. Testing these also exposed a retained directory replaced by a
  file, which failed instead of falling back to its parent; fixed. A target outside the root, which had no test, now has one.
* P3 no Python linter, narrow CI trigger: the trigger now covers the repository checks' inputs. The review proposed adding
  a Python linter and type checker to `scripts/check.sh`, the hook and CI; that changes what every contributor must
  install and what the size rule counts, which is the repository owner's decision, so it is disclosed instead.
* P3 an early worker failure was hidden by a missing output directory: the real cause is reported, with a test.

The review listed what it did not verify: v1's report fields and snapshot schema as the oracle reads them, the nested-mount
handling of the bind-mounted scan root, the helper functions of the container worker, and the hash vectors and windows
(recomputing them was outside its scope; both languages pin them). The smoke is the first check of the first three.
