# Paired v1 benchmark protocol

Protocol `v1-single-operation-pairs-v2` includes the mandatory outer-runner
cleanup receipt. Early v1-protocol runs remain pre-hardening evidence only.

W1 extends the existing GitHub-hosted representative runner. Run it only on an
ephemeral standard Ubuntu VM. The guard rejects local execution, including host
cache flushing, fixture generation and Docker measurements.

The `Representative scanner benchmark` workflow defaults to a 100k-file smoke
control on push. Manual stages are `smoke`, `screen` (1m files, three pairs each
for scan and refresh), and `local-scale` (~7.3m entries, one scan pair). Large runs
require a positive screening-based whole-job forecast that fits 120 minutes with
30% margin. Source and fixture counts are recorded; mixed directory structure
means total entries exceed the requested file count.

`comparison=control` uses byte-identical binaries for both arms. It estimates
observed variability, not a population confidence interval. `comparison=candidate`
builds the specified immutable baseline and head with the same recorded compiler,
target and flags. Initial controls must pass before evaluating optimization claims.

Each measured command gets a fresh CPU/no-swap memory cgroup. Two baseline scans
prepare current/previous seed generations. Every arm receives a verified copy of
that seed; AB/BA order alternates. Setup receipts include seed scans and copying.
Caches are synced/dropped after copying, before the single measured command.

A separate container mounts the closed snapshot read-only and records its own
memory/I/O/time. Full-file hashes check observer immutability. SQLite quick checks,
row counts and streaming semantic hashes compare current and previous generations.
Generated snapshot IDs/timing and changing filesystem-capacity observations are normalized;
untouched timestamps, generations, IDs, hardlink records and content stay exact.
New observation timestamps are normalized relative to the seed timestamp. This is
a baseline-v1-refresh oracle, not a fresh-full-scan replacement for refresh policy.

Application peak disk includes case current/previous/output/staging/temp allocation.
It derives from dedicated-filesystem occupancy minus the static fixture/setup
baseline, adding initial case snapshot allocation. Extra growth is reported
separately. This captures open unlinked allocation; 20ms samples remain lower
bounds. The paired runner preallocates a 40 GiB filesystem (42 GiB host headroom).
Before fixture creation, bytes and inodes must satisfy documented planning
allowances, retained seed storage and reserve. During commands, exhausted reserve
aborts the operation. Final cleanup must succeed; active mounts/backing files are
preserved on teardown failure.

I/O profiles are sensitivity cases. `native` is unrestricted; `limited` retains the
historical 32/16 MiB/s, 1000/500 read/write IOPS envelope. `local` uses 32/32 MiB/s,
2200/3600 IOPS, covering much of the observed local workload burst range. Runner
fio validates enforcement. These are not measured maximum local-device rates or
proof of identical local timing. Local scan/refresh receipts are under
`docs/benchmark-results/local-v1-*2026-10-31*`.

A result is invalid if an expected arm is missing, provenance/configuration/seed
or semantic hashes differ, a writer/observer fails, OOM occurs, or cleanup fails.
`scripts/bench_v1_summary.py` writes descriptive `summary.json` and Markdown and
returns nonzero for invalid evidence. It never promotes a candidate automatically.

The initial campaign has six aggregate runner-hours, including builds, setup,
failures and concurrent jobs. This is an experiment-duration cap: standard hosted
compute for this public repository is free. The operator records each run's job
usage before another dispatch. No automatic retry or complete Cartesian sweep is
allowed. Smoke, screen and one local-scale dispatch have maximum job envelopes
of 60, 95 and 150 minutes respectively; repeated/failed runs consume that same cap.
If no remaining admissible budget exists, retain the evidence and stop.

Current limitations: changed-refresh is supported by the entry point but is not
part of the initial workflow dispatch; the 100-entry folder and 4 GiB budget are
initial scenarios. Other folder sizes, memory-budget sweeps and repeated full-scale
confirmation require explicit later campaign allocation. Neither a smoke pass nor
one full-scale pair is optimization qualification. Owner gates and future tests
are in `docs/designs/v1-performance-plan.md`.
