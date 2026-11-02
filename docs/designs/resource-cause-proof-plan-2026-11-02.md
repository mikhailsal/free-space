# RAM and disk: causal proof campaign

Date: 2026-11-02. Status: protocol preparation; no new run admitted yet.

The owner requests causal explanations of application RAM and disk costs,
evidence and documentation, and improvement hypotheses including storage-stack
changes. This expands research beyond the earlier v1-only implementation plan;
it does not authorize a production rewrite or change its promotion criteria.
Heavy execution remains on dedicated GitHub runners within the existing 60-hour
aggregate cap. Installed application settings and all stopped local timers stay
unchanged.

## Required outcome and current evidence gaps

| Requirement | Evidence needed | Current state |
| --- | --- | --- |
| Explain full-scan RAM | Controlled interventions separating directory enumeration, per-entry metadata, the full engine and collector effects | Large reclaimable-slab/file observations exist; their responsible operations are not yet isolated |
| Explain refresh RAM | Source, allocation-capacity observations and an allocation-strategy counterfactual | Whole-tree reconstruction and vector growth are observed; exact-capacity intervention reduced full-scale RSS in one pair; allocator-internal byte attribution is unproven |
| Explain persistent and peak disk | Unique-inode lifetime budget, page categories and measured release steps | Source proves current/previous/replacement/stage overlap; independently sampled maxima cannot prove an exact simultaneous budget |
| Explain v2 growth | Repeated unchanged-scan row/page observations tied to actual write/retention paths | Complete new version sets on full scans are established; small refresh is a different path; GC byte return is unmeasured |
| Propose improvements and stack alternatives | Mechanism, saving ceiling, tradeoffs, falsifier and a fair prototype proposal | Primary-source alternatives are being documented; no engine is assumed faster or smaller |
| Make the argument auditable | Frozen sources, actual binary identity, raw evidence, failed cases and explicit inference limits | Existing records are retained; new diagnostic outputs still required |

The independent local disk-full incident was caused by another project's nightly
whole-tree test copies. Its evidence stays private and is not attributed to this
application or used to claim an engine improvement.

## Fixed application and measurement boundary

Use the already-built musl baseline `e8265778ef78ec4acd9da1375eedb0be37a123f5`,
actual SHA-256
`ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`.
Its source epoch is 1793464183. Retrieve it on GitHub from successful run
41277463149, artifact 13071807048 (`representative-binary`), checking the run,
build record and actual payload. The diagnostic branch starts at same-app
control `4f22bf8e3b67f7f59049a72116b90c061fb1328b`; application code is unchanged.
No local application execution or new Rust build is required. The registered
`v2-balance.yml` dispatch slot is overridden only on the isolated diagnostic
branch; the canonical `representative-bench.yml` stays byte-identical to the
control and its existing tests remain applicable. Default-branch workflows are
not changed.

Reuse the existing mixed fixture generator, dedicated ext4 loop filesystem with
verified direct backing I/O, local I/O envelope, two baseline seed scans,
immutable seed copies, cold-cache request, separate semantic observer and
bounded cleanup. Setup, collectors and observers are not silently included in
CLI latency or excluded from runner-time accounting. Record their own scopes.

Each measured worker gets a fresh 4096-MiB, two-CPU, no-swap cgroup and one
application worker. Host slab counters are supporting global context, not
per-worker attribution. Required raw counters include current/peak/stat, local
and hierarchical events, memory/I/O pressure, I/O and CPU statistics. Preserve
unreported device counters rather than inventing zeros.

## RAM interventions

The four diagnostic modes use a common measurement worker and unchanged fixture:

* `idle`: one-second empty wrapper control; zero traversed entries, explicitly
  distinct from a scan and never presented as a product semantic success.
* `names`: streaming `readdir`, explicit `d_type`, directory opens and one common
  root stat; no per-file metadata lookup or payload read.
* `stat`: the same enumerator plus one explicit no-follow metadata lookup for
  every non-root entry, with inode/type parity checks.
* `scan`: the exact baseline CLI, forced staged v1, complete publication and
  independent current/previous validation.

The enumerator rejects unknown directory-entry types rather than hiding a stat
fallback. It streams bounded traversal state and raw-byte path/type digests.
At 1m files the existing generator yields 1,004,499 entries: 1,000,000 regular
file direntries, 4,499 directories including root and 999 hardlink aliases.
`names` and `stat` must agree on counts and their multiset digest; `stat` adds
exactly 1,004,498 explicit metadata calls. Limits and smaller-smoke counts are
derived from its own fixture receipt, not extrapolated from rounded labels.

