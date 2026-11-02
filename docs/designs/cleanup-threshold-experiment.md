# Cleanup threshold: preregistered prototype experiment

Date: 2026-11-02. Status: isolated research implementation, not a product backend.
Owner authorization: practically test the configurable 0.1% storage threshold,
measure its benefit and the integration problems; continue GitHub-first work.
No installed database, configuration, timer, master branch or release is changed.

## Question and information contract

Can a cleanup-oriented snapshot retain only a root and objects whose attributed
allocated subtree bytes reach 0.1% of filesystem capacity, folding all smaller
files AND directories into disjoint parent residual summaries? Preserve observed
allocated/apparent totals, inclusive kind counts, errors and freshness extrema.
Discarded per-path identity and historical per-file search/diff are deliberately
outside this prototype's contract. This experiment cannot qualify a drop-in
replacement for the existing complete catalog or waive existing product gates.

Compare current v1, current v2 and a separate Rust research example built from
the same source tree/compiler/target. The production scanner code is unchanged.
The example uses real filesystem metadata, streamed traversal and coarse-only
SQLite output. It does not build a complete per-object SQLite staging database.
Both of its metadata passes and exceptional hardlink ownership map count in
time/RAM. Current/previous coarse images are retained; no entire old fine tree
is retained implicitly. Unary-chain compression is not implemented initially.

## Faithful large-allocation fixture without a full physical disk

Use an owned **500 GiB logical sparse backing image**, ext4, a fixed 2-million
inode budget and 64 MiB journal on an ephemeral GitHub VM. A 500 GiB volume has
a threshold near 512 MiB; the actual threshold always comes from measured
`statvfs` capacity using integer ceiling: `(capacity * 1000 + 999999) / 1000000`.
This is GiB/MiB, distinct from the owner's illustrative 500 decimal GB / 500 MB.

Preallocation inside ext4 must produce real inner `st_blocks` above/below the
threshold while unwritten extents leave most outer backing-file data sparse.
Capture actual inner capacity/allocation and outer backing logical/allocated
bytes before/after allocation controls. Stop before accepting measurements if
this capability fails. Do not fake `stat`, use apparent-only holes as occupied
bytes, or silently fall back to a smaller filesystem. Ordinary sparse files
are a negative control: large apparent length must not imply large allocation.

The fixture must protect BOTH filesystems: inner free space and outer host/
results free space must remain at least 2 GiB. A sparse image's apparent free
capacity alone is not protection against host ENOSPC. Bounded, run-owned loop/
container/process cleanup is mandatory. Local workloads remain prohibited.

## Fixed comparison cases

Two shapes: mostly files and many small directories. Smoke scale is 100,000
bulk objects; screen is 1,000,000. The directory shape has half directories and
half regular files. Additional fixed controls are recorded separately from the
bulk count: actual allocated files straddling the threshold, an aggregate of
individually subthreshold files, a deep large file, empty files, an apparent-only
sparse file and raw non-UTF-8 names. Bulk allocation distribution and exact final
counts are captured before measurement; this is synthetic, not a measured
distribution of the user's real filesystem. Special hardlink/depth cases are
covered by the example's GH-only tests, not silently omitted from claimed scope.

Each arm begins from its own two-scan seed on the same unchanged fixture. Measure
three scenarios separately: a full scan; refreshing retained `large`; refreshing
omitted `small`, whose nearest retained boundary is the root. The last scenario
intentionally exposes the cost of missing old per-path contributions. A hardlink
fallback may also require a root scan and must report the actual boundary.
These unchanged cases do not establish performance for arbitrary changed trees;
threshold crossing/pruning/promotion tests supply correctness evidence only.

Smoke has one ordered comparison per scenario/profile: 12 seed commands,
18 timed commands and six independent semantic phases. Screen has three cyclic
orders, placing each arm in each position; 12 seed commands, 54 timed commands
and 18 semantic phases. No automatic retries or post-data changes to policy.
Setup, seed copies, evidence compression and observers are outside writer time
and RAM, but inside whole-job accounting and prospective runtime projection.

## Measurements, invariants and interpretation

* Fresh private cgroup per writer: 2 CPUs, 4 GiB, no swap, one application worker;
  actual requested limits must be verified. Capture kernel memory peak/stat/
  events, GNU-time command RSS, wall time, I/O counters and exit/OOM status.
  A memory cap/reclaim event makes an unconstrained-peak inference unavailable.
* Same direct-loop I/O envelope for every arm: 32 MiB/s read/write and
  2,200/3,600 read/write IOPS, measured calibration retained. This is a controlled
  envelope, not proof of physical equivalence to the user's local disk.
* Peak output metric: initial allocated snapshot plus sampled inner-filesystem
  growth during the command. Initial fixture allocation is excluded. Filesystem
  metadata and open-unlinked output may contribute. Sampling is a lower bound;
  independent file maxima are not added to invent a simultaneous decomposition.
* Root/retained/rest allocation, apparent size, inclusive counts and errors are
  checked against a streamed independent full-v1 SQLite reference; v2 root
  totals are crosschecked. Missing/extra retained nodes, double counting, sparse
  misclassification and invalid threshold arithmetic fail. Old/new command
  timestamps differ, so cross-arm freshness extrema are not asserted identical.
* Exact binary bytes, source file hashes, compiler/target, runtime image and
  commands are archived. Source capture includes build-time web inputs and all
  directly used measurement/helper/oracle modules. Hidden files are included.
* Report each profile/scenario separately, with all screen points and paired
  medians. A coarse-format benefit includes a changed information contract and
  traversal algorithm; it is not an isolated SQLite-engine optimization.

## Admission and stops

Central owner-approved campaign remains capped at 216,000 aggregate runner
seconds. Root reserves 3,600 seconds for smoke and 7,200 for conditional screen
in a new ledger namespace; both maxima include setup/build/test/cleanup.
Smoke measurement step cap is 2,700 seconds; screen is 5,100, with 240 seconds
reserved for cleanup. A separately recorded same-source/same-binary successful
smoke plus raw semantic acceptance is required for screen. Before dispatch,
project all host time at 10x entry scale and add the two extra screen cycles,
including seed-copy and case cleanup costs, then add 30% margin and fixed build.
If any step/job/remaining-budget cap fails, do not dispatch the screen.

Source/protocol changes after failure require a new recorded revision and fresh
review/admission; do not rerun unchanged input or reinterpret failure as a pass.
Actual job seconds, including failed runs, are charged once by root under the
existing ordered bookkeeping locks. No old frozen canonical helper is reused
to account for a namespace it does not understand.

## Known prototype limitations and integration deliverable

The example rejects inaccessible entries and nested mounts instead of providing
the production unknown-region model; limits depth to 256, retained nodes and
hardlink owner groups to 100,000, and entries to 10 million/pass. It has no GUI,
complete-catalog find/top/diff adapters, live drill cache or unary compression.
Its output must reside outside the scan root. Current-file replacement is
atomic, but replacing current AND previous is not a two-name atomic operation;
interruption can leave duplicate generations. Do not infer production recovery
equivalence from successful timing cases. Those gaps are integration work.

The result must report all four requested metrics and supported semantic checks,
plus the concrete API/storage/publish/refresh/UI/skill work in
[the integration impact audit](cleanup-threshold-impact.md). Quantify useful
wins and regressions without predicting unmeasured physical-disk, workload,
compression or minimum-RAM behavior. The final product and its rollout require
their own implementation and acceptance work after this research.
