# Proposed refresh allocation diagnostic

Status: diagnostic source prepared and published; no runner allocation or
optimization acceptance. Normal pre-push checks and 65 tiny Python contract tests
passed. The [Opus source review](v1-refresh-allocation-probe-review.md) closes its
findings with a final no-findings verdict. Rust workspace execution remains for
the proposed GH build. This is a narrow investigation before an allocation fix or the
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

## Proposed two-stage allocation — not authorized

After D18, 10,985 aggregate runner-seconds remain under the same six-hour cap.
At most two screening jobs with a 55-minute measured-step cap would each reserve
5,100 seconds (25-minute build + 60-minute paired job), or 10,200 total. This
would leave 785 seconds unreserved. A conservative 2,400-second whole-job forecast
plus 30% margin is 3,120 seconds, below the 3,300-second measured-step cap. These
are proposed envelopes, not dispatches or promised durations.

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
No full-scale expansion is included in this proposed allocation.

## Owner decision D19 — pending before any dispatch

Permit this two-stage investigation within the existing remainder, or close the
initial campaign after W3/W4. This is a new scope choice because D18 authorized
only its completed control and child-batch candidate jobs. Source preparation is
reversible and does not reserve or consume new runner time.