Screening uses four Williams orders, balanced for position and first-order carryover:
`idle,names,scan,stat`; `names,stat,idle,scan`;
`stat,scan,names,idle`; `scan,idle,stat,names`.
This is an exploratory mechanism screen, not a population confidence interval.

The existing 20-ms occupancy sampler is a separate question. Three adjacent
full/reserve-only scan pairs alternate order. Both retain watchdog and free-space
checking; reserve-only omits repeated pathname and sampled-memory collection.
Unavailable sampled peaks are null and explicitly labelled, never reported as
zero or accepted by ordinary performance qualification.

Both variants require successful free-space checks with a maximum observed
check-start gap of 250 ms, including initialization and finish boundaries.
Missed or late 20-ms ticks are descriptive collector diagnostics; they are not
themselves grounds to replace an observed result. Occupancy peaks remain sampled
lower bounds even when reserve coverage passes.

The following conclusions are deliberately different:

* A reproducible `stat - names` slab increase shows that the added metadata
  lookups are sufficient to create that additional charge in this environment.
* Similar magnitudes in `stat` and `scan` support a dominant metadata-workload
  explanation; their difference is not an additive SQLite/heap decomposition.
* Full/reserve-only pairs bound the effect of the collector variant. They do
  not calibrate new phase instrumentation, another scale or another cap.
* Global `dentry`/inode slab changes support object hypotheses but are not
  automatically allocations of the measured cgroup.
* The true kernel peak and sequential category samples remain separate.

The fixed exploratory decision policy is `resource-cause-outcomes-v1`.
Its two primary measures are the post-CLI `slab_reclaimable` difference and
frozen true worker `memory.peak` difference for `stat - names`. Support requires
all four differences in both measures to exceed the larger of 64 MiB and the
cross-round ranges of the `names` and `idle` controls. The proposed material
extra-charge claim is rejected in this screen only if every absolute difference
and both control ranges are at most 16 MiB; every other outcome is unresolved.
These are preregistered screening rules, not confidence intervals or universal
claims of no effect.

Collector comparability requires all three full/reserve pairs within absolute
bounds: total peak max(16 MiB, 2% of reserve), RSS 1024 KiB, and time max(0.5 s,
5% of reserve). A comparable metadata/complete-scan mechanism requires primary
support, collector comparability, and every round's stat/scan ratio at least
0.75 in both slab endpoint and frozen kernel peak. Failure leaves that stronger
claim unresolved; it does not turn `scan - stat` into SQLite attribution.

## Disk intervention without changing application code

Run one separately labelled descriptor-retention diagnostic after normal runs.
Open the seed's old previous inode and the newly created stage inode read-only,
keeping their descriptors through the baseline CLI's normal unlink/rotation.
Also retain each inode through an exclusive hardlink in a private sibling
inspection directory on the same filesystem. Tiny SQLite tests show that opening
a naturally unlinked database through `/proc/self/fd/N` is unsupported here; the
named aliases permit read-only finalized `dbstat` inspection without copying
payload bytes. Record this deliberate retention intervention and deduplicate
its aliases. The application still publishes normal current and previous
snapshots.

After CLI exit, while the measurement wrapper is alive:

1. Verify inode identity and expected unlink/replacement; inventory named files
   and retained descriptors with device/inode deduplication and timestamp bounds.
2. Read finalized current/previous, old-previous and stage page categories using
   bounded read-only immutable SQLite connections. Record page/freelist counts,
   schemas, dbstat availability, payload and unused bytes. Bind each inspected
   database to its held inode, record SQLite/header versions and auto-vacuum,
   require rollback-format header and the stated no-pointer-map/<1GiB page
   equality scope. Close all inspection
   connections before releasing descriptors.
3. Remove only the owned inspection aliases and require link count zero on both
   retained descriptors. Release the stage descriptor, then old previous,
   recording immediate and
   bounded settling filesystem observations. Compare freed allocation with
   each inode's allocated blocks and report filesystem residual explicitly.

During this disk-only pass, a 50-ms host watcher records timestamp-bracketed
path identity transitions and only the worker's exact cgroup processes' open
removed files. It bounds PID/FD counts and records races/missing coverage;
normal unlink/FD-close/allocation races do not become observed absence.
Largest bracketed sums are not atomic simultaneous peak bounds. The quiescent
held inventory plus release steps is the stronger allocation evidence. Remaining
filesystem differences stay unattributed and can include SQLite temporaries,
metadata and sampling-time differences. Separate maxima are never added.

