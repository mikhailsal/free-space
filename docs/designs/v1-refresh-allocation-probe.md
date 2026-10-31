# Proposed refresh allocation diagnostic

Status: GNU diagnostic control completed after owner D19 authorization and a tenfold
total-budget extension; no optimization acceptance. Normal pre-push checks and 65 tiny Python contract tests
passed. The [Opus source review](v1-refresh-allocation-probe-review.md) closes its
findings with a final no-findings verdict. The first GH build and ordinary paired
series passed; the original diagnostic contract failed. See the
[preserved result](../benchmark-results/v1-refresh-allocation-control-2026-10-31.md).
This is a narrow investigation before an allocation fix or the
approved W5 staged-refresh experiment. It does not cancel that experiment or
change publication, history, hardlink or freshness semantics.

## Question and prepared source

The [allocation investigation](v1-refresh-allocation-investigation.md) identifies
a hypothesis: a tiny subtree graft can grow a full-tree node/name vector and
transiently overlap buffers. Actual capacities and the peak's attribution remain
unknown. The local 7.27m-entry/two-entry refresh reached 1.78 GiB process RSS;
the 1m-entry/100-entry GH control reached about 245 MiB. Neither observation
proves a vector-reallocation cause.

The isolated branch `perf/v1-refresh-allocation-probe` starts from d8b6046, with
neither rejected SQL experiment. It adds only diagnostic boundaries and reuses
the existing opt-in `FREESPACE_MEMORY_PROFILE` trace. There is no reserve,
preallocation, SQLite policy, public CLI or default-backend change.
The published source commit is `274d412`; exact hashes are in the review receipt.

The proposed paired runner enables the trace only for measured refresh workers;
seed/full scans and observers remain untraced. An explicit spec flag and manifest
policy record this configuration. Trace files live under each result directory,
outside snapshot storage. Both compared arms use the same diagnostic policy.

Events include refresh start, loaded tree, before graft, after names, after nodes,
after graft and after finalization. Allocation events record scalar node/name
lengths and capacities, actual Node size, buffer-change booleans, boundary RSS and
cumulative VmHWM. Pointer addresses and tree contents are not emitted. The first
buffer-change flags mean false because no prior allocation event exists.
Refresh start is after writer-lock acquisition and just before the existing core
timer begins; external command wall time includes that marker's overhead. Named
phase times from diagnostic runs must not be compared with untraced phase times.

Require the ordered mandatory event subsequence while tolerating existing extra
phase events. For a returned subtree, after-names and after-nodes are mandatory;
for `removed=True`, no append occurs and only common boundaries are required.
The proposed unchanged 100-entry workload would fail ordinary subtree-count
validation if it unexpectedly reported removal.

These are boundary samples and cumulative process peaks, not per-phase peaks or
an allocator live-allocation trace. Pointer movement does not by itself prove
copying or a physical-RAM peak. Disabled diagnostics perform no trace allocation
or `/proc` reads. No per-entry production tracing is added.

## Initial two-stage allocation — D19 authorized

After D18, 10,615 seconds were used. The owner then increased the total cap from
six to 60 aggregate runner-hours and instructed continuation, leaving 205,385
seconds before new D19 jobs. The original proposal's 10,985-second remainder
belongs to the superseded six-hour cap.
At most two screening jobs with a 55-minute measured-step cap would each reserve
5,100 seconds (25-minute build + 60-minute paired job), or 10,200 total. This
initial reservation leaves 195,185 seconds unreserved. A conservative 2,400-second whole-job forecast
plus 30% margin is 3,120 seconds, below the 3,300-second measured-step cap. These
are maximum envelopes, not promised durations. Exact initial revisions and
conditional admission are recorded in the
[manifest](../benchmark-results/v1-refresh-probe-manifest-2026-10-31.json).

1. Diagnostic baseline/base control: 1m files, three scan and three unchanged
   100-entry-refresh pairs, current v3 observer, unchanged `local` I/O, 4 GiB/no
   swap, one worker/two CPU quota units and retained seed generations. Both arms
   use the instrumented baseline. Full Rust workspace tests run before measurement.
2. Only if complete diagnostics support an actionable allocation cause: prepare
   one narrow capacity-management counterfactual, source-review it with the
   requested Opus model, freeze its exact SHA and compare against the same
   instrumented baseline. Repeat the same paired screening and all guards after
   renewed remaining-time admission. No allocation fix is applied by this source
   preparation; its mechanism depends on the evidence from stage 1.

Missing/incomplete traces, missing stages, invalid scalar fields, nonmonotonic
elapsed/HWM values or failed ordinary receipts invalidate the diagnostic purpose,
even if snapshot semantics pass. No automatic retry or parameter sweep. If the
peak does not move with buffer growth or is dominated by later finalization,
report the hypothesis as unsupported and stop the conditional candidate.

The RAM benefit floor remains at least 25% process peak RSS, beyond measured
uncertainty, or the separately specified tested-capacity route. All speed, disk,
freshness and no-regression gates remain unchanged. A successful 4 GiB run is not
a measured minimum RAM budget, and a diagnostic screen is not release qualification.
No full-scale expansion was included in the initial two-job proposal. The
conditional allocation candidate is stopped because the original diagnostic was
invalid and the GNU observations do not establish its proposed cause. A separately
frozen release-target control sequence under the extended budget is described
below.

## Prospective release-target controls

The source selects mimalloc only for musl, while the initial control recorded a
GNU binary. Before testing an allocation counterfactual, add an explicit validated
compilation target to the harness and measure the release musl target with its
existing allocator. Both arms use the same source, toolchain, flags and binary;
this is a new control series, not an optimization or a comparison with installed
binary provenance. Explicit GNU `--target` builds also start a new binary series.

The new `profile-v2` tool describes trace structure separately from counter
accuracy and product acceptance. It requires finite monotonic elapsed time,
valid scalar types, exact node counts and outcome-dependent ordered events, and
recomputes the strict ordinary paired summary. It retains raw VmHWM values,
reports every decrease and exposes the original monotonic-HWM contract result
separately. The archived original failed verdict remains failed. The
[Linux status manual](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html)
documents VmHWM and VmRSS as inaccurate; no clamping, peak correction or inferred
copying is allowed. The [source review](../benchmark-results/v1-refresh-release-source-reviews-2026-10-31.json)
closed all findings with the requested `claude-opus-5-5` and a final
`NO_FINDINGS` verdict before new dispatches. The published source is
`e8265778ef78ec4acd9da1375eedb0be37a123f5`. The four product metric gates are unchanged.

The next bounded sequence is one musl screening control (1m files, three scan
and three unchanged 100-entry-refresh pairs), followed conditionally by one full
7,264,383-file / 7,297,076-entry refresh control pair. The latter requires complete
ordinary/diagnostic structure receipts and a measured screening forecast including
setup, seeds and observers plus 30% margin within its 120-minute measured-step cap.
The job envelopes are 5,100 and 9,000 runner-seconds. Exact immutable revisions,
admission and outcomes are recorded before/after dispatch; no blind retries,
candidate acceptance or unbounded expansion follows from these controls.

## Owner decision D19 — accepted with budget extension

The owner explicitly requested a tenfold increase in the time budget and
continuation. This accepts the prepared investigation, followed by conditional
accepted-plan work under the 60-hour cap. Current stop rules and acceptance gates
remain binding; future workloads are frozen and admitted before dispatch.
