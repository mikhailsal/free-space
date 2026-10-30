# Representative performance investigation

Base: master `afce6fd`. Research runs only on ephemeral GitHub-hosted runners.

## Local evidence, without a new scan

On 2026-10-31, the installed 0.2.0 binary reported 7,202,464 entries in its
existing snapshot, generation 5, schema 2. The service configuration uses v2,
disk staging, previous-generation retention and (in service logs) one worker.
The filesystem is ext4 on a 450 GiB VMware virtual disk. The machine has eight
logical CPUs and approximately 16 GiB RAM. Virtual-device rotational metadata
does not establish actual disk performance.

Existing successful service records:

| Date | Generation | Total seconds | Preparation seconds | Publication seconds | Reported snapshot GiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Oct 27 | 1 | 2271.9 | 1093.8 | 1088.2 | 5.1 |
| Oct 28 | 3 | 2432.3 | 1148.2 | 1236.9 | 6.0 |
| Oct 29 | 4 | 2798.3 | 1096.1 | 1639.4 | 6.9 |
| Oct 30 | 5 | 3133.9 | 1088.8 | 1834.6 | 7.8 |

These are observational logs, not paired controlled trials. File content,
competing work and cache state can change. They establish the symptom, not the
causal share of history growth. Available sysstat archives were from May and
cannot calibrate the October workload. No local scan, cache flush or I/O load
test was performed for this investigation.

## Hypotheses to test

V2 full scans retire every live version and insert a complete new generation,
including unchanged files (`core/src/snapshot/v2/seed.rs`). Publication does
not collect obsolete history. Maintenance is explicit. More versions increase
storage; whether growing indexes and page churn explain growing publication
time requires controlled measurements. Do not assume the retire query scans
all history: its `dead IS NULL` predicate has a partial index.

Normal staging retains the original tree while building entries with owned
paths, sort keys and scopes. Low-memory scan trades disk staging for lower
process RSS. Refresh still stages the selected subtree in memory and includes
affected hardlink groups and ancestors. It does not rewrite all unrelated rows
as v1 does.

## Runner protocol

`Representative scanner benchmark` builds and tests one release binary, then
runs native and limited-I/O pilots. Only successful pilots admit full-scale
jobs. Each profile compares current v1 and v2 from that same binary, with one
worker, a 2-CPU quota, 4 GiB RAM, no swap and disk staging. This is a controlled
resource budget, not an assertion of CPU/RAM equivalence to the local machine.

The pilot has 100,000 unrelated files, three scans and two alternating repeats.
The full case has 7,195,248 unrelated files plus directories and a fixed
18-entry refresh target: exactly 7,202,464 total entries. It has five scans and
three complete CLI refreshes per arm. Full-scale results have one repeat and
must not be described as statistically precise. The fixture contains empty
files in shallow directories: it matches entry count, not the local path-depth,
name-length, hardlink, permission or directory-fanout distribution.

A dedicated preallocated 30 GiB ext4 image supplies 8.5 million inodes. The host must
have at least 32 GiB free before setup; backing blocks are reserved with fallocate
and formatting disables discard. Loop direct I/O limits double caching. Fixtures are generated
once per count outside measured cgroups, then mounted read-only into each arm.
Each arm starts with a verified host cache flush. Later commands retain cache,
including observer reads. No local cache is flushed.

The limited profile requests 32 MiB/s reads, 16 MiB/s writes, 1000 read IOPS and
500 write IOPS. These are **sensitivity assumptions, not measured local disk
limits**. Before any scanner measurement, direct sequential and random fio
probes record achieved rates and latencies. The profile fails if configured
`io.max` is absent, observed rates exceed limits by more than 25%, or the target
device has no charged I/O. Each measured command also verifies `io.max` and
captures `io.stat`. An unsupported loop throttling configuration must fail,
never silently produce a nominally limited result.

Compare observed scan phases with the historical local ranges above. Similar
wall time alone does not prove disk equivalence: metadata layout, caching,
filesystem fullness and contention still differ. Exact disk calibration would
require additional trustworthy local I/O evidence.

## Four metrics and validity

- Full command and refresh wall time include process start and final close.
- GNU time RSS is per process; cgroup peak includes file cache, helper and all
  preceding commands/observations in that arm. Report both, never equate them.
- Sample named snapshot files every 20 ms: DB, WAL, SHM, staging and other.
  SQLite temporary files are directed to the measured device. Path sampling
  misses already-unlinked temporary files.
- Also sample whole-filesystem allocated use with `statvfs`: starting, ending,
  peak and incremental peak. On this dedicated filesystem it includes unlinked
  temporary allocations but also the fixed fixture and filesystem metadata.
  Both peak methods are sampled lower bounds, not strict maximum guarantees.
- Retain raw reports, exit status, OOM counters, device configuration, SHA-256
  provenance, table counts and index sizes. Failed or incomplete commands are
  diagnostics, never successful timing samples.

The observer has a separate bounded SQL budget and missing table/index evidence
invalidates the command. Large jobs have a 280-minute measurement deadline within
a 300-minute job budget, leaving time to upload partial diagnostics. A deadline
failure is an incomplete experiment, not a completed latency measurement.

No product optimization is accepted before these measurements identify the
cost and a counterfactual experiment verifies the proposed cause.