This deliberately extends lifetimes in a diagnostic pass. It proves retained
inode costs and release steps; it is not an ordinary latency/RAM benchmark or
an exact measurement of the unmodified run's true maximum. Reconcile its stable
inventory with ordinary sampled peaks and the source lifetime argument without
summing independent maxima. Hardlink rotation creates aliases, not a fourth
copied generation. Index pages are already part of their database files.

## Stages, stop rules and budget

1. A 100k-file protocol smoke: one pass of each RAM mode, reserve-only scan and
   retained-descriptor disk pass. It proves protocol/count/semantic/cleanup
   behavior, not an optimization effect.
2. Only after valid smoke: the fixed 1m screen above, including four balanced
   RAM cycles, three collector pairs and one disk diagnostic. Any later scale,
   stack prototype or capacity test needs its own prospective case.

Each proposed run has one standard-runner job capped at 60 minutes and a
45-minute measured/setup step. Initial whole-step forecast is 1800 seconds;
30% margin is 2340 seconds, below 2700. This allowance is prospective and must
be checked against smoke before screen admission. No reservation exists until
the reviewed source and actual manifest are frozen. At preparation, 50,644
runner-seconds were completed, 34,800 reserved for older conditional cases and
130,556 unreserved under 216,000. The campaign ledger is authoritative.

The second-stage forecast is computed by `cause_forecast.py` from valid smoke
fixture/seed and per-case host timings. It scales entry-dependent work by ten,
uses the fixed screen case multiplicities, retains observed outer setup, and
applies a 30% margin. Require host margin <2100s, whole-step margin <2700s,
and host margin + observed outer setup +240s cleanup reserve <2700s before
manual screen admission. A workflow input is not the financial admission gate:
the root dispatcher alone checks/fixes the source freeze, smoke, forecast and
ledger before invoking the Actions API; the raw workflow could be intentionally
invoked outside that controlled process. No automatic progression is claimed.

Stop on missing/contradictory provenance, count/digest/semantic failure,
unknown probe type, unreadable required counters, foreign running containers,
pre-case host MemAvailable below5GiB or memory PSI some avg10 above1%,
swap/OOM, cap/reclaim censoring of the intended unconstrained comparison,
failed cache/DIO/I/O-limit receipts, disk/inode reserve failure, timeout,
acknowledgement or cleanup failure. Preserve failures and charge their time.
Transport-only evidence retrieval can be repeated; no measurement is silently
rerun to obtain a preferred result.

The host records before/after global observations while each worker waits for
a bounded acknowledgement after its timed command. This hold is outside CLI
timing; the frozen post-command snapshot is used for timed accounting because
live peaks may increase during inspection. No phase marker is enabled in the
first campaign, and no stack improvement or H2 waiver follows from it.

The 2700-second absolute step deadline starts before runner preparation. Setup
commands have bounded deadlines. The host gets at most2100s, reduced to retain
240s for cleanup; its outer timeout permits60s of protected host cleanup within
that reserve. The host deadline covers fixture setup and all cases,
with bounded per-command deadlines, a 90-second acknowledgement hold and
mandatory owned-resource cleanup. The outer job still charges all setup,
inspection and cleanup time. The independent summary requires the fixed
schedule, exact source/binary identity, cold-cache and effective I/O receipts,
counts/digests/semantics, frozen cgroup boundaries, probe seed-integrity receipts,
disk page/inode arithmetic and successful outer cleanup. Per-group receipt
statuses preserve valid RAM/collector evidence if a later disk pass fails, while
the entire campaign remains invalid and cannot advance. Every summary remains
`qualification=false`.

## Research deliverables

The final explanation will map claims to source, intervention and raw evidence;
separate established causes from residuals and hypotheses; quantify attainable
savings under unchanged semantics; and rank the proposals in
[storage-stack hypotheses](resource-stack-hypotheses-2026-11-02.md). Ordinary
product qualification and the earlier H2 verdict remain unchanged.

Existing refresh-RAM and v2-growth evidence is separately verified in
[prior causal evidence](../benchmark-results/resource-causes-prior-evidence-2026-11-02.md).
It is reused at its original binary/toolchain/scale with explicit limits; this
campaign does not falsely claim new refresh or v2 intervention. Physical bytes
returned by v2 GC and replacement-engine performance remain unmeasured hypotheses.
The owner asks for proposed improvement hypotheses, not a shipped engine prototype.
