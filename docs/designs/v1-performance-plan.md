# V1-first performance plan

Date: 2026-10-31. Status: W1/W3 campaign recorded; W3 not promoted; W4 source reviewed; D18 runner allocation pending.
Target and report destination: `docs/designs/v1-performance-plan.md`.
Source baseline: master `afce6fd`; investigation branch HEAD `c28bc7f`.
Owner request: optimize peak disk, RAM, full scan and folder refresh on v1;
use GitHub runners for expensive work. Snapshot contents are disposable: the
owner explicitly prioritizes having enough disk space over retaining history.

## Current review state

The initial review skipped required interactive decisions; its completion claim
was withdrawn. The owner has now answered D1–D16. Decision alternatives/grids later
in this document are historical briefs: their old pending labels describe the
state when asked, not the current approval state. Current accepted rules:

| Decision | Owner answer / binding result |
| --- | --- |
| D1 structure | Existing modules, six delivery waves |
| D2 peak disk | No confirmed increase beyond calibrated uncertainty |
| D3 memory | No confirmed peak-RSS increase or higher minimum tested successful budget |
| D4 full scan | At most 15% slowdown, conditionally |
| D5 exchange | Other-metric percentage reduction at least twice full-scan slowdown percentage |
| D6 folder refresh | No confirmed slowdown per scenario |
| D7 refresh experiment | Include one bounded staged-refresh experiment |
| D8 speed-only benefit | At least 15% confirmed improvement |
| D9 RSS benefit | At least 25% confirmed reduction |
| D10 disk benefit | At least 25% confirmed peak-allocation reduction |
| D11 capacity benefit | Alternative: at least 25% lower tested successful budget, with repeated success and baseline OOM proof |
| D12 local evidence | One immediate measured local v1 scan authorized; old DB deletion only if needed; no fio/cache flush |
| D13 first GH campaign | Six aggregate runner-hours; time cap, not money—standard public-repo runner compute is free |
| D14 RSS exchange floor | RAM savings justify full-scan slowdown only with both 2:1 percentage gain and at least 256 MiB absolute savings |
| D15 host refresh | One measured local refresh authorized and completed |
| D16 test host | Owner identifies this machine as a test host; choose/switch backend as useful for testing without another cutover approval |

Initial-campaign time cap is not a cap covering every future candidate. No GH
campaign has started. Local scan and refresh calibration completed; the owner-requested external review
with `claude-opus-5-5` completed with findings (see v1-performance-opus-review.md). No product optimization has been implemented. D16 permits backend switching on
this test host; it does not certify a production release or authorize unlimited
local load tests. New substantive findings will be discussed
before changing accepted contracts. Source/binary equivalence and proposed gains
still require evidence.

## Owner direction and proposed constraints

1. Use v1 as the optimization baseline. Keep v2 available as experimental code;
   do not revert the entire v2 commit history or remove shared correctness fixes.
   V1 already is the source-code default; the installed configuration selects v2.
2. Start fresh v1 history. No v2-to-v1 conversion, snapshot archive or mandatory
   old-data retention. Preserve the binary/configuration rollback recipe; rollback
   may require a new scan rather than restoring old results.
3. Heavy builds, fault/mount tests and performance fixtures run on disposable
   GitHub runners. D12/D15 authorized one local scan and one refresh (both completed),
   with old-database deletion if necessary for space. D16 permits choosing the
   test-host backend as needed. Other heavy experiments remain on GH runners.
   An eventual deployment scan is a separately scheduled operational action.
4. Optimize all four outcomes together: no speed claim may hide more disk, RAM,
   maintenance time, observer time or lost semantics. Keep small and large folder
   refresh distinct. Do not infer a minimum RAM requirement from process RSS.
5. Keep v1's immutable published snapshots, current/previous contract during
   normal operation, raw-byte names, hardlink policy and crash-safe publication.
   Disposable migration history does not authorize early deletion of previous
   snapshots during every ordinary scan.
6. No new storage engine, MVCC layer, dependency, background service or filesystem
   watcher in this program. Use existing staging, query, publication and test code.

## Evidence and premises

Historical measurements, not release thresholds; re-measure before promotion.
Source: [investigation and raw archives](../representative-performance-investigation.md).

| Premise | Evidence | Verdict |
| --- | --- | --- |
| V1 is already the default in source | `core/src/config.rs:15-19,43`; explicit override in `cli/src/backend.rs:20-33` | verified static |
| Installed v2 selection exposes a separate store | backend dispatch above; read-only installed config/service inventory | verified; repeat inventory before execution |
| Native 7,202,464-entry v1 scan 1 / 5: 340.624 / 348.872 s; v2: 559.162 / 825.249 s | run 41134701592, measured harness `5b62c14`, product `afce6fd`, one repeat | measured; not variance estimate |
| Native sampled CLI disk peak v1 / v2: 2.954 / 11.339 GiB | same run, dedicated filesystem minus initial arm baseline | measured lower bound |
| Native 18-entry refresh median v1 / v2: 27.931 / 0.0176 s | three refreshes of an unchanged small subtree | measured; not changed/large-folder result |
| V1 scan / refresh peak RSS: 23.23 / 1107.15 MiB; both backends filled a 4 GiB cgroup budget | same run | measured; not minimum RAM |
| V1 final writing is a promising first target | scan preparation ~125–131 s, insertion ~212–214 s; leaf SELECT/UPDATE in `staged/write.rs:146,156` | verified mechanism; gain unverified |
| Local snapshot data can be discarded for headroom | owner answer 2026-10-31: snapshots are not valuable; disk capacity for tests matters | binding user decision |
| Existing native and limited profiles reproduce local disk exactly | different devices, shape, history and installed binary provenance | false; sensitivity profiles only |
| Leaf/batching/cache changes will meet target improvements | no candidate measurements yet | unverified U1 |
| Lowest useful full-scale RAM budget | no systematic scan-and-refresh budget sweep | unverified U2 |
| Mixed synthetic shape matches local path/fanout/hardlink distribution | only biased small local sample exists | unverified U3 |
| Installed binary equals the runner build target/revision | installed version alone is insufficient | unverified U4 |
| Complete limited-I/O v1 baseline | previous limited sequence timed out before v1 | unverified U5 |

Native evidence uses two CPUs, one worker, disk staging and 4 GiB/no swap.
The limited sequence timed out at 280 minutes; five v2 scans are partial evidence,
not a complete paired baseline. Query/observer work warmed caches between commands.
Do not use those results as the new protocol's acceptance baseline.

The October 31 local scheduled scan used 18.021 GiB at its observed peak, including
2.080 GiB of legacy v1 files. It finished with v2 DB allocation 8.848 GiB. These
are historical observations, not current free-space guarantees or migration costs.

## Scope challenge and reuse

The smallest useful path is to improve the existing v1 disk-backed writer, then
address refresh memory without abandoning immutable output. A wholesale revert
would remove useful shared work and does not solve the installed stale snapshots.
A new incremental storage format would recreate the architecture question that
caused the v2 tradeoff. It is outside this plan.

What already exists:

- Disk-backed traversal, 8 MiB connection cache targets and free-space guards:
  `core/src/scan/staged.rs`, `core/src/scan/staged/walk.rs` and `write.rs`.
- Non-directory traversal shortcut already used by `staged/visit.rs:95`.
- Snapshot schema, bounded denied-path spool, hardlink attribution, generation
  selection, fsync/rename publication and failure handling. Reuse these contracts.
- Differential staged tests, publication/refresh tests, real fault/mount harnesses,
  CLI differential guard and installer comparator. Extend focused cases.
- Dedicated runner filesystem, calibrated I/O limits, sampled filesystem peaks,
  bounded cleanup and strict summary validation from the investigation.
- `TODOS.md` already requests a scan-path performance guard. Extend that work;
  do not create a competing benchmark framework or another unrelated TODO list.

Architecture alternatives considered: [Layer 1] preserve v1 publication and remove
redundant work (selected); [Layer 1] staged refresh (bounded-memory experiment);
streamed immutable splice (conditional later research); in-place mutable v1 or
new history store (rejected here). No new dependency or concurrency architecture.

Complexity: six delivery waves, one reviewable PR per wave. Any additional
conditional experiment requiring another delivery wave needs a new scope decision. Roughly 20 existing files
may be touched across the whole roadmap, not in one change; zero new services.
Each additional PR incurs its own hook, correctness CI and relevant benchmark
promotion gate. Separate waves prevent instrumentation, migrations and risky
refresh changes from hiding inside a micro-optimization.

## Architecture and execution order

```text
                 W1 paired measurement contracts
                            +
                 W2 reset/cutover rehearsal
                            |
                qualified baseline v1 candidate
                    /                    \
          W6 early cutover            W3 leaf SQL
          when authorized                  |
                                     W4 batches/cache
                                           |
                                W5 refresh experiment
                                (optional; may fail)
                                           |
                          W6 qualify selected improvements
                          and roll out when authorized
```


| Wave | Modules | Dependencies |
| --- | --- | --- |
| W1 | scripts/benchmark, workflows, harness tests | none |
| W2 | deployment docs and isolated CLI/install tests | drafting independent; execution requires W1 disk admission contract |
| W3 | core/scan/staged and staged tests | W1; W2 can progress independently |
| W4 | core/scan/staged, writer and tests | W3; compare each mechanism separately |
| W5 | core/refresh, staged loader/writer, CLI dispatch and tests | W1 and the settled W4 outcome (promoted or rejected) |
| W6 | workflows, release/install docs, operational runbook | W1/W2 and a qualified selected candidate; baseline v1 is allowed; W5 only if it passes |

