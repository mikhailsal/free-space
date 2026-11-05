# Cleanup threshold, revision 3: bucketed residual summaries, million-entry screen

Date: 2026-11-04. Source [`ba8281f`](https://github.com/mikhailsal/free-space/commit/ba8281f0a035509019f49459ff69b6a7b5bd2e11) on the research branch `perf/cleanup-bucket-probe`. Hosted runs: [smoke 41912961869](https://github.com/mikhailsal/free-space/actions/runs/41912961869) (565 s, one pair per cell, judges nothing) and [screen 41916505663](https://github.com/mikhailsal/free-space/actions/runs/41916505663) (job time 4,498 s, measured step 4,238 s), both run attempt 1. This is a research screen of a prototype on synthetic trees in a real filesystem. It is not a product qualification and not a measurement of the installed system.

## Verdict

The pre-registered success rule is met: `overall.success` is `true` in the frozen analysis (script sha256 `b7aa8ba3…323b`, byte-identical in git, in the smoke artifact and in the screen artifact, and never edited after any hosted run). The comparison is the bucketed probe against v1 on identical fixtures, judged separately in eight cells (two profiles times four scenarios) from three paired repeats each. The author recomputed every bar and prediction, and re-derived every value from the raw per-case files, with separate code; the round-4 model reviewer (`claude-opus-5-5`, without tools) recomputed the bars and predictions from the repeats in the analysis JSON. Both agree (see "Cross-checks").

| Gate (owner decision) | Cells | Result |
| --- | --- | --- |
| D2 peak disk, no increase | 8 of 8 | passed |
| D3 process RSS, no increase | 8 of 8 | passed; the minimum-tested-budget clause was not measured |
| D6 folder refresh, no slowdown | 6 of 6 refresh cells | passed |
| D4 full scan, at most 15% slower, exchange rules | 2 of 2 full-scan cells | passed with "no slowdown beyond noise": the probe was faster in every repeat, so no exchange was used. As the pre-registration said this report would state: had a slowdown been exchanged, the disk exchange would have been met by a wide margin (v1's disk value contains its whole database), so D4 would in practice have been a plain "at most 15% slower in every repeat" cap |
| Benefit bars D8 (time, 15%), D9 (RSS, 25%), D10 (disk, 25%) | 24 combinations | 23 confirmed; D8 failed for `directories/full` (-8.2%, below 15%); at least one is required |

Open by design and not measured: the minimum tested successful memory budget (clause of D3) and the capacity alternative D11. Nothing here says the probe needs less RAM to run; it says its writer process has a smaller resident set under an 8 GiB ceiling.

The pre-registered predictions are scored beside the verdict and never change it: **31 of 32 hit, one missed.** The miss is `directories/wide`. The prediction was "the probe's median under 10% of v1's and under 1 s"; the probe's median was 1.07 s against v1's 10.91 s (9.8%), so the 1 s bound was missed. As the pre-registration requires, it is reported as a surprise about the idea and is discussed below without being explained away.

## Result in plain terms

All figures are medians of three paired repeats of the 1,001,176-entry fixtures; the probe against v1 in the same run.

* **Refresh of a retained folder** (`large`, 74 entries): 10.59 s to 0.03 s in both profiles (-99.7%).
* **Refresh of an omitted folder** (`small`, 100 entries): 10.56 s (files) and 10.59 s (directories) to 0.03 s in both. In this run the omitted-folder refresh is faster than v1 in both profiles, so the regression against v1 that the previous revision showed does not appear here. (That revision, which kept omitted sizes and counts as one residual per parent directory, needed 36.7 s and 269 s for the same request, because it had to rescan from the root; those figures are from [another run](cleanup-threshold-screen-2026-11-02.md) and are context, not a comparison.) This scenario is the **best case** of the idea: `small` sits alone in the root's lone bucket, so the window holds only its own 100 entries. An omitted path that shares a bucket with many others costs the whole window, as the wide scenario shows, and after demotions a bucket can exceed the 4,096-entry cap until it is next re-tiled.
* **Refresh of a wide bucket** (`bulk/g000050`, 1,001 entries sharing a bucket with three other chunk directories, 4,004 entries in the window): files 10.59 s to 0.18 s (-98.3%); directories 10.91 s to 1.07 s (-90.2%). This is the cost, in this run, of a near-full bucket window (4,004 of at most 4,096 entries).
* **Full scan**: files 62.72 s to 37.01 s (-41.0%); directories 293.36 s to 269.38 s (-8.2%).
* **Process RSS**: 35.4 and 36.8 MiB to 10.9 MiB on full scans (-69% and -70%); 366-370 MiB to 10.8-10.9 MiB on every refresh (-97%). This is the resident set of the writer process under an 8 GiB ceiling, not a lower required RAM.
* **Peak disk** (a sampled lower bound: the output directory's allocated bytes at the start of the command plus the sampled rise of its filesystem's used bytes, as the pre-registration defines it): 432-434 MiB on full scans and 357 MiB on refreshes for v1; for the probe, whose output is two single-file databases of 36 KiB each (`coarse.db` and `coarse.prev.db`; the filesystem allocated one more 4 KiB block to `coarse.db` in most runs), a median of 76 KiB (individual runs 72-76 KiB, and one sampled in-flight publication of 92 KiB). The bars judge the probe at four times its observed value (a refresh can finish between two samples while publication holds extra files), which still gives -99.92% to -99.93%; the largest value ever sampled, 92 KiB, is below four times 76 KiB.
* **Total cgroup memory**, reported but not judged because it contains reclaimable page cache and slab: -12.4% (files) and -6.0% (directories) on full scans, -87% to -91% on refreshes.

The verdict and the direction of every bar were the same in all three repeats of every cell. The repeats are also unusually tight (full-scan repeats differ by at most 0.7%, v1 on the files profile, and by less than 0.1% for the probe; refresh repeats by at most 0.1 s), which fits a throttled and cache-flushed environment but is a reminder that three repeats are screening evidence.

## What was measured

Two profiles, each with **1,001,176 entries**: `files` has 1,000,169 files and 1,007 directories; `directories` has 500,169 files and 501,007 directories. Fixtures live on a 500 GiB sparse ext4 loop volume (capacity 536,166,531,072 bytes), and the retention threshold is 536,166,532 bytes (0.1%). Writers run in containers with 2 CPUs, an 8 GiB cgroup ceiling, no swap, a direct-loop envelope of 32 MiB/s and 2,200/3,600 read/write IOPS (the job's own calibration run reached every limit to within 0.11%), and caches are flushed before every command. The smoke and screen used identical binaries and the identical runtime image (`sha256:d26357ca…a05698`). v2 was not run.

The v1 arm is the `freespace` binary built from `ba8281f`. Against master `afce6fd`, which the pre-registration names as v1, that commit changes no file under `core/`, `cli/src/` or `web/` and adds no dependency: it adds the research example `cli/examples/cleanup_probe/`, a six-line `[[example]]` entry in `cli/Cargo.toml`, benchmark harness scripts, workflows and docs (checked with `git diff --name-only afce6fd ba8281f`). The probe is built from the same source and toolchain.

The probe keeps a node when it is the root, when its subtree allocates at least the threshold or when it holds more than 4,096 entries. The omitted children of a retained directory are folded into buckets chosen by a hash of the child name (4,096 slots tiled into buckets of at most 4,096 entries), and a refresh of an omitted path rescans only the bucket window that holds it. Both profiles ended with **9 retained nodes and 259 buckets** (largest bucket 4,004 entries; two buckets are empty). The retained nodes, read from the preserved probe snapshots, are the root, `bulk`, `large` and, under `large`, `above`, `deep`, `deep/deeper`, `deep/deeper/large`, `equalish` and `group`; `equalish` allocates just above the threshold. The full contract, the evidence rules and the predictions are in the [pre-registration](cleanup-bucket-preregistration-2026-11-04.md), a verbatim copy of the file committed at `ba8281f` before the first run.

Each arm and profile is seeded once by two full scans (8 seed scans in all, excluded from the comparison); every timed command then starts from a fresh copy of that arm's seed, so all twelve timed commands of an arm in a profile (four scenarios times three repeats) share one seed state. (The pre-registration says each scenario is "measured separately from its own two-scan seed"; that sentence is inaccurate about what the host does.) Scenarios, three repeats, arm order alternating:

| Scenario | Command | Probe's reported work (identical in all repeats and both profiles) | v1's reported work |
| --- | --- | --- | --- |
| full | scan root | 2,002,352 visits (two passes), scope `full` | 1,001,176 entries |
| retained | refresh `large` | 148 visits, boundary `large`, scope `retained` | `subtree_entries` 74 |
| omitted | refresh `small` | 202 visits, boundary root, range window `[0,4096)` | `subtree_entries` 100 |
| wide | refresh `bulk/g000050` | 8,010 visits, boundary `bulk`, range window `[945,956)`, 4,004 entries replaced by 4,004 | `subtree_entries` 1,001 |

The run had 48 timed commands, 8 excluded seed scans and 24 independent semantic checks. All 56 cases validated, none carried a memory cap, reclaim, OOM or swap flag, and none had a scope error, sampler error or non-zero exit status (read from the 56 per-case `metrics.json` files, which are in the capsule). All 24 oracles accepted the snapshots, including the seed-based refresh proof: a full scan rewrote 9 nodes and 257 buckets (the other two buckets are empty and carry no stamp), a retained refresh 7 nodes and 2 buckets, and the omitted and wide range refreshes exactly one node (the re-read parent) and one bucket each, while every other node and bucket that carries a stamp kept the seed's stamp. That proves which stored stamps advanced; it does not prove the filesystem was re-read, which rests on the operation's own work report, checked against the frozen plan, and on the elapsed time. The analysis requires each oracle to report that it checked the refresh and rewrote something; the per-scope counts above are the author's reading of the oracle files, and their link to the plan's expectation rests on the plan code.

## Bars cell by cell

`passed`, `failed` and `inconclusive` are the analysis's own words. No cell had a memory flag, so no memory comparison was inconclusive.

| Cell | D2 | D3 | D6 | D4 | D8 | D9 | D10 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| files / full | passed | passed | n/a | passed | passed | passed | passed |
| files / retained | passed | passed | passed | n/a | passed | passed | passed |
| files / omitted | passed | passed | passed | n/a | passed | passed | passed |
| files / wide | passed | passed | passed | n/a | passed | passed | passed |
| directories / full | passed | passed | n/a | passed | **failed** | passed | passed |
| directories / retained | passed | passed | passed | n/a | passed | passed | passed |
| directories / omitted | passed | passed | passed | n/a | passed | passed | passed |
| directories / wide | passed | passed | passed | n/a | passed | passed | passed |

## Absolute measurements

Medians of the three observations of each arm. Command wall time is GNU `time` (0.01 s resolution). Total RAM is the cgroup peak and contains page cache; it is reported, not judged. Peak disk is the observed sampled lower bound for both arms (the probe's judged value is four times higher). The probe's disk values are medians: of its 24 runs, 20 peak at 76 KiB, three at 72 KiB (`directories` retained, omitted and wide, one repeat each) and one sampled an in-flight publication of 92 KiB (`directories/wide`, repeat 3, which ended at 76 KiB). The final allocation is read once after each run, not sampled: 21 runs end at 76 KiB and three at 72 KiB.

| Profile | Operation | Arm | Command wall s | Process RSS MiB | Total RAM MiB (reported only) | Peak disk (observed lower bound) | Final snapshot allocation |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| files | full | v1 | 62.72 | 35.41 | 1827.5 | 432.44 MiB | 242.25 MiB |
| files | full | probe | 37.01 | 10.93 | 1600.7 | 76 KiB | 76 KiB |
| files | retained | v1 | 10.59 | 366.16 | 433.5 | 357.13 MiB | 236.00 MiB |
| files | retained | probe | 0.03 | 10.92 | 38.0 | 76 KiB | 76 KiB |
| files | omitted | v1 | 10.56 | 366.13 | 433.9 | 357.13 MiB | 236.00 MiB |
| files | omitted | probe | 0.03 | 10.92 | 38.0 | 76 KiB | 76 KiB |
| files | wide | v1 | 10.59 | 366.11 | 429.5 | 357.13 MiB | 236.00 MiB |
| files | wide | probe | 0.18 | 10.93 | 44.3 | 76 KiB | 76 KiB |
| directories | full | v1 | 293.36 | 36.78 | 3854.8 | 433.94 MiB | 242.27 MiB |
| directories | full | probe | 269.38 | 10.93 | 3624.7 | 76 KiB | 76 KiB |
| directories | retained | v1 | 10.59 | 366.22 | 431.8 | 357.12 MiB | 235.97 MiB |
| directories | retained | probe | 0.03 | 10.93 | 38.1 | 76 KiB | 76 KiB |
| directories | omitted | v1 | 10.59 | 366.16 | 435.4 | 357.12 MiB | 235.97 MiB |
| directories | omitted | probe | 0.03 | 10.80 | 38.1 | 76 KiB | 76 KiB |
| directories | wide | v1 | 10.91 | 370.16 | 415.3 | 357.12 MiB | 235.97 MiB |
| directories | wide | probe | 1.07 | 10.86 | 53.3 | 76 KiB | 76 KiB |

Per-repeat command times, probe against v1: files/full 37.01, 37.00, 37.03 against 63.00, 62.58, 62.72; directories/full 269.36, 269.38, 269.38 against 293.36, 293.37, 292.87; directories/wide 1.07, 1.08, 1.06 against 10.87, 10.91, 10.95. All 48 raw points are in the [points file](cleanup-bucket-screen-points-2026-11-04.csv).

## Matched percentage changes

Negative means lower for the probe. Each percentage is computed against the matched repeat of the same cell and then the median of the three is taken; this need not equal the ratio of the absolute medians above. `judged` disk uses the probe at four times its observed value, as the bars did; `observed` is as measured.

| Profile | Operation | Time % | Process RSS % | Peak disk % (judged) | Peak disk % (observed) | Total RAM % (reported only) |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| files | full | -41.0 | -69.1 | -99.93 | -99.98 | -12.4 |
| files | retained | -99.7 | -97.0 | -99.92 | -99.98 | -91.2 |
| files | omitted | -99.7 | -97.0 | -99.92 | -99.98 | -91.2 |
| files | wide | -98.3 | -97.0 | -99.92 | -99.98 | -89.6 |
| directories | full | -8.2 | -70.2 | -99.93 | -99.98 | -6.0 |
| directories | retained | -99.7 | -97.0 | -99.92 | -99.98 | -91.2 |
| directories | omitted | -99.7 | -97.1 | -99.92 | -99.98 | -91.3 |
| directories | wide | -90.2 | -97.1 | -99.92 | -99.98 | -87.1 |

## The missed prediction and what it costs

The prediction for omitted and wide refreshes was "the probe's median under 10% of v1's and under 1 s". It applies to four cells (omitted and wide, both profiles); it hit in three and missed in `directories/wide`: above 1 s in all three repeats (1.07, 1.08 and 1.06 s; median 1.07 s) against v1's median of 10.91 s. In `files/wide` the same window costs 0.18 s at the same reported work (8,010 visits, 4,004 entries replaced). The two profiles differ in what the window holds (below), and possibly in more than that; nothing in this run isolates the cause.

A hypothesis that fits the numbers, offered as untested and not as a finding: the fixture alternates directories and files inside every chunk of the `directories` profile, so the window holds about 2,004 directories (four chunk directories and about 2,000 children) and 2,000 files, while the `files` window holds 4,000 files. If each empty directory costs about one cold block read for its entry list, with caches flushed and reads capped at 2,200 IOPS, that is roughly 2,000 / 2,200 = 0.9 s, close to the 0.89 s by which `directories/wide` exceeds `files/wide`. That would require the second of the probe's two passes to be served from cache and the filesystem to store empty directories in a block (no ext4 `inline_data`); neither was checked. The same reading is consistent with the full scans: the `directories` profile has 500,000 more directories than the `files` profile, 500,000 / 2,200 is about 227 s, and both arms are slower on `directories` by about that much (probe 269.38 - 37.01 = 232 s; v1 293.36 - 62.72 = 231 s). If so, both arms pay the same per-directory floor, which would also explain why D8 failed for `directories/full`. All of this is consistent with the data and none of it was tested: whether a smaller bucket cap or warm caches would change the picture would need a new experiment.

The miss does not touch any bar: D6 asks only for no slowdown against v1 and the cell is 90% faster. It matters for the design: a refresh of an omitted path pays for the whole bucket window; in this run the same 4,004-entry window cost 0.18 s or 1.07 s depending on the profile.

## What this does not show

* **It is not a product measurement.** The probe is a research example with a changed information contract: small files and small folders have no identity in the snapshot, only retained nodes and bucket totals. It has no GUI, no complete-catalog queries and no live drill-down, rejects inaccessible entries and nested mounts, caps depth, retained nodes and buckets, and keeps a two-name current/previous publish gap. It is not a drop-in replacement for v1's complete catalog and nothing in this experiment approves it as a product contract.
* **Unchanged trees only.** Every hosted refresh runs on a filesystem that did not change between seed and refresh. Merge behaviour on changed trees (promotion, demotion, deletion, replaced directories, retained siblings) is covered by Rust unit tests and the local functional run, never by a hosted run. Hardlink regions fall back to a full root rescan; the fixtures have no hardlinks, so that path is covered by tests only.
* **Best case versus realistic case.** The omitted scenario is the best case (a window of 100 entries). A refresh also rescans every sibling that shares the target's bucket, up to about 4,096 entries and more after demotions, and a range refresh does not revisit retained siblings, so a renamed name leaves another bucket stale until that bucket is refreshed.
* **Other pre-registered limitations stand** (see its "Known limitations"): a retained file target rescans its whole parent, a target below a deleted retained directory is answered by the nearest surviving ancestor, no test covers a demotion into an over-cap bucket followed by a refresh of that bucket, and inside one judged cell the three repeats are ordered 2:1 by arm (each arm leads half of all comparisons; no order effect was estimated). The pre-registration also says no test follows a non-UTF-8 name through scan, store and refresh end to end. The hosted fixture does contain one such name, `large/raw-\xff`, an omitted child of `large` that is folded into a bucket: every full scan and every retained refresh of `large` walked it, and the oracles accepted the snapshots. No refresh targeted it and its stored identity was not checked.
* **Synthetic fixtures, one runner type, three repeats.** The trees are generated, the volume is a sparse loop image with a throttled I/O envelope (32 MiB/s, 2,200/3,600 IOPS) and flushed caches, so absolute times scale with that envelope; equivalence to a local disk is not inferred. Three paired repeats that share seed state can show a consistent direction, not a confidence interval.
* **Memory is a resident-set result.** The bars judge the writer process's peak RSS. The minimum tested successful budget (D3 clause) and the capacity alternative D11 were not measured, and total cgroup memory, which contains reclaimable cache, moved little on full scans (-12.4% and -6.0%). No claim of a lower required RAM follows.
* **Disk is a lower bound, judged conservatively.** Peak disk is sampled every 50 ms, so a short probe refresh can show only its end state (two runs sampled a publication in flight, and one of them raised the sampled peak to 92 KiB); the bars use four times the probe's value. v1's value contains its whole existing database, which is part of why the margin is so large.
* **The oracle proves stamps, not reads.** See above.

## Cross-checks

The frozen analysis was cross-checked outside its own code. The author's [independent check](cleanup-bucket-independent-check-2026-11-04.py), written from the pre-registration's rules and not importing the analysis, (1) re-derives all 7 values of each of the 48 timed cases from the raw `time.json` and `metrics.json` files of the capsule and finds no mismatch with the points file, and (2) recomputes all 48 bar verdicts, the 32 predictions and the overall rule from the points file with exact fractions and finds no disagreement. It does not implement the exchange branch of D4 or the memory-flag rule, neither of which this run reached. It reproduces from the committed capsule with two commands, unpack and run (see its docstring). The reviewer of round 4 recomputed the bars and predictions by hand from the repeats in the analysis JSON and agreed with every one, and compared every median, matched change, per-repeat time and table cell of the first draft of this report with the JSON and the points file without finding a discrepancy in the tables. It did find errors in the prose (a wrong cell count for the missed prediction, an inaccurate seeding sentence, imprecise disk wording), which are corrected here and listed under the review record. The round-4 reviewer saw the verdict half of the check; the extraction half was added afterwards and was seen by a model only in round 5, which had its code and output but not the raw per-case files it reads.

## Cost and protocol history

Both runs were dispatched from `ba8281f` (the same head SHA in GitHub's run records), each as run attempt 1; the captured source of the two artifacts is byte-identical (229 files, equal to git), as are the binaries, the runtime image and the analysis script. The smoke was valid with bars and predictions masked. The screen was admitted by the in-job forecast made from the smoke (projected 4,420 s of host time, 5,746 s with a 30% margin, against a 12,600 s step cap; the same projection was made locally before dispatch and is archived as the preflight), and the measured step took 4,238 s. No run was repeated, and no source, workflow, binary, image or analysis script changed between the smoke and the screen.

Runner time for this experiment is **5,292 s (1 h 28 min 12 s)**: smoke 565 s, screen 4,498 s and four dev-CI runs of 75, 46, 54 and 54 s. The owner-approved campaign stands at **66,495 s (18 h 28 min 15 s) of 216,000 s (60 h)**. Mixed bases: the earlier entries of the ledger do not include CI runs while this experiment's total does, to be conservative; on the earlier basis the campaign total would be 66,266 s. No local application scan, installation, production-data deletion or master merge was performed; local work was limited to formatting, lints, contract and unit tests, and one functional run of the debug build on a 100,000-entry tree for correctness (no timing recorded or used). The screen's job log shows no `docker rm` warning.

## Review record

The source was reviewed three times before the runs by `claude-opus-5-5` (verified from the CLI's init event, every assistant message and `modelUsage`, which name only that model; tool-less, one turn each, as the archived outputs show). Round 1 on `b141a3e`: NO-GO (2 P1, 7 P2, 11 P3), fixed or disclosed in `59a977f`. Round 2 on `59a977f`: GO for the smoke (4 P2, 8 P3), fixed or disclosed in `0942a50`. Round 3 on `0942a50`, a delta review: GO (2 P2, 8 P3), fixed or disclosed in `ba8281f` before the first dispatch. The round-3 fixes in `ba8281f` were **not reviewed by a model before the runs**; their evidence at that time was local (contract tests mutation-checked, 124 Python tests, pyflakes).

**Round 4, after the runs.** `claude-opus-5-5` (same verification, one turn, no tool use) reviewed this report with its evidence and, for the first time, the `ba8281f` delta. Verdict A: publish after the listed fixes (Part A: 1 P1, 7 P2, 7 P3). Verdict B: nothing in the delta puts the reported verdict in doubt (Part B: 9 P3, all disclosures). Every Part A finding was applied to this report: the wrong cell count of the miss (the P1), the inaccurate seeding sentence, the "removes entirely" overclaim and the unstated best case, the partial-credit and causal wording around the miss, the per-repeat disk wording (and a wrong decomposition of the 76 KiB that the reviewer's check exposed and the raw files corrected), the incomplete review record, the protocol wording about the smoke and screen, and the P3 wording and disclosure items. Part B disclosures, which change no result:

* The pre-registration's sentence on seeding is inaccurate (stated above); the copy in this directory is verbatim and is not edited.
* "At least one fresh item" in the oracle adds nothing for a real snapshot, because the boundary check already requires a fresh retained boundary; the round-3 wording that "nothing enforced a fresh item" overstated the gap.
* The oracle's JSON does not echo the scope, boundary and window it was given, and the analysis requires only that the oracle report `refresh_checked` and a non-zero number of fresh items, not the per-scope counts; that link rests on the plan's expectation, so two coinciding bugs (a wrong plan window and a probe that stamps the same wrong bucket) could pass the oracle. The plan's windows equal those pre-registered from the independent Python reference (`[945, 956)` at 1M), which bounds but does not exclude such a coincidence.
* Empty buckets carry no stamp, so "nothing else advanced" means nothing else that carries a stamp.
* No test pins the "under 1 s" prediction bound at exactly 1.0 s with the relative half holding; the reported 1.07 s miss is not affected, and the round-3 claim of tests "for every boundary" was too strong.
* The prediction wording was loosened in the unreviewed delta before any run ("well under 1 s" became "under 1 s"; "failure of the idea" became "surprise about the idea"); the reviewed version was stricter, and 1.07 s misses both.
* Exact arithmetic starts at `metrics.json`; the worker's conversion of GNU `time` output is not covered, and all values in this run are clean two-decimal numbers.
* The host's failure handling can still mask a writer's log if `docker inspect` returns malformed output or times out, and `docker rm -f` warnings reach only the job log (the screen's shows none).
* The analysis's smoke cross-check covers `runtime.json` only; the equality of binaries and source between smoke and screen is enforced by the workflow and was also checked by the author (above).

**Round 5, after the corrections.** `claude-opus-5-5` (same verification, one turn, no tools) re-reviewed the corrected report against the round-4 findings, the analysis JSON, the points, the proof, the pre-registration, the independent check with its output and a list of author-checked facts. It confirmed all 48 bars, `overall.success: true` and 31 of 32 predictions, found no table error (it spot-recomputed medians, unit conversions, the cgroup-memory column, the per-repeat times and the arithmetic of the hypothesis) and judged the verdict section faithful to the pre-registration's reporting duties. Disposition of round 4: thirteen Part A findings fixed, one fixed but worded incorrectly (the cell count of the miss carried a self-contradicting parenthesis) and one partly fixed (a surviving "removes" in the evidence list). Decision: publish after the listed fixes, with twelve new findings (3 P2, 9 P3), all wording or disclosure: the self-contradicting "1.07 s in all three repeats", the surviving "removes", review-record sentences made stale by this round, the missing "surprise about the idea" wording, the non-UTF-8 limitation (the hosted fixture does contain such a name), two generalisations around the plain-terms wide bullet and the miss, the understated seed sharing, the count of in-flight disk samples and the definition of peak disk, the scope and reproduction steps of the independent check, the softened oracle disclosure, the attribution of the verdict recomputation, and an over-broad "nothing changed". All twelve were applied here. Points the reviewer could not verify were checked by the author against the artifacts: the nine retained nodes, the 2-CPU limit, the calibration, the logical sizes of the two database files and the model identity, turn count and tool absence of all five rounds. **These last text changes were checked by the author but not model-reviewed.**

Raw prompts, inputs, outputs and extracted results of all five rounds are in the [review archive](cleanup-bucket-reviews-2026-11-04.tar.gz); the dispositions of rounds 1 to 3 are in the pre-registration.

## Evidence

* [Analysis of the screen, all raw points](cleanup-bucket-screen-analysis-2026-11-04.json) and [points as CSV](cleanup-bucket-screen-points-2026-11-04.csv)
* [Raw artifact capsule](cleanup-bucket-screen-2026-11-04.tar.gz): every artifact member under 300 KB except the captured source (which is byte-identical to git), including all per-case `metrics.json`, `time.json` and oracle files, `SHA256SUMS` of the members, and `ZIP-INVENTORY.txt` with sizes and CRC-32 of every member of the original ZIP. The two v1 snapshot dumps (46 MB and 48 MB), the two binaries and the runtime image are omitted for size and are covered by that inventory. The screen ZIP's sha256 was verified against GitHub's recorded digest; the smoke ZIP was not independently hashed.
* [Independent check](cleanup-bucket-independent-check-2026-11-04.py) of extraction and verdict, [proof](cleanup-bucket-screen-proof-2026-11-04.json) (run, artifact and script hashes, integrity counts, runner seconds)
* [Pre-registration](cleanup-bucket-preregistration-2026-11-04.md), [frozen analysis script](cleanup-bucket-analysis-2026-11-04.py), [screen preflight admission](cleanup-bucket-screen-preflight-2026-11-04.json)
* [Smoke analysis](cleanup-bucket-smoke-analysis-2026-11-04.json) and [smoke capsule](cleanup-bucket-smoke-2026-11-04.tar.gz) (one pair per cell; judges nothing)
* [Reviews, rounds 1 to 5](cleanup-bucket-reviews-2026-11-04.tar.gz)
* Source: [`ba8281f`](https://github.com/mikhailsal/free-space/commit/ba8281f0a035509019f49459ff69b6a7b5bd2e11) on `perf/cleanup-bucket-probe`; the earlier [threshold screen](cleanup-threshold-screen-2026-11-02.md) is context for the omitted-refresh regression that revision showed, which does not appear in this run; its numbers are never pooled with these.
