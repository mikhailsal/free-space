# Cleanup threshold: controlled million-entry screen

Date: 2026-11-02. Run [41485277207](https://github.com/mikhailsal/free-space/actions/runs/41485277207), source `678e998e47020576224337b8fe8ab941f3df2bef`, actual aggregate job time 7,762 seconds. This is a research screen on synthetic trees in a real filesystem, not production qualification or a measurement of the installed local system.

Two profiles each contain **1,001,176 entries**. The files profile has 1,000,169 regular files and 1,007 directories. The directory profile has 500,169 regular files and 501,007 directories. Each arm starts from its own two-scan seed. There are 12 excluded seed writers, 54 timed writers and 18 independent semantic checks; each arm takes each of the three positions across the three repeats. Cases are unchanged filesystems, not arbitrary changed-tree performance.

Controls are the v1 and v2 implementations from unchanged master `afce6fdfc007a2c6914cab61aab4d84daccbb99b`, built with the research example from the same source/compiler. All timed writer arms use 2 CPUs, an 8 GiB cgroup ceiling, no swap, and a direct-loop envelope of 32 MiB/s plus 2,200/3,600 read/write IOPS. Source, binary hashes and the exact smoke runtime image/package metadata match; host total/available memory and CPU headroom were checked. The ceiling is not a claim of minimum required RAM.

The configurable threshold is 1,000 ppm (0.1%) of measured filesystem capacity. The volume is a 500 GiB logical ext4 loop image: real preallocated inner blocks need little physical outer backing while unwritten. Ordinary sparse apparent-only length is a negative control. Both inner and outer free space are guarded; completed-case files are deleted and trimmed outside writer timing. This controls metadata volume and the I/O envelope; it does not establish physical equivalence to the local disk.

The whole runner test volume's outer backing high-water mark was
5,253,033,984 bytes (about 4.89 GiB), including fixtures and all backend states,
while its logical length was 500 GiB. The four large controls had
2,144,686,080 actual allocated inner bytes; their outer growth was 98,304 bytes
in the files profile and 36,864 bytes in the directory profile, above the
already formatted volume's fixed metadata. This validates the unwritten-extent
workaround for this metadata scanner. It does not require storing or reading
500 GiB of file contents on the runner or the local computer.


## Practical result and integration decision

The prototype stored **nine named nodes** in each 1,001,176-entry profile, with
all omitted sizes and counts preserved as parent residuals. Its current and
previous coarse files together occupied **32 KiB at the endpoint**, compared
with about **242 MiB for v1** after full scan and **626–628 MiB for v2**. The
v2 endpoint follows two full seed scans plus the timed third full scan; it
therefore includes its three-scan allocation/history policy, rather than a
normalized two-image history. V2 refresh cases instead begin from the two-full-
scan seed and end at about 520–522 MiB after a sparse refresh. No history-
normalizing compaction/vacuum was performed, so the v2 storage ratio includes
this policy difference as well as the different information contract. Sampled
output peaks were about 432–434 MiB and 1,124–1,125 MiB respectively, versus a
32 KiB **median observed lower bound** for the prototype. Across all timed
prototype operations the largest sampled peak was **52 KiB**, in omitted-target
refresh. Neither reading is a certified maximum for its brief publication; final allocation is independently
measured, and loss of per-object detail is the primary storage trade.

Full scan improved versus v1: files took 61.41→36.73 s and total peak memory
1,826.9→1,600.5 MiB; directories took 293.55→269.04 s and
3,854.8→3,624.4 MiB. Median matched worker-time changes are −40.23%/−8.33%, and
total-memory changes are −12.39%/−5.98%. The process-RSS decrease is much larger
than the total-memory decrease; retaining a small snapshot still requires
walking and statting the input tree. These deltas describe the Linux-charged
footprint without observed pressure, rather than irreducible memory demand.

The exit-time `memory.stat` samples show why that distinction matters. For the
prototype, reclaimable slab and file cache dominate the charged total. The
files/full endpoint had about 8 MiB anonymous memory, 1,285 MiB reclaimable slab
and 300 MiB file cache; the directories/omitted endpoint had about 8 MiB
anonymous memory, 1,343 MiB reclaimable slab and 2,266 MiB file cache. These
endpoint samples were taken after the command exited, include the measurement
wrapper, and are **not a simultaneous decomposition of the recorded peak or
a measurement of the application heap**. Caches may be reclaimed under
pressure; required RAM and the timing/ranking under tighter limits remain
unmeasured in this screen.

The material regression is exact persisted refresh inside an omitted subtree.
The directory case took **269.02 s versus 10.70 s for v1 and 0.13 s for v2**
(GNU command-wall medians). Total peak memory was **3,624.2 MiB versus 435.0 MiB
and 45.5 MiB**. Relative to v1 this is about 25× the time and 8.3× the memory;
the files case also regresses (36.72 versus 10.66 s and 1,600.5 versus
434.4 MiB). Refreshing a retained candidate stays short, around 0.02–0.03 s for
the prototype versus 0.12 s for v2 and 10.65 s for v1 here. These short timings
have 0.01 s command resolution and worker-polling uncertainty; they do not
establish precise application/API latency or a universal speed ratio.

This is useful evidence for a cleanup-specific coarse overview, with an
explicitly separate live detail operation. It is not a uniform four-metric
improvement for the old arbitrary-folder refresh contract. Production work
includes the nine mapped integration units: core scan/error/mount handling;
roots and self-exclusion; byte paths and hardlinks; versioned representation;
queries/coverage; live refinement; exact refresh/history/atomic publication;
CLI/API/UI; and lifecycle/validation. A capped aggregate frontier is an
unmeasured alternative when repeated exact refreshes of selected small paths
justify additional retained state. Neither option is implemented or approved
as a product contract by this experiment.

Validation passed: 21 Python contracts and 318 Rust tests (one ignored), all
66 writer cases including seeds, all 18 independent semantic oracles, source
byte hashes, same binaries/runtime, actual host headroom, I/O calibration and
owned cleanup. No timed cap/reclaim/OOM/swap flags were observed. Actual
prototype-study runner usage is **9,192 s (2 h 33 min 12 s)** across its two
smokes and this screen; the entire owner-approved campaign is **61,203 s
(17 h 0 min 3 s) of 216,000 s (60 h)**. No local application scan, installation,
backend cutover, production-data deletion or master merge was performed.

## Absolute measurements

All cells below are medians of the three observations for that exact arm/profile/operation. Total memory is cgroup peak including the wrapper, filesystem cache and kernel memory; process RSS is shown separately. Worker elapsed includes completion detection by a timeout wait that can add up to about 50 ms. GNU command wall has 0.01 s resolution. Both are retained; small fast-refresh differences are not precise latency gains.

| Profile | Operation | Arm | Worker wall s | GNU command wall s | Total RAM MiB | Process RSS MiB | Sampled output peak lower bound MiB | Final snapshot allocation MiB |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| files | full | v1 | 61.4747 | 61.41 | 1826.918 | 36.664 | 431.75391 | 242.22266 |
| files | full | v2 | 129.6745 | 129.63 | 2758.887 | 39.965 | 1125.40625 | 627.68750 |
| files | full | probe | 36.7526 | 36.73 | 1600.535 | 8.887 | 0.03125 | 0.03125 |
| files | retained | v1 | 10.6926 | 10.65 | 434.672 | 366.176 | 357.10547 | 235.98438 |
| files | retained | v2 | 0.1651 | 0.12 | 45.324 | 15.105 | 522.03125 | 522.02734 |
| files | retained | probe | 0.0328 | 0.02 | 36.117 | 8.828 | 0.03125 | 0.03125 |
| files | omitted | v1 | 10.6928 | 10.66 | 434.395 | 366.148 | 357.10547 | 235.98438 |
| files | omitted | v2 | 0.1651 | 0.12 | 45.543 | 15.023 | 522.03125 | 522.02734 |
| files | omitted | probe | 36.7457 | 36.72 | 1600.531 | 8.805 | 0.03125 | 0.03125 |
| directories | full | v1 | 293.5997 | 293.55 | 3854.812 | 36.777 | 433.66016 | 242.51953 |
| directories | full | v2 | 363.8105 | 363.78 | 4745.465 | 39.191 | 1124.24609 | 626.34766 |
| directories | full | probe | 269.0547 | 269.04 | 3624.418 | 8.926 | 0.03125 | 0.03125 |
| directories | retained | v1 | 10.6900 | 10.65 | 434.723 | 366.391 | 357.36719 | 236.09766 |
| directories | retained | v2 | 0.1650 | 0.12 | 45.531 | 15.027 | 520.46875 | 520.46484 |
| directories | retained | probe | 0.0649 | 0.03 | 36.293 | 8.875 | 0.03125 | 0.03125 |
| directories | omitted | v1 | 10.7404 | 10.70 | 434.969 | 366.297 | 357.36719 | 236.09766 |
| directories | omitted | v2 | 0.1650 | 0.13 | 45.543 | 15.027 | 520.46875 | 520.46484 |
| directories | omitted | probe | 269.0682 | 269.02 | 3624.172 | 8.922 | 0.03125 | 0.03125 |

## Matched percentage changes

Negative means lower for the prototype. Compute each percentage against its matched repeat, then take the median; this need not equal the ratio of the absolute medians. Three repeats are screening evidence, not a precision guarantee or a confidence interval.

| Profile | Operation | Reference | Worker wall % | GNU command wall % | Total RAM % | Sampled output peak % | Final allocation % | Pressure flagged |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| files | full | v1 | -40.23 | -40.22 | -12.39 | -99.9928 | -99.9871 | False |
| files | full | v2 | -71.67 | -71.68 | -41.99 | -99.9972 | -99.9950 | False |
| files | retained | v1 | -99.6929 | -99.8119 | -91.69 | -99.9912 | -99.9868 | False |
| files | retained | v2 | -80.10 | -81.82 | -20.40 | -99.9940 | -99.9940 | False |
| files | omitted | v1 | +243.65 | +244.47 | +268.45 | -99.9912 | -99.9868 | False |
| files | omitted | v2 | +22157.83 | +30508.33 | +3413.91 | -99.9940 | -99.9940 | False |
| directories | full | v1 | -8.33 | -8.32 | -5.98 | -99.9928 | -99.9871 | False |
| directories | full | v2 | -26.05 | -26.04 | -23.62 | -99.9972 | -99.9950 | False |
| directories | retained | v1 | -99.3925 | -99.7183 | -91.65 | -99.9913 | -99.9868 | False |
| directories | retained | v2 | -60.66 | -75.00 | -20.29 | -99.9940 | -99.9940 | False |
| directories | omitted | v1 | +2405.48 | +2414.67 | +733.20 | -99.9913 | -99.9868 | False |
| directories | omitted | v2 | +162944.50 | +206838.46 | +7857.41 | -99.9940 | -99.9940 | False |

## Work and interpretation

Retained-folder refresh requests `large`, a 74-entry subtree; the prototype visits 148 entries over its two metadata passes. Omitted-folder refresh requests `small`, a 100-entry subtree. V1/v2 report 100 scanned entries; the prototype has discarded its old contribution and selects the root, visiting 2,002,352 entries. Exact work sizes/boundaries/fallback flags and snapshot totals are checked. The latter scenario is a material change to the update contract, not an equivalent fast incremental update.

Every coarse `own + retained children + rest` total, kind count and error aggregate matches the independent full-v1 reference; v2 root/kind counts, independently measured capacity/threshold, named control classification and raw paths are checked. Newest file-mtime extrema are compared; scan-time extrema are checked structurally, not asserted equal across separately timed scans. Rust tests cover exceptional hardlink/fallback/depth/threshold cases; the timed fixtures have no hardlinks and no denied/nested-mount regions.

Coarse storage deliberately omits small-file AND small-directory identities. This is an information-contract and traversal change while retaining Rust/SQLite; it is not lossless compression or an isolated SQL-engine comparison. The prototype still stats all entries twice, with a bounded exceptional hardlink-owner map. Full-scan RAM and both refresh RAM measurements must be read separately; process RSS alone is not total memory.

Output peak is initial allocated snapshot plus maximum observed inner-filesystem growth sampled every 50 ms. It excludes initial fixture bytes but includes filesystem metadata and possible open-unlinked output. It is a sampled lower bound, particularly for brief coarse publication; independent file maxima are not added. Final allocation is a separate endpoint measurement. Observers, fixtures, seed copies, trims and evidence compression are outside writer time/RAM but inside job accounting.

History is not equivalent: v1 refresh preserves its previous full-scan baseline; the prototype copies its immediately preceding coarse image. It has a two-name current/previous crash gap, rejects inaccessible/nested-mount entries, requires output outside the scan root, caps depth/retained/link-owner state and does not implement existing complete-catalog queries or UI. It cannot be installed as a compatible replacement from this experiment.

Pressure-flagged timed observations: none. Any flagged memory comparison is inconclusive as an unconstrained peak; raw points and events remain in the analysis JSON. No minimum-RAM requirement is inferred.

## Protocol history and evidence

The initial 4 GiB pilot was valid, but its original 85-minute-step/120-minute-job screen forecast was rejected and that screen was never dispatched. A separately reviewed, prospective roomy-v2 source and fresh 8 GiB smoke admitted this run within a 210-minute measurement-step/240-minute-job ceiling and the unchanged owner-approved 60 aggregate runner-hours. The 4 GiB pilot, 8 GiB smoke and million-entry screen are not pooled.

Evidence: [all raw-point analysis](cleanup-threshold-screen-analysis-2026-11-02.json), [selected raw artifact capsule](cleanup-threshold-screen-2026-11-02.tar.gz), [source freeze](cleanup-threshold-roomy-source-freeze-2026-11-02.json), [pre-dispatch admission](cleanup-threshold-roomy-screen-admission-2026-11-02.json), [initial pilot](cleanup-threshold-initial-smoke-2026-11-02.md), [nine integration work units](cleanup-threshold-integration-audit-2026-11-02.md), [small-folder update alternatives](cleanup-threshold-refresh-options-2026-11-02.md). Large image/binary/snapshot payloads are intentionally omitted locally; their remote ZIP inventory is preserved. Raw selected ZIP members are retrieved by HTTP ranges/CRC checks and indexed by SHA-256; original GitHub ZIP digest is recorded, not independently claimed verified locally.