Two initial worktree lanes: W1 and W2. Merge their contracts before W3 qualification.
W3/W4/W5 share staged code and must be sequenced. Do not concurrently edit the
writer from multiple optimization branches. Backend cutover is not blocked by a
failed optional refresh experiment: a qualified unchanged v1 remains a valid choice.
W6 can first qualify the unchanged baseline after W1/W2, then reuse its checklist
for later promoted improvements. This reuse does not create an extra implementation
PR or require completing all experiments before returning the host to v1.

## W1: establish a usable v1 comparison and resource budget

Implementation started on the investigation branch. See
[runner protocol](../v1-benchmark.md), [test-host runbook](../v1-test-host.md) and
[campaign ledger](../benchmark-results/v1-w1-campaign-2026-10-31.json).
The first remote campaign completed its planned baseline screening, one large
control and W3 exploratory measurements. See the
[campaign report](../benchmark-results/v1-w1-w3-campaign-2026-10-31.md).
Current protocol-v3 smoke passed; full-scale controls, memory-budget sweeps and
sampling sensitivity remain incomplete. No engine optimization is promoted.

Use the existing harness rather than rewriting it. Add explicit baseline-v1 and
candidate-v1 arms; current baseline dispatch is v2-oriented and cannot be assumed
correct for v1. Freeze source SHA, binary hashes, compiler/target/flags, fixture
seed, CPU quota, workers, filesystem and I/O calibration in every manifest.
Build base and candidate with identical toolchains. Installed binary equivalence
remains unknown until an implementation/deployment inventory establishes it.

Protocol version: create a new version, separate from the historical v4 sequence.
One measured operation per fresh arm; seed preparation is separately accounted.
For cold cases flush caches on the disposable runner immediately before each arm.
For warm cases specify and repeat identical warm-up operations; never call
observer-conditioned cache state ordinary warm usage. Alternate AB/BA order on
one VM per pair. Different profile VMs are not disk-only causal comparisons.

After each operation, close the writer, save its metrics and terminal receipts,
then validate in a separate resource-accounted observer container with read-only
snapshot storage. Retain checksums, logical validation, expected counts and OOM
checks. Account observer duration/I/O separately. A failed observer, missing arm,
missing result, seed mismatch or timeout invalidates promotion. Run terminal
observers after every accepted arm, never silently relax them to meet time limits.

Disk admission before fixture creation must include:

```text
remaining capacity >= fixture allocation + filesystem metadata/inodes
                    + both arm seeds/copies that coexist
                    + old/current/previous snapshot allocation
                    + candidate output + stage + temp + observer scratch
                    + measured uncertainty margin + enforced reserve
```

These are conservative planning bounds, not a measured sum of independent peaks.
Use exclusive filesystems for app peak measurement; record setup high-water marks
separately. Preallocation is charged to host backing capacity too. Check inode
capacity as well as bytes. On insufficient space: fail before generating millions
of entries, report required/available bytes and do not delete unrelated files.
Delete each run's disposable fixture/seed/output after artifacts are saved;
bounded cleanup removes only owned containers/mounts/files. Cleanup failure is a
failed job, not a passed measurement. Preserve small diagnostic artifacts, not
multi-gigabyte databases by default. Active mounts are never recursively deleted.

### Workload ladder and proposed time caps

| Stage | Fixture / repetitions | Job / measured-step cap |
| --- | --- | --- |
| PR smoke | 100k, native, one paired representative operation | 45 / 30 min |
| Promotion screening | 1m, selected scenarios, three pairs, AB/BA | 90 / 65 min per job |
| Native confirmation | 7.2m, selected scenario, one pair per job; three jobs | 150 / 120 min per job |
| Limited confirmation | 7.2m, selected scenario, start with one pair per job | 180 / 145 min per job |
| Cutover rehearsal | coexistence or disposable-history flow, fault cases | separate manual dispatch; preflight budget |

All caps are proposed envelopes, not measured estimates. Before launching a large
job, project fixture creation, seed preparation, both commands, observers, cleanup
and upload from screening results. Admit only when forecast plus 30% margin fits.
Scaling from 1m is an initial estimate, not a proof; failure stays inconclusive.
If not admissible, reduce scenarios/repetitions per job, not validation or fixture
size while still calling it full scale. Keep both compared arms on the same VM.
Do not automatically retry failed expensive jobs or multiply the full Cartesian
matrix. Before each candidate campaign, persist the exact scenario IDs, arms,
repetitions, job count and maximum aggregate runner-minutes in its manifest.
The initial full-scale screening allocation is one primary-scenario pair and
one secondary-operation guard pair. Expand to three confirmation pairs for each
required metric/profile only after screening success and renewed admission.
No arbitrary "selected scenarios" expansion or unlimited retry loop is allowed;
more than the predeclared jobs requires a revised experiment budget.

Retain shallow legacy shape as a control. Add a deterministic mixed-depth shape
with varied fanout/name lengths, allocated and sparse files, hardlinks and mount
boundaries. Bound total physically allocated fixture bytes before generation; sparse apparent
size does not stand in for allocated-byte load. Use small deterministic payloads
and record the allocation budget rather than copying a real user corpus.
Publish distributions; without reliable local aggregates call it a
sensitivity fixture. No local traversal is needed to generate it.

Refresh cases: unchanged and changed folders with ~100 / 10,000 / 100,000 entries.
One-million-entry refresh is conditional on smaller-case admission. Mutations:
add/delete/rename, size and mtime changes, nested removal, cross-boundary hardlinks.
Prepare a shared mutation manifest, independent byte-identical pre-change snapshot
seeds and one read-only changed tree; do not undo/recreate files between arms and
thereby change inode identity. Measure preparation/copying peak separately too.

Semantic oracle for refresh: unchanged baseline-v1 refresh on an identical seed,
with nondeterministic metadata normalized under explicit rules. A full rescan is
only a secondary check of applicable content/aggregate invariants: it is NOT an
exact oracle for retained freshness, previous-generation or hardlink-owner policy.
Normalize generated snapshot IDs and map genuinely new observation timestamps
consistently. Keep untouched entries' stored times, previous generation metadata,
ordering and freshness relationships exact; assert ancestor minimum relationships
and refresh history explicitly. Include old untouched entries and newly refreshed
descendants. Never mask all timestamps in order to make a differential pass.

### Four outcomes and promotion rules

For each operation record full CLI wall time, process RSS, cgroup peak/current,
`memory.stat` (anonymous/file/kernel attribution), pressure/OOM/swap, sampled
simultaneous filesystem allocation and named allocation, baseline/end sizes,
WAL/stage/temp contributions and supporting I/O counters. For total application
footprint subtract only fixture/static-filesystem overhead, NOT the seeded
current/previous snapshots. Report extra growth above command start separately.
Remove prior arm outputs before establishing the next arm baseline; seed copies
needed concurrently remain charged in setup/capacity accounting. GiB = 2^30 bytes;
MiB = 2^20 bytes; durations in seconds. Peaks are sampled lower bounds.

Screen memory budgets 512/1024/2048/4096 MiB on 1m entries, with no swap and fixed
CPU/workers. Confirm the lowest promising budget and the next larger budget at
7.2m for BOTH scan and refresh. Report a minimum *tested successful* budget and
latency tradeoff, not a theoretical minimum or RSS-only requirement. Cgroup peak
pinned to its configured ceiling cannot demonstrate memory improvement by itself.

Proposed prospective acceptance thresholds, fixed before candidate results:

- Semantic/CLI/publication parity, successful observers and complete expected
  coverage, no candidate OOM, and successful failure recovery are mandatory.
  Ordinary speed trials require both arms to succeed. Only a designated capacity
  test may record an expected baseline OOM as an outcome: keep its exit/cgroup
  receipts, and obtain the baseline semantic oracle at a sufficient budget.
  Missing evidence or timeout is never an expected resource-success outcome.
- Speed-only candidates: target-operation paired median at least 15% better
  (owner D8=B), beyond calibrated measurement uncertainty, with all
  three confirmation pairs improving. Memory candidates: complete at a tested
  budget at least 25% lower than the baseline minimum tested successful budget,
  with reproduced baseline OOM at the candidate budget and repeated candidate
  success (owner D11=A),
  or reduce peak process RSS by at least 25% (owner D9=A), at equal semantics
  and workload and beyond calibrated measurement uncertainty. These are
  goals/gates, not promised results.
- Disk-savings candidates: reduce total application-attributable peak disk
  allocation by at least 25% (owner D10=A), beyond calibrated measurement
  uncertainty, under identical fixture and retained-generation state. The stricter
  D5 exchange requirement still applies when full scan slows down.
- Memory candidates must also meet a target-operation latency regression ceiling
  of 15% for full scan, conditional on the owner-approved D5=B exchange rule; folder refresh must not show a confirmed slowdown beyond calibrated measurement
  uncertainty (owner D6=A). Compare against a
  successful baseline at the same tested budget; capacity-only
  success where baseline OOMs is a separate result and cannot establish latency
  parity. Confirm the performance gate at a common successful budget.
- Full-scan latency: at most 15% regression only for a justified gain in another
  metric (owner D4). Owner D5=B requires a reduction in at least one other
  requested metric of at least twice the full-scan slowdown percentage. For
  example +15% scan time requires at least -30% peak disk, peak process RSS, or
  folder-refresh time under the same paired workload. Benefit must exceed
  calibrated measurement uncertainty; all other acceptance constraints still apply.
  If peak-RSS savings justify the slowdown, also require at least 256 MiB absolute
  savings (owner D14=A). This floor does not apply to memory savings without a
  scan slowdown or to the other independently accepted benefit routes.
  Folder-refresh latency: no confirmed increase beyond calibrated measurement
  uncertainty (owner D6=A), evaluated separately for small/large folder and change scenarios. Simultaneous
  disk peak: no demonstrated increase beyond calibrated measurement uncertainty
  (owner D2=A), within absolute admitted capacity. Process peak RSS: no demonstrated increase beyond calibrated measurement
  uncertainty, and no increase in minimum tested successful memory budget (owner D3=A). Cache charges/pressure are reported separately,
  not hidden.
- If baseline variance prevents a decision, mark inconclusive; do not tune bars
  after seeing candidate results. Three pairs are a consistency screen, not a
  tight confidence interval. Do not claim p95 from three observations.
- Select non-dominated configurations rather than a weighted score hiding a
  regression. A failed gate is no promotion; a desired tradeoff needs a separately
  reviewed change to this plan, never an automatic gate exception.

Fast baseline/base controls expose harness bias. Retain the existing 15% scan-path
regression-guard TODO as a separate historical guard; promotion's tighter bars do
not rewrite that policy silently. Add the promoted protocol to relevant CI paths.

## W2: disposable-history cutover and disk headroom

The owner does not value old snapshots. Prefer a controlled reset over exporting,
copying or retaining gigabytes solely for rollback. Source already defaults to v1;
this is primarily installed configuration and fresh-data work, not a code revert.

1. Record installed binary hash/version/build target, config and environment
   overrides, unit/drop-ins, snapshot root, ownership/modes, mounts and capacity.
   Verify the selected binary can create/read schema-v1 snapshots. Retain binary
   and small configuration copies; no mandatory database backup.
2. Rehearse on a disposable runner: quiesce timer/manual writers and readers; verify
   no scan remains and no process retains open old snapshot files. Stop/restart
   serving processes too, because unlinking a file held by a reader may free no space.
3. Use an explicit allowlist of app-owned generated snapshot/state/progress/temp
   paths in the verified snapshot roots. Reject symlink/path escapes or unexpected
   layout. Remove disposable old v1/v2 results only when quiescent; preserve config,
   binary and unrelated data. Do not use uninstall as migration: it removes more.
4. Recheck actual allocated bytes and available capacity after deletion. Require
   enough capacity for the fresh scan, staging/temp, filesystem reserve and margin.
   A same-filesystem backup or an extra rehearsal database is not free storage.
5. Create fresh v1 using the intended binary and explicit config/environment in
   the chosen empty snapshot directory. Do not expose stale legacy v1 results.
   Schedule this one necessary operational scan separately from benchmark loads.
6. Verify roots, counts, sampled queries, expected incomplete-directory diagnostics,
   publication and doctor/status. Fresh history has no previous snapshot; show
   `NO_PREV` honestly until the next normal full scan. Do not force a second scan
   solely to manufacture history or relabel generations.
7. Enable normal v1 service operation and verify its actual effective backend,
   low-memory selection and SQLITE_TMPDIR. Before snapshot disposal, an abort may restore the previous timer activation
   state. After disposal, keep writers/timers disabled until a verified scan or
   explicitly chosen regeneration/recovery is ready; never restart the old v2
   job automatically into insufficient capacity. Keep settings recoverable.
8. On failure: keep service safely stopped or show no snapshot; restore binary/config
   if necessary and regenerate a snapshot. There is deliberately no instant
   data rollback after disposal. User-visible absence is preferable to stale data
   presented as current. Confirm subsequent retry succeeds and leaves no temp leak.

This plan is not an instruction to delete anything during review. Before actual
execution produce a concrete path/capacity manifest for the selected host. The
snapshot-disposal policy is already settled by the owner; do not re-ask whether
old history has value. Stop if paths differ from that manifest or capacity fails.

## W3: remove redundant leaf work first

Existing `core/src/scan/staged/write.rs:146` calls `next_child(parent)` for every
frame, including files; line 156 executes a final aggregate UPDATE for every row.
`enter()` already inserts leaf aggregates and `subtree_end=id` after hardlink
attribution. `staged/visit.rs:95` demonstrates the existing non-directory shortcut.

Test two independent counterfactuals, then the combination: skip child lookup for
known non-directories; skip a leaf's redundant final UPDATE. Preserve parent merge,
per-merge saturation, denied/mount side tables, raw name ordering and full-path
hardlink ownership. Empty directories and denied directories are not files merely
because `items==0`. Use node kind and prove inserted values equal final values.
Instrument statement counts in diagnostic experiments without adding a per-entry
production tracing cost. Verify reduced SQL work and complete command improvement.

Do not factor a generic traversal framework solely because visitor/writer code
looks similar. Reuse existing semantics and small predicates only when contracts
match; writer publication/aggregation effects differ from a read-only visitor.

## W4: bounded traversal and cache experiments

After W3, compare fixed child batches to the current `(name,id)` keyset LIMIT 1.
Use the existing `raw_children(parent,name,id)` index and a GLOBAL retained-byte
and row budget across all active depth frames. Include long names and deep trees;
per-frame batching without a total budget can silently restore O(N) RAM use.
Preserve deterministic DFS IDs, contiguous subtree intervals and root order.

Only screen cache reallocations within the existing aggregate cache budget; do
not include the previously proposed 16/32 MiB-per-connection increases by default.
Those arms conflict with the accepted no-RSS-regression constraint absent new
evidence that they can fit. Keep source/destination budgets explicit, holding
journal mode, mmap, spill, temp placement and durability constant. Two connections
coexist; their budgets add. Do not expose a permanent public tuning flag before
showing a useful stable configuration. SQLite `cache_size` is a suggested page
cache bound, not a process/RAM guarantee: [SQLite PRAGMA documentation](https://www.sqlite.org/pragma.html#pragma_cache_size).

Only promote mechanisms independently supported by paired evidence. If batching
fails its memory or speed gates, retain the last qualified product baseline and
the current keyset loop. W3's combined leaf experiment missed D8 and is not a
qualified dependency: a batching-only experiment must start from unchanged v1.
If larger
caches harm low-memory hosts, keep defaults and publish the sensitivity result.

Prepared experiment: [bounded child batches](v1-child-batch-experiment.md), branch
`perf/v1-child-batches`. Normal pre-push checks compile the tests and pass; the
[Opus source review](v1-child-batch-implementation-review.md) closes its P2 findings
with a final no-findings verdict. Rust test execution and W4 timings remain
unmeasured. D18 is pending: two 1m screening dispatches (current-observer control,
then candidate only after a valid control) would reserve at most11,400 aggregate
runner-seconds from the remaining13,482, within the same21,600-second cap. No W4
job starts before the owner's allocation choice. No full-scale expansion or
threshold exception is included.

## W5: improve v1 refresh without recreating v2

Current refresh loads the full tree (`core/src/refresh.rs:242`), grafts/refinalizes
it and writes a full replacement (`:282`). The ~1.1 GiB RSS on the large fixture
is evidence of this limitation. Full-scan `low_memory` does not cover refresh.

First identify the policy boundary: published directory values are aggregates,
whereas staging requires own values. Mirror the reconstruction and saturation
semantics in `core/src/refresh/load.rs:152-187`, retain raw hardlink sizes and prior
owners, and supply refresh-specific attribution to output. The full-scan writer
currently chooses owners by full path (`staged/write.rs:249`); invoking it unchanged
would violate refresh semantics. This boundary must be explicit and tested before
sharing the emission path.

First bounded experiment: stream existing snapshot rows into disk-backed staging,
replace the selected subtree there, then reuse immutable staged output. Retain
raw names, metadata, root/mount/denied records, cross-subtree hardlink ownership,
ancestor extrema and previous-generation semantics. Route the low-memory policy
explicitly through refresh without silently changing unrelated CLI behavior.

This can bound RAM but still reads/writes O(total entries), and may cost additional
disk/time. Its acceptance gate includes all four outcomes. Do not promise v2-like
18 ms latency for v1. If it fails the no-regression gates, do not ship it by default.
Record the result and keep the existing v1 refresh while considering a separate
streamed-splice proposal with measured benefit.

Conditional streamed splice would remap dense IDs and all side-table references,
reuse unchanged aggregates and recompute affected ancestors/hardlink groups.
Deletion of a maximum/minimum contributor requires recomputation, not subtraction.
This is a separate architecture review, not a shortcut hidden in W5.

## W6: qualify, publish and verify the selected configuration

Publish reproducible baseline/candidate artifacts and the four-metric comparison,
including failed/inconclusive candidates. Build the actual distribution target,
not just an unrelated GNU binary; verify install/upgrade behavior on runners.
Run relevant CLI/API compatibility, publication/fault/mount suites and quality
hooks; never bypass hooks or weaken a failing guard. Keep source/function size
limits from CLAUDE.md. Use normal release/ship workflow only after implementation.

Package the reviewed cutover manifest and capacity estimate for the actual host.
After authorized deployment, use the existing scheduled scan to observe results;
keep expensive diagnostics on runners. No claim of local improvement before a
comparable completed local observation. Record unavailable comparisons honestly.

## Conditional experiments and NOT in scope

- Selected deferred indexes: only if writer profiling still identifies index work;
  include external-sort files, RSS, reserve/failure behavior. Current early indexes
  deliberately avoid large sorts. Do not defer all indexes merely for speed.
- Staging compression/representation: only after dbstat occupancy shows worthwhile
  space. Native v1 peak is structurally current + previous + replacement + staging;
  eliminating ~480 MiB staging cannot justify a claimed multi-gigabyte reduction.
- No conversion of v2 history, old-snapshot archive or preservation window: owner
  explicitly chose disposable data. No removal of ordinary v1 current/previous
  safety, no in-place published database writes, no new storage format.
- No v2 rescue/GC project on the critical path. Preserve its code for optional later
  research; it does not block v1 progress. No GUI redesign, watcher, scheduler port,
  query feature expansion or new permanent tuning service.
- Existing unrelated TODOS remain unchanged. The speed-guard TODO is included in
  W1/W6; completed corruption/mount/error fixes must remain protected, not rebuilt.

## Engineering review: findings and dispositions

Evidence is static code inspection plus historical benchmark artifacts. No proposed
optimization or migration has been executed. Planning scope is authorized by the
owner's request; implementation, deployment and quantitative performance remain
future work. Confidence below applies to the diagnosed risk, not predicted gain.

### Architecture

- A1 [INFORMATIONAL, confidence 10/10] `core/src/config.rs:43`,
  `backend: Backend::V1`: a code-default rollback is already done. Disposition:
  W2 changes effective installation and resets disposable history; no blanket revert.
- A2 [CRITICAL, confidence 9/10] `cli/src/backend.rs:31-32`, separate V1/V2 paths:
  flag-only cutover can revive stale data; live readers can hold deleted bytes.
  Disposition: W2 quiescence, verified path manifest, fresh scan and actual capacity
  recheck. Owner explicitly permits losing snapshot history as a planning policy.
- A3 [INFORMATIONAL, confidence 9/10] `core/src/refresh.rs:242`,
  `let mut loaded = load_tree(&snap)?;`: immutable dense v1 does not imply cheap
  subtree writes. Disposition: W5 is a gated memory experiment with O(N) output;
  streamed splice stays outside the initial implementation contract.

### Code quality

- Q1 [INFORMATIONAL, confidence 9/10] `staged/write.rs:146`,
  `if let Some(raw) = self.next_child(parent)?`: leaf work is avoidable in principle,
  but mount/denied directories and hardlink attribution make naive skipping risky.
  Disposition: W3 independent transformations with semantic differential coverage.
- Q2 [INFORMATIONAL, confidence 9/10] `staged/write.rs:110`,
  `WHERE parent=?1 AND (name,id)>(?2,?3) ORDER BY name,id LIMIT 1`: batching may reduce
  repeated statement work but per-depth buffers can explode retained memory.
  Disposition: W4 global byte/row budget; no generic traversal abstraction or new
  library. Tests preserve stable ordering and IDs.

### Tests and user flows

- T1 [CRITICAL, confidence 9/10] `core/src/refresh.rs:423-424`, retained previous
  metadata; `core/tests/staged.rs:145`: refresh and full scan are different contracts.
  Disposition: baseline-v1 refresh is the primary differential oracle. Full rescan
  validates only explicitly shared invariants, never all timestamps/history blindly.
- T2 [CRITICAL, confidence 9/10] `core/tests/publish.rs`, `cli/tests/faults.rs`,
  `scripts/tests/test_bench_summary.py`: correctness suites exist, but proposed reset,
  globally bounded batches and staged refresh need targeted extension. Disposition:
  the following value cards and failure matrix are prerequisites to shipping each
  changed behavior. Existing green tests alone are not proof of proposed code.

```text
CODE / BRANCHES                                USER OUTCOME / REQUIRED PROOF
W2 effective config + path manifest
  | override/path mismatch -> stop              wrong root is never deleted [GAP E2E]
  | busy writer/reader -> wait/stop               no partial store removal [GAP E2E]
  | low space/inodes -> fail before work          capacity reason shown [GAP E2E]
  ` empty store -> scan -> verify -> activate     fresh data; first NO_PREV [GAP E2E]
       | ENOSPC/kill -> no publication            retry usable, no stale claims [extend faults]
       ` failed activation -> restore config     no history rollback promised [GAP E2E]
W3 staged walker -> enter -> leaf / directory
  | leaf/hardlink -> correct insert -> merge     exact rows/owner/aggregates [extend staged]
  ` directory -> ordered children -> aggregate   empty/denied/mount preserved [extend staged]
W4 keyset/batch cursor -> budget / next row
  | budget full -> drain/reload, no drop          deep/wide/raw-name parity [GAP core]
  ` end/error -> close/cleanup                    no memory/temp leak [extend fault]
W5 read snapshot -> disk stage -> replace subtree
  | missing vs inaccessible target               deletion vs error retained [extend edges]
  | owner outside / extremum removed              ownership/min/max correct [extend queries]
  ` emit IDs + side tables -> publish             diff/previous/readers valid [GAP E2E]
W1 fixture -> seeds -> AB/BA -> CLI -> observer
  | seed mismatch/OOM/timeout/missing output      INCOMPLETE, no promotion [extend harness]
  ` validated + terminal receipts -> ratios       comparable four-metric report [GAP harness]
```

Legend: GAP = proposed behavior not yet tested; extend = reuse a named existing
behavioral suite. E2E means actual CLI/storage boundary on disposable runners.
No LLM/eval or visual interaction change is proposed. No fabricated coverage
percentage: these are prospective paths, not executed candidate coverage.

| ID | Test destination / existing coverage | Required assertion and value card |
| --- | --- | --- |
| C1 | `cli/tests/v2_backend.rs`; proposed runner cutover script tests | Fresh empty v1 starts without old history; v2 isolation; overrides resolve intended root. Value: protects=cutover destination; fails_when=stale/wrong store exposed; why_new=no reset flow covered; seam=none; no_claim=real host configuration. |
| C2 | proposed `scripts/tests/test_bench_cutover.py` plus runner integration | Busy writer/reader, path escape and low bytes/inodes prevent cleanup/scan; retry restores usable configuration. Value: protects=owned-data-only reset; fails_when=wrong path deletion or optimistic headroom; why_new=no cutover rehearsal; seam=none; no_claim=unobserved OS variants. |
| C3 | extend `core/tests/staged.rs:76,105,124,145,182,210,250` | Leaf/directory/raw-name/root/hardlink/overflow parity; reserve failure preserves published state. Value: protects=stored semantic parity; fails_when=leaf shortcut drops side effects; why_new=new branches; seam=none; no_claim=performance gain. |
| C4 | staged tests plus GH memory scenario | Wide and deep long-name trees consume only global batch budget and emit identical preorder IDs. Value: protects=ordering and bounded retention; fails_when=per-depth buffers multiply; why_new=batching absent; seam=none; no_claim=all allocator/cache charges bounded. |
| C5 | extend `core/tests/queries.rs:466,533,575`, `core/tests/edges.rs:47,79` | Refresh removal/rename/permissions, stale freshness, ancestor extrema and external hardlink owner match old v1. Value: protects=refresh semantics; fails_when=full-rescan policy replaces refresh policy; why_new=staged-refresh path; seam=none; no_claim=hardware latency. |
| C6 | `core/tests/publish.rs:157` and `cli/tests/faults.rs:186`, fault/mount scripts | Before/after publish failures, current/previous integrity, long-lived reader, late mounts and ENOSPC at staging/output/index. Value: protects=atomicity and recovery; fails_when=partial output published or prev lost; why_new=new temp path; seam=existing fault points; no_claim=all power-loss hardware behavior. |
| C7 | `scripts/tests/test_bench_*.py` | Baseline/base controls, expected arm/scenario coverage, seed/observer failures, signal cleanup, rejected incomplete runs. Value: protects=measurement validity; fails_when=partial/wrong-backend result promoted; why_new=v1 paired protocol; seam=none; no_claim=runner hardware equivalence. |
| C8 | GH full-scale memory/latency fixtures | Both full scan and changed-folder refresh complete under declared budgets with semantic parity and all receipts. Value: protects=resource claim; fails_when=OOM/space/time exceeds envelope; why_new=budget sweep absent; seam=none; no_claim=universal minimum RAM. |

Reuse `cargo test --workspace`, targeted crate test files, `scripts/check.sh`,
Python unittest discovery, actual fault/mount scripts and CLI differential checks.
Full suite and expensive boundary tests run on GitHub. Unit counters/SQL plans
can diagnose mechanisms but cannot replace end-to-end timing or semantic tests.
Tests to retire: none. No production API added solely to make a test easier.

### Performance

- P1 [CRITICAL, confidence 10/10] investigation limited sequence: five scans plus
  observers exhausted 280 minutes before the comparison arm. Disposition: W1
  single-operation arms, separate observer accounting, admission budget and shards.
- P2 [INFORMATIONAL, confidence 10/10] native large case has one paired sequence,
  and cgroup peak saturates 4 GiB. Disposition: re-baseline, alternating pairs,
  memory budget sweep including refresh; no p95/minimum-RAM fiction.
- P3 [INFORMATIONAL, confidence 9/10] `snapshot/publish.rs:107,125` preserves old
  immutable data until publication. Disposition: measure simultaneous footprint;
  do not delete previous early or advertise speculative peak-disk savings.

Scope challenge found two planning corrections (already-v1 default; disposable
history). Four review sections found 13 issues, all mapped to work or explicit
bounds. They are not claims that candidate changes already pass tests.

### Additional independent technical findings

- A4 [CRITICAL, confidence 9/10] `core/src/refresh/load.rs:152-187` and
  `staged/write.rs:249`: published aggregates and full-scan owner selection cannot
  be passed through refresh unchanged. Disposition: explicit own-value/raw-link
  reconstruction and refresh attribution seam added to W5 and C5.
- Q3 [CRITICAL, confidence 9/10] freshness is observable in query/refresh metadata;
  broad timestamp masking can hide a wrong implementation. Disposition: W1 now
  defines allowed normalization and exact/relational freshness assertions.
- P4 [INFORMATIONAL, confidence 9/10] resource capacity and ordinary success are
  different trials; a memory candidate's own latency was insufficiently bounded.
  Disposition: expected baseline OOM only in named capacity cases; candidate must
  succeed, semantic oracle at adequate budget, and common-budget latency gate.

## Failure modes and recovery contract

| Failure | User-visible response / handling | Planned proof |
| --- | --- | --- |
| Wrong path, symlink escape, env override | Stop with resolved manifest; leave data intact | C1/C2 |
| Readers/writers still active | Do not remove snapshots; identify blocker | C2 |
| Insufficient bytes or inodes | Refuse before fixture/reset/scan; report capacity | C2/C7/C8 |
| ENOSPC during stage/output/index | Fail, clean owned temps, keep published state during ordinary scans | C3/C6 |
| Reset completed but first scan fails | No fresh snapshot; explicit failed state, restore configuration or retry | C1/C2/C6 |
| Kill before/after rename or state update | Atomic published file; existing recovery behavior; no fake success | C6 |
| Hardlink owner/ancestor extremum changes | Preserve baseline refresh semantics; reject mismatches | C3/C5 |
| Deep/wide workload grows buffers | Budgeted draining, no OOM at supported envelope | C4/C8 |
| Observer or arm incomplete | Invalid experiment; retain diagnostics; no release claim | C7 |
| Cleanup leaves mount/container | Job fails; preserve mount until safe teardown; never rm through it | C7 |
| Candidate faster but violates another budget | No promotion; retain known-good v1 | C8 |

Critical gaps with neither planned error handling nor meaningful proof: 0.
All listed proofs are future implementation requirements, not currently executed
acceptance coverage. Security boundary remains local snapshot-root ownership;
no new network/auth/API boundary is introduced. Existing CLI/API output safety
and serving behavior remain compatibility requirements.

## Decision ledger

| ID | Decision / alternatives | Basis and disposition |
| --- | --- | --- |
| R1 | V1-first, retain experimental v2 code vs wholesale revert | Owner accepted v1-first direction; existing default and shared fixes verified. Planning scope fixed. |
| R2 | Disposable history vs export/archive both v2 generations | Owner explicitly says snapshots are not valuable and capacity matters. Fresh v1, no required data archive; binary/config recovery remains. |
| R3 | Writer leaf fast paths before new storage architecture | Routine plan ordering under owner request; lower-risk mechanism is verified, gain remains a hypothesis. |
| R4 | Same-VM v1 pairs and staged promotion vs another monolithic long run | Required validity for requested optimization plan; reuses timeout learning. |
| R5 | Preserved v1 semantics/atomicity and targeted regression proof | Necessary proof of optimizing existing behavior; no intentional semantic weakening approved. |
| R6 | Performance gates | Superseded by explicit D2–D14 decisions; use Current review state and promotion rules, not original draft thresholds. |
| R7 | Staged refresh experiment vs promising subtree-proportional writes | Bounded optional candidate; cannot replace default unless it passes all gates. No new storage architecture approved. |

Approval readiness: D1–D16 resolved. The plan's selected gates and bounded
experiments are recorded above. Local scan/refresh receipts and external Opus
review are available; findings have dispositions below. Backend switching on the
test host is authorized by D16, with concrete capacity/path checks at execution.
No gain is claimed until candidate qualification; a planning review is not a
production-release certification. The initial GH campaign remains capped at six
aggregate runner-hours, despite free standard runner compute.

## Interactive decisions (resumed)

D1 — Structure: accepted A by the owner (reply: "A"). Retain existing module
boundaries and six delivery waves, roughly 20 existing files, no new services.
This accepts structure only, not numerical gates, optional experiment scope,
aggregate runner-time consumption or implementation/deployment. No feature cut was approved.

D2 — Peak disk tradeoff: ACCEPTED A by the owner (reply: "A"). No demonstrated peak-disk increase beyond calibrated measurement uncertainty is allowed. The alternatives below are retained as decision history. The original draft proposed allowing up to
10% additional peak disk for a candidate that improves another metric. No owner
approval exists for that bound. Recommended A: require no demonstrated peak-disk
increase beyond calibrated measurement uncertainty. B: allow at most 10% increase
relative to baseline v1 for the same operation/fixture. Both retain absolute
capacity admission; absolute headroom, RAM/latency limits and minimum useful gain
remain pending independently. A may reject useful speed/RAM improvements; B may
spend scarce disk space. Both require paired measurements; uncertain results are
inconclusive, not permission to weaken the gate. No candidate is implemented by
this answer.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Relative peak-disk ceiling | Draft +10%, unapproved | No demonstrated increase beyond calibrated uncertainty | At most +10% vs baseline v1 |
| Absolute capacity admission | Required by owner disk priority | Retained; numerical reserve pending | Retained; numerical reserve pending |
| Structure | D1=A | Unchanged | Unchanged |
| RAM/latency bounds, useful gain, refresh scope, runner budget | Pending | Pending | Pending |

D3 — RAM tradeoff: ACCEPTED A by the owner (reply: "A"). No demonstrated increase in peak process RSS or minimum tested successful memory budget is allowed. The alternatives below are decision history. Recommended A: no demonstrated increase in
process peak RSS beyond calibrated measurement uncertainty, and no increase in
the minimum tested successful memory budget for the same workload. B: allow up
to 10% higher process peak RSS for a speed improvement, provided the same tested
memory budget still succeeds. This choice concerns memory consumption only;
minimum useful speed gain and latency limits remain pending. Both report cgroup
usage/cache pressure separately and require successful candidate completion.
A preserves memory headroom but may reject useful cache/batch acceleration.
B permits more tuning room but consumes memory headroom. Both require the same
paired memory measurements; neither authorizes implementation or aggregate runner-time consumption.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Peak process RSS | Draft +10%, unapproved | No demonstrated increase beyond calibrated uncertainty | At most +10% for speed improvement |
| Minimum tested successful memory budget | Must remain usable on baseline-supported budget | No increase | No increase |
| Peak disk | D2=A | Unchanged | Unchanged |
| Latency limits, useful gain, refresh scope, runner budget | Pending | Pending | Pending |

D4 — Full-scan latency tradeoff: OWNER COUNTERPROPOSAL ACCEPTED as a ceiling: up to 15% slowdown, only if improvements in other metrics justify it. Owner reply: "можно допустить замеление на 15%, но курс замедление/другие метрики должен быть хороший". Exchange acceptance rule resolved by owner D5=B (minimum 2:1 percentage improvement); this is not unconditional permission for +15%. The original A/B choices below are history. A (recommended): no demonstrated
increase in full-scan elapsed time beyond calibrated measurement uncertainty.
B: allow at most 5% greater full-scan elapsed time in exchange for a verified
improvement in another metric. The comparison uses the same fixture, successful
memory budget, CPU/I/O profile and cache condition. Both retain D2/D3 disk and
memory constraints and identical correctness requirements. A protects scan
latency but can reject valuable resource savings; B permits a bounded slowdown
but delays completion. Both require the same paired measurements. Folder-refresh
latency, minimum useful gain, experiment scope and runner budget remain pending.
Neither answer authorizes implementation or launches benchmarks.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Full-scan elapsed-time regression | Draft +5%, unapproved | No demonstrated increase beyond calibrated uncertainty | At most +5% for another verified metric improvement |
| Peak disk and memory | D2=A, D3=A | Unchanged | Unchanged |
| Folder-refresh latency and minimum useful gain | Pending | Pending | Pending |
| Experiment scope and runner budget | Pending | Pending | Pending |

D5 — Rule for justifying a slower full scan: ACCEPTED B by the owner (reply: "B"). The minimum 2:1 percentage rule applies, with the 15% full-scan slowdown ceiling. Alternatives below are decision history. Recommended A:
review each candidate's measured tradeoff with the owner before accepting any
confirmed full-scan slowdown. Report absolute and relative changes for all four
metrics, successful memory budgets and measurement uncertainty. The D4 maximum
of 15% is retained, not automatically granted. B: a predeclared relative rule:
require at least twice the slowdown percentage as a reduction in at least one
other requested metric (peak disk, peak RSS, or folder-refresh elapsed time),
with a demonstrated benefit beyond measurement uncertainty. For example +15%
scan time requires at least 30% reduction in one other metric. This is a proposed
heuristic, not a claim that percentages have equal user value. Existing disk/RAM
constraints remain binding; refresh regression policy remains independently
pending. A requires a future user decision but considers actual workload value;
B permits automatic eligibility under this rule but may misvalue small absolute
savings. Neither changes the other qualification requirements or authorizes work.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Maximum full-scan slowdown | Owner D4: 15%, conditionally | Unchanged | Unchanged |
| Justification for slower scan | Unresolved | Owner evaluates measured absolute/relative tradeoff per candidate | At least 2x slowdown percentage reduction in one other metric |
| Disk/RAM constraints | D2=A, D3=A | Unchanged | Unchanged |
| Refresh regression policy, scope and runner budget | Pending | Pending | Pending |

D6 — Folder-refresh latency ceiling: ACCEPTED A by the owner (reply: "A"). No confirmed increase beyond calibrated measurement uncertainty is allowed, separately for each folder/change scenario. No slower-refresh exchange rule is needed. Alternatives below are decision history. A (recommended): no
confirmed increase in folder-refresh elapsed time beyond calibrated measurement
uncertainty. B: allow up to 5% slowdown. C: allow up to 15% slowdown. B/C would
also need a separately resolved gain rule before accepting a slower refresh;
D5 applies only to full-scan slowdown and is not silently extended here. Compare
the same folder/change scenario and memory budget; distinguish small and large
folders so an aggregate does not hide a regression. A protects interactive wait
time but may reject resource-saving variants. B allows a small tradeoff; C offers
more room but longer waits. All require paired measurements and preserve disk/RAM
constraints and D4/D5 full-scan rules. This choice sets only the refresh ceiling;
experiment scope, runner budget and any refresh exchange rule remain pending.

| Commitment | Current | A | B | C |
| --- | --- | --- | --- | --- |
| Refresh latency regression ceiling | Draft +5%, unapproved | No confirmed increase beyond calibrated uncertainty | At most +5% | At most +15% |
| Gain rule for slower refresh | Unresolved | Not applicable | Pending separate choice | Pending separate choice |
| Disk/RAM and full-scan rules | D2–D5 accepted | Unchanged | Unchanged | Unchanged |
| Experiment scope and runner budget | Pending | Pending | Pending | Pending |

D7 — Staged-refresh research scope: ACCEPTED A by the owner (reply: "да" to the explicit recommendation/confirmation of A). Include one bounded experiment; execution budget remains pending. Alternatives below are decision history. Recommended A: include one
bounded staged-refresh experiment in W5, subject to a later fixed runner budget.
The mechanism streams existing snapshot rows into disk-backed staging rather
than loading the entire tree into RAM, preserving refresh semantics. It still
rewrites the snapshot and may increase disk/time; D2/D3/D6 reject promotion if
those metrics regress beyond calibrated uncertainty. An unsuccessful experiment
produces evidence only, not a changed default. B: defer the decision on this
experiment until W1–W4 measurements are available; refresh optimization remains
an objective, but no staged-refresh prototype is in the currently accepted scope.
A directly tests the observed refresh-memory limitation but costs prototype and
validation effort. B avoids that commitment now but delays an answer on refresh
memory. Estimated prototype/proof effort for A: human 2–4 days / agent 6–12 hours,
excluding runner wall time; these are rough planning estimates. B has no prototype
effort now and needs a later scope decision. Neither choice authorizes execution,
weakens metric/correctness gates, or approves a new storage engine.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Staged-refresh prototype | Proposed W5, unapproved | Include one bounded experiment | Defer scope decision until W1–W4 evidence |
| Refresh optimization objective | Owner requested | Retained | Retained |
| Promotion constraints | D2–D6 | Unchanged | Unchanged |
| Runner budget and implementation authorization | Pending | Pending | Pending |

D8 — Minimum useful speed gain for a speed-only candidate: ACCEPTED B by the owner (reply: "B"). Require at least 15% confirmed target-operation acceleration. Alternatives below are decision history.
Recommended A: at least 5% reduction in the target operation's paired median
elapsed time, exceeding calibrated measurement uncertainty, with all three
confirmation pairs improving. B: retain the draft 15% minimum. Both apply only to
a candidate whose claimed benefit is speed; material disk/RAM improvements are
not rejected merely because speed gains are smaller. All accepted cross-metric
constraints remain binding, including D4/D5 if the full scan slows down.
A can retain modest useful improvements from simple changes but admits more
candidates requiring maintenance. B filters small gains but can discard cheap,
reliable improvements. Neither threshold turns a noisy result into evidence.
Both use the same confirmation protocol. Resource-gain thresholds, representative
fixture calibration and aggregate runner budget remain separate pending choices.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Minimum target-operation speed gain for speed-only candidate | Draft 15%, unapproved | At least 5%, beyond calibrated uncertainty | At least 15%, beyond calibrated uncertainty |
| Confirmation | Proposed three improving pairs | Same | Same |
| Accepted disk/RAM/latency constraints | D2–D6 | Unchanged | Unchanged |
| Resource-gain threshold and runner budget | Pending | Pending | Pending |

D9 — Minimum useful peak-RSS reduction: ACCEPTED A by the owner (reply: "A"). Require at least 25% lower peak process RSS for an RSS-savings candidate. Alternatives below are decision history. Recommended A: at least
25% lower peak process RSS for a candidate justified by RSS savings. B: at least
15% lower peak process RSS. Compare identical workloads and a common successful
memory budget; gains must exceed calibrated measurement uncertainty. A requires
substantial memory savings before accepting added implementation complexity;
B retains smaller useful gains but may admit changes with limited practical
benefit. Both retain D2–D8 constraints. In particular, a 15% full-scan slowdown
requires at least 30% improvement under D5; the RSS minimum never weakens that
exchange rule. A separate capacity result (success at a budget where baseline
fails) must not be inferred from RSS and remains a separate acceptance choice.
Both options use the same paired proof; neither authorizes execution. Absolute
capacity targets, disk-only gain threshold and runner budget remain pending.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Minimum useful peak-RSS reduction | Draft 25%, unapproved | At least 25% | At least 15% |
| Full-scan exchange rule | D4/D5: at most +15%, at least 2:1 gain | Unchanged; stricter applicable requirement wins | Unchanged; stricter applicable requirement wins |
| Other accepted constraints | D2–D8 | Unchanged | Unchanged |
| Capacity-success route, disk-only gain and runner budget | Pending | Pending | Pending |

D10 — Minimum useful peak-disk reduction: ACCEPTED A by the owner (reply: "A"). Require at least 25% reduction in application-attributable peak disk allocation for disk-savings candidates. Alternatives below are decision history. Recommended A: at
least 25% reduction in total application-attributable peak disk allocation for a
candidate justified by disk savings. B: at least 15% reduction. Both compare the
same operation, fixture and retained-generation state, including simultaneously
allocated current/previous snapshots, staging and temporary files. Final database
size alone is not this metric. A favors substantial savings but can reject a
simple useful improvement; B captures smaller savings but may admit maintenance
work for a modest return. No extra measurement protocol is needed beyond the
planned paired disk observations. Existing correctness and D2–D9 constraints
remain binding; the stricter D5 exchange rule wins when scan slowdown requires
more savings. Memory-capacity acceptance, workload calibration and runner budget
remain independently pending. Neither choice authorizes execution.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Minimum useful peak-disk reduction | No approved minimum | At least 25% | At least 15% |
| Peak-disk accounting | Full simultaneous app allocation | Unchanged | Unchanged |
| Accepted correctness/resource/latency constraints | D2–D9 | Unchanged | Unchanged |
| Memory-capacity acceptance, calibration and runner budget | Pending | Pending | Pending |

D11 — Lower-memory-capacity success as an alternative benefit: ACCEPTED A by the owner (reply: "A"). At least 25% lower minimum tested successful container budget is an alternative memory-benefit route, subject to the proof and unchanged gates below. Alternatives below are decision history.
Recommended A: accept a verified reduction of at least 25% in the minimum tested
successful container memory budget as an alternative to D9's 25% peak-RSS
reduction. Require repeated candidate success at the lower budget and reproduced
baseline OOM at that exact budget; baseline must succeed at its higher budget.
Use identical workloads, CPU/I/O conditions and swap policy, with semantic
comparison to a successful baseline. Report tested budgets, not a theoretical
minimum. A reflects actual deployability but needs additional capacity trials;
B requires D9's 25% RSS reduction even if lower-budget operation succeeds, using
capacity trials as supporting evidence only. B retains a simpler benefit gate
but can reject a practical reduction in required RAM. D2/D3/D6 still apply;
latency comparison uses a common successful memory budget. A baseline OOM is
not treated as infinite elapsed time. This exception alone does not satisfy D5's
speed/resource exchange rule: a slower full scan still needs its independently
approved 2:1 measured gain in disk, peak RSS or refresh time. Capacity trials are
charged to the later aggregate runner budget; no run is authorized here.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Alternative memory-benefit gate | Draft capacity success, unapproved | At least 25% lower tested successful budget, with paired baseline failure evidence | None; D9 RSS minimum required |
| D9 RSS-based benefit route | At least 25% reduction | Retained as alternative | Required for memory benefit |
| Other resource/latency constraints | D2–D10 | Unchanged | Unchanged |
| Calibration and aggregate runner budget | Pending | Pending | Pending |

D12 — Local-comparability evidence: OWNER COUNTERPROPOSAL ACCEPTED. Owner explicitly authorized one immediate local calibration scan with measurements, and deletion of old databases if required for space. Do not launch a standalone disk benchmark or additional scans. Existing GH-only restriction is relaxed for this one calibration only. Original A/B alternatives below are history.
Recommended A: start from existing 7,202,464-entry inventory and passive local I/O
observations; use runner-native plus bounded I/O sensitivity profiles and varied
fixture shapes. Label results as synthetic local-scale evidence, not an exact
replica or a forecast of local elapsed time. Existing observed local scan I/O is
not device maximum throughput: medians were roughly 1,022 read IOPS / 4.806 MiB/s,
with burst behavior that the old 1,000-IOPS cap suppresses. B: require another
passive observation of the next already-scheduled local scan before fixing the
runner profiles. Do not launch an extra local scan, fio, cache flush or traversal.
B may improve current workload evidence but delays profile finalization and still
cannot establish exact hardware equivalence. A permits earlier runner work using
known evidence but leaves representativeness uncertainty explicit. Both preserve
GH-only heavy work, 7.2m confirmation scale and all accepted metric gates; neither
starts any jobs now. Runner spend and exact disk-headroom reserve remain pending.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Initial profile evidence | Existing entry count and passive I/O, incomplete equivalence | Use existing evidence with explicit sensitivity bounds | Await next already-scheduled scan's passive evidence |
| Additional local workload | Owner: avoid local load | No benchmark/scan/traversal | No benchmark/scan/traversal; passive counters only |
| Exact local-equivalence claim | Unsupported | Not claimed | Not claimed |
| Runner budget and disk reserve | Pending | Pending | Pending |

D13 — First GH campaign aggregate time cap: ACCEPTED A (6 runner-hours), owner answer via asynchronous question. Subsequent billing clarification: this public repository uses standard ubuntu-24.04 runners, whose compute time is free. This is an experiment-duration cap, not a monetary budget or GitHub free-minute quota. Owner has questioned the cost framing but has not yet changed the selected cap. Alternatives below are history. Asked via asynchronous
question while the separately authorized local calibration runs. Recommended A:
6 runner-hours. B: 12 runner-hours. Scope: initial harness validation and baseline
v1 campaign only. Sum all job durations, including builds, preparation, failures
and concurrent jobs; no automatic retries beyond the cap and no transfer of this
budget to later optimization campaigns. Admit a full job only if its projected
total duration plus margin fits the remaining budget. A limits aggregate runner time but may
require a later budget decision to finish confirmation; B permits more full-scale
work at twice the allowed aggregate runner time. Neither is a promise that the complete matrix
fits. The local scan is separately authorized and excluded from this GH budget.

Local calibration execution receipt (D12): started the installed binary with
explicit v1, low-memory and one worker in transient unit
`freespace-calibration-20261031.service`. Output directory:
`/var/lib/freespace/calibration-20261031`; observer artifacts:
`/tmp/freespace-calibration-20261031`. Existing production config and timer are
unchanged; no old databases deleted. Initial free space about 22.6 GiB. Observer
stops the owned scan below 8 GiB free or above 10 GiB calibration allocation;
unit runtime limit two hours. These are operational stop guards, not measured
requirements or GH budget decisions. No cache flush or standalone fio performed.
The observation completed: 1779.09 s, process max RSS 27.660 MiB, sampled
peak calibration allocation 1.8046 GiB, minimum free space 20.7959 GiB. Exit 0
but incomplete due to two inaccessible temporary paths. This first-generation,
uncleared-cache result is a workload observation, not a steady-state disk peak
or a clean comparative benchmark. Receipt: ../benchmark-results/local-v1-calibration-2026-10-31.json.

D14 — Absolute RSS saving floor for the D5 exchange: ACCEPTED A by the owner (reply: "A"). Require both the 2:1 percentage rule and at least 256 MiB absolute peak-RSS savings when RAM savings justify full-scan slowdown. Alternatives below are history, prompted by
Opus M2. At a historical scan RSS near 23 MiB, a 30% reduction is only about
7 MiB but can currently justify 15% scan slowdown (roughly 50 seconds). Recommended
A: a slowdown justified by RSS savings must meet BOTH the existing 2:1 relative
rule and at least 256 MiB absolute peak-RSS savings. B: retain the percentage-only
rule. The proposed floor is an owner choice, not a measured optimum. This changes
only RSS-based justification of a slower full scan; resource improvements without
slowdown, disk/refresh benefit routes and the 15% ceiling remain unchanged. A
avoids small absolute benefits buying substantial latency; B preserves a simpler
rule but allows that exchange. No candidate is implemented by this answer.

D15 — Host refresh qualification measurement: ACCEPTED A by the owner (reply: "A"). One local measured refresh of a small source folder in the isolated calibration snapshot is authorized. No production backend switch or extra full scan. Alternatives below are history. Recommended A:
authorize one measured local v1 refresh of a small existing source folder using
the isolated calibration snapshot and installed binary. Do not switch the
production backend, start a second full scan, or delete old databases. Use the
same capacity protections and collect elapsed time, process/cgroup memory and
peak allocation. This directly tests the host-refresh concern in Opus H6 and
informs a later cutover decision; it accepts no latency regression relative to
v2 by itself. B: keep additional local refresh measurement out of scope and
qualify on GH only for now; actual-host refresh readiness remains unverified and
must be resolved before cutover. A adds local load for one operation; B avoids
that load but delays host-specific evidence. D12 authorized one full scan only,
so this is a new operation, not a repeated permission request.

## Outside-review dispositions and protocol refinements

External receipt: [Opus review](v1-performance-opus-review.md). The external
review is evidence, not approval. D14 resolves its absolute-RSS exchange concern;
D15 authorizes the host refresh measurement but does not approve cutover.

- H1: six aggregate runner-hours covers initial W1/harness/baseline evidence only,
  not all six waves. Allocate at most 60 minutes to build/smoke, 120 minutes to
  baseline/base calibration on the selected native profile at screening scale,
  150 minutes to an admitted local-scale baseline pair, and 30 minutes contingency.
  These are maximum allocations, not duration predictions. All overhead/failures
  count. Each dispatch timeout is bounded by both its lane remainder and the
  campaign remainder. If forecast plus margin does not fit, stop with incomplete
  qualification; do not shrink a fixture and relabel it full scale. No unused
  allocation silently authorizes another campaign. A revised allocation requires
  recording it before dispatch within the accepted total cap.
- H2: freeze the uncertainty protocol before candidate results. For each measured
  profile/scenario/scale, collect three baseline/base pairs with interleaved order;
  record paired absolute/relative deltas, median and maximum absolute spread. The
  observed spread is an empirical screen, not a confidence bound. Do not transfer
  small-fixture uncertainty to 7.2m or native uncertainty to limited I/O. If the
  budget cannot establish the needed control, that scenario is unqualified.
  A no-regression gate cannot pass merely because a regression is statistically
  unconfirmed: a positive median degradation, an out-of-band worse pair, unstable
  controls or incomplete evidence yields inconclusive/reject, not promotion.
  Benefits must meet their owner-approved floor beyond the empirical noise band
  and improve in every confirmation pair. Report uncertainty honestly; three
  pairs cannot establish tight population-level guarantees.
- H3: source inspection shows hardlink ownership is selected from the complete
  staged links table before leaf insertion, rather than reassigned on later DFS
  visits. Keep adversarial byte-name/path-order fixtures as proof; do not report
  the reviewer's hypothetical late reassignment as an existing defect.
- H4: constrain batch/cache experiments to the accepted RAM budget. Use a global
  retained-byte budget and measure RSS; reject at screening if it grows. Default
  larger-cache arms are removed; this narrows mechanics to the accepted contract,
  not a weakening of the RAM gate.
- H5: W5 first runs the existing 1m screening stage. A confirmed increase in disk
  or refresh time stops that candidate before 7.2m confirmation. An inconclusive
  result is not promoted. The accepted bounded experiment can fail; do not reopen
  D2/D6 just to make it succeed.
- H6: local refresh measurement completed under D15: 66.61 s elapsed, 1822.36 MiB process peak RSS, 2 subtree entries out of 7,274,483 total. See the local refresh receipt. D16 subsequently identifies the host as a test machine and authorizes backend
  switching as useful. D6 compares candidate v1 with baseline v1; it does not
  establish v2-to-v1 refresh parity. No further test-host cutover gate is needed.
- M1: further 25% v1 disk reduction has no demonstrated mechanism yet. Separate
  v2-to-v1 operational savings from improvements over v1; a failed disk hypothesis
  is not relabeled successful merely because old databases were deleted.
- M3/L3: if a memory sweep passes its lowest rung, report it as floor-censored,
  not a measured minimum. Extend downward or bisect only within an explicit
  campaign allocation. For a claimed D11 boundary, require 3/3 candidate successes,
  3/3 baseline OOMs at the lower budget and 3/3 baseline successes at its higher
  budget, preserving diagnostic receipts and semantic checks. Mixed outcomes are
  inconclusive. No finite grid proves a theoretical minimum.
- M4: local calibration observes installed v1 workload I/O, first-generation
  capacity and wall time; it does not measure maximum disk capabilities or directly
  calibrate steady-state retained-snapshot peak. No cache flush was performed.
  Additional distribution extraction, if needed, must use bounded metadata queries
  and must not be presented as already collected. Unknown source provenance remains.
- M5/L1: exactly six accepted waves; extra scope requires a decision. W2 execution
  consumes W1 admission; W5 consumes the W4 outcome even if no W4 arm is promoted.
- L2: W1 checks harness coverage by existing lint/size/test gates. Add new languages
  to size enforcement together with their linter where the repository rule applies.
  Update pregate metadata only for actual added test files; no fictitious e2e suites.
- L4: declare the same peak-allocation sample interval for both arms, record missed
  samples and observer overhead, and count open unlinked files. Faster candidates
  can hide transient peaks; validate sampling sensitivity during baseline controls
  and label all sampled peaks as lower bounds. Unresolved sampling differences
  prevent a peak-disk promotion claim.

## Completed local calibration and cutover decision

Receipts:
- [Full scan](../benchmark-results/local-v1-calibration-2026-10-31.json):
  7,274,483 entries; 1779.09 s elapsed; 27.66 MiB max process RSS; 1.8046 GiB
  sampled application allocation peak, starting without prior generations.
- [Small-folder refresh](../benchmark-results/local-v1-refresh-calibration-2026-10-31.json):
  2 subtree entries, 66.61 s elapsed; 1822.36 MiB (1.780 GiB) max process RSS;
  1.9106 GiB sampled application allocation peak including the prior generation.
  Load 16.155 s, subtree walk 0.00148 s, finalize 6.636 s, insert 11.855 s,
  index 30.916 s, publish 0.368 s. The full immutable rewrite dominates.

Both commands exited 0 but inherited/reported two inaccessible temporary
subtrees; neither is a clean completeness pass. No old database was deleted,
no second full scan or extra refresh was launched, and production backend/timer
configuration remains unchanged. These single observations are not paired v1/v2
comparisons. The minimum successful memory budget has not been measured locally.
Local achieved I/O distributions are workload evidence, not disk throughput caps.

D16 — Test-host backend choice: RESOLVED by owner clarification: this machine is used for tests, and the agent may switch to whichever backend is useful. No separate early-cutover approval is required. This does not approve unlimited additional local workloads. Current backend need not be changed without an actual test purpose. Original alternatives below are history, reopened by the new D15 host
measurement. Recommended A: remove early cutover from the active execution path;
keep v1 as the optimization baseline on GH, and revisit installed-backend cutover
only after refresh improvements and measured qualification are reviewed. This
preserves the current installed behavior but defers v1 operational disk savings.
B: keep early cutover eligible, explicitly accepting that the measured v1
small-folder refresh took 66.6 seconds and about 1.78 GiB process RAM. This enables
earlier v1 disk/storage simplification but accepts that observed refresh cost.
Neither choice changes optimization gates, authorizes an immediate production
migration, or establishes exact local v2-to-v1 ratios. D1–D15 remain binding.

| Commitment | Current | A | B |
| --- | --- | --- | --- |
| Early v1 cutover path after W1/W2 | Proposed, host refresh cost previously unknown | Defer until refreshed evidence and acceptance | Retain eligibility with measured v1 refresh cost acknowledged |
| V1 optimization baseline | Accepted | Unchanged | Unchanged |
| Actual deployment | Not authorized by planning choices | Not authorized | Not authorized |
| Existing metric gates | D2–D14 | Unchanged | Unchanged |

## W3 candidate preparation and campaign-scope choice

Prepared in isolated worktree `/tmp/free-space-v1-leaf`, published branch
`perf/v1-leaf-fast-path`, based on W1 revision accfe9d. Commit6b70986 skips child
queries for known non-directory nodes; commit93b2e2d additionally skips their
redundant final UPDATE. Directory traversal, hardlink attribution and parent
aggregation remain unchanged. Differential fixture now asserts leaf time/subtree
invariants, zero-sized aliases, empty directories and future mtime clamping.
The candidate passed Rust workspace tests on GitHub (318 passed, 0 failed, one
existing ignored test) and normal repository gates; added STAT_ERROR and
non-directory-root cases executed. The
[Opus implementation review](v1-leaf-implementation-review.md) found no P1/P2.
Neither source review nor correctness checks constitute performance promotion.

The measured combination is **not promoted**: paired median scan acceleration
was 14.29% at 1m files and 11.92% in one 7.3m-entry pair, below the fixed 15% D8
floor. No useful disk/RAM saving was demonstrated. No further W3 confirmation job
is planned. Independent SELECT-only/UPDATE-only timing and diagnostic statement
counts were not collected; no separate mechanism is accepted from the combined
result. W4 preparation compares with unchanged v1, not this rejected combination.
The [campaign report](../benchmark-results/v1-w1-w3-campaign-2026-10-31.md) records
all results, protocol refinements, uncertainty and missing qualification.

D17 — First-campaign scope expansion: ACCEPTED A by the owner via asynchronous reply. Candidate tests/measurements may use only the remaining part of the same six aggregate runner-hours after baseline controls succeed. Original alternatives below are history. Recommended A: permit this W3
candidate's GH tests/measurements after baseline controls succeed, using only the
remaining part of the same six aggregate runner-hours. Keep all accepted gates;
if full qualification does not fit, retain preliminary evidence and stop. B:
finish only W1/harness/baseline within the original allocation; defer candidate
campaign allocation. This changes campaign scope only, not its time cap or any
metric/correctness requirement. No candidate job starts before the actual answer.

Execution receipt: D17 was received before W3 dispatch. The completed campaign
used 8,118 of 21,600 aggregate runner-seconds; 13,482 remain and no job is active.
Further W4 runner use needs a new explicit scope allocation; preparing its
already-planned source experiment is reversible work and does not launch jobs.

## Implementation Tasks

Estimates are planning ranges, not commitments. Human/agent time excludes runner
wall time and unknown iterations; measured work cost must be recorded separately.

- [ ] T1 (P1, human 1–2 days / agent 3–6h): W1 paired v1 harness, capacity/time admission, receipts, observers and memory accounting. Files: `scripts/bench-v2-container.py`, `bench_v2_config.py`, `bench-representative.sh`, `summarize-v2-container.py`, harness tests, representative workflow. Verify C7/C8 and base/base controls. Source P1/P2.
- [ ] T2 (P1, human 0.5–1 day / agent 2–4h): W2 disposable-history runbook and runner rehearsal. Files: deployment docs, proposed cutover harness/tests; reuse `cli/src/backend.rs` and install contracts. Verify C1/C2/C6. Source A1/A2.
- [ ] T3 (P2, human 0.5–1 day / agent 1–3h): W3 independent leaf SELECT/UPDATE eliminations and differential cases. Files: `core/src/scan/staged/write.rs`, `core/tests/staged.rs`. Verify C3/C6 plus paired promotion. Source Q1.
- [ ] T4 (P2, human 1–2 days / agent 3–6h): W4 bounded child batches and independent source/output cache trials. Files: staged writer/configuration and tests. Verify C3/C4/C8; keep unsuccessful variants out of default. Source Q2/P2.
- [ ] T5 (P2 experiment, human 2–4 days / agent 6–12h): W5 staged refresh prototype and hardlink/metadata correctness. Files: `core/src/refresh.rs`, refresh loading helpers, staged ingestion, CLI refresh policy and core/CLI tests. Verify C5/C6/C8; do not ship if disk/time gates fail. Source A3/T1.
- [ ] T6 (P1 for release, human 0.5–1 day / agent 2–4h): W6 final artifact table, distribution-target build, CI guard integration and concrete host cutover package. Files: workflows, release/install docs and the selected optimization diff. Verify relevant full CI and current manifest; observe next authorized scheduled scan. Source T2/P3.

P1 denotes a prerequisite for shipping its affected wave, not a demand to complete
all future research before releasing a safe independent improvement. There are
six planned wave PRs; conditional index/splice/representation research is not an
extra mandatory PR or hidden implementation commitment.

## Review completion summary

D1–D16 resolved; local scan and refresh completed. Six delivery waves, eight
meaningful test contracts and failure handling remain mapped to implementation.
The initial internal review identified 13 technical issues. External Opus review
identified 15 findings; source-checked dispositions are recorded above. Some
findings exposed protocol gaps, some need future candidate proof, and H3's proposed
late-hardlink-reassignment mechanism was not supported by source inspection.
Five factual/performance premises remain unverified; no measured optimization
result is claimed. Current review completes planning, not implementation gates.

## Outside review receipt

First attempt: configured `claude-fable-5-1` failed with HTTP 429,
`credits_required` / `model_requires_usage_credits`; organization overage disabled.
The gstack parser expected one JSON object and hid the CLI event-array error.
Second attempt: owner explicitly selected `claude-opus-5-5`; exit 0, reported
modelUsage matches the requested model. An invocation-local adapter normalized
the final result object from the CLI event array. Global gstack settings and code
were not changed. [Full review and initial triage](v1-performance-opus-review.md).
The reviewer saw the first 30 KB of the then-current plan plus all D1–D13 decisions,
with no repository access; subsequent D14–D16 decisions and local results are
parent-reviewed additions, not an external clean-pass claim.

## Suppressed findings

No low-confidence code-defect claims are used as implementation mandates. Runtime
gains, local-shape equivalence, exact installed provenance and minimum RAM remain
the five explicitly unverified premises above. The plan does not invent results
for unbuilt candidates. No new broad feature or unrelated TODO was introduced.

## Reality checks

Source history and origin/master were read before tasks were drafted. No relevant
revert was found in the named staged/refresh paths. No new pool, driver, transaction
mode or concurrency protocol is proposed. Main/native historical reproduction is
on product master afce6fd; candidates and the new protocol require fresh evidence.

REALITY: premise-table finding unverified=5; gains/budgets/shape/provenance/limited baseline remain unknown docs/designs/v1-performance-plan.md:67
REALITY: already-done pass origin/master fetched at afce6fd; existing v1 default/staged path reused, no duplicate rollback docs/designs/v1-performance-plan.md:98
REALITY: surface-check pass backend/config/install behavior read in authored registry; proposed interfaces labeled future docs/designs/v1-performance-plan.md:376
REALITY: binding-decisions pass v1-first, GH-only and disposable snapshot decision retained docs/designs/v1-performance-plan.md:10
REALITY: numbers pass historical evidence marked for remeasurement; prospective caps/gates and effort not claims docs/designs/v1-performance-plan.md:990
REALITY: deferred-asks pass no linked issue; all four requested metrics covered and exclusions explicit docs/designs/v1-performance-plan.md:447
REALITY: pr-count pass six waves one PR each; conditional research not hidden mandatory PRs docs/designs/v1-performance-plan.md:132
REALITY: transaction-semantics pass immutable output/fsync/rename retained; failures/reader handling mapped to C6 docs/designs/v1-performance-plan.md:334
REALITY: units pass GiB MiB seconds explicit; RSS vs cgroup vs capacity separate docs/designs/v1-performance-plan.md:263
REALITY: perf-table finding historical one-repeat limitation; versioned interleaved pairs and no p95 claim proposed docs/designs/v1-performance-plan.md:213

## Saved QA and task artifacts

- QA plan: `/home/tass/.gstack/projects/mikhailsal-free-space/tass-perf-representative-gh-bench-eng-review-test-plan-20261031-114936.md`.
- Flat task JSONL: `/home/tass/.gstack/projects/mikhailsal-free-space/tasks-eng-review-20261031-114936.jsonl`.
- Both are planning artifacts. The owner-authorized local scan and refresh were
  completed and archived; no candidate optimization or GH campaign was executed.

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
| --- | --- | --- | ---: | --- | --- |
| CEO Review | Not invoked | V1-first direction settled | 0 | Not required | Scope preserved |
| Outside Review | Claude Code via plan-eng-review | Independent system | 2 | COMPLETED WITH FINDINGS | Fable unavailable; Opus completed with 15 findings, dispositions recorded |
| Eng Review | plan-eng-review, resumed interactively | Architecture, quality, tests, performance | 1 resumed | ISSUES OPEN FOR IMPLEMENTATION | D1–D16 resolved; future correctness/performance proof still required |
| Design Review | Not invoked | No visual redesign | 0 | Not applicable | Existing UI contract retained |
| DX Review | Not invoked | No new public CLI contract | 0 | Not required | Preserve existing contract |

OUTSIDE COVERAGE: Claude Code, requested/reported claude-opus-5-5, completed;
partial plan-body coverage plus all then-current decisions. No external re-review
of later amendments is claimed.

VERDICT: DONE_WITH_CONCERNS. Planning decisions resolved; local calibration and
external findings incorporated. Optimization gains, exact host/runner equivalence
and successful lower-memory budgets remain unproven. Test-host backend switching
is authorized; candidate implementation/qualification and normal repository gates
remain necessary before declaring any optimization successful.

NO UNRESOLVED OWNER DECISIONS FOR COMPLETED D17 WORK. W3's combined candidate was
not promoted. NEXT OWNER DECISION D18: two bounded W4 screening jobs within the
existing remainder, or stop the first campaign. No new job starts without the
answer; the same six-hour cap and existing metric gates remain binding.
