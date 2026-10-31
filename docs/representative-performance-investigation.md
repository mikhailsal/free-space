# Representative performance investigation

Base: master `afce6fd`. Induced benchmark loads run only on ephemeral
GitHub-hosted runners. A pre-existing scheduled local scan is observed passively;
this investigation does not launch a local scan.

## Full-scale native results: 7,202,464 entries

The native job in [run 41134701592](https://github.com/mikhailsal/free-space/actions/runs/41134701592)
completed successfully. All 16 commands pass the hardened completeness checker:
CLI, observers, terminal experiment status and arm receipts. Product source is
master `afce6fd`; measured harness is `5b62c148d14f5b3a1f947928d11ec2430b045a3c`.
[Raw JSON archive](benchmark-results/representative-local-scale-native-2026-10-31.json.gz).
The limited-I/O full-scale job remains pending. Pilot results below are separate.

| Metric | v1 | v2 |
| --- | ---: | ---: |
| Scan 1, seconds | 340.624 | 559.162 |
| Scan 2, seconds | 343.878 | 601.123 |
| Scan 3, seconds | 349.245 | 692.520 |
| Scan 4, seconds | 348.046 | 749.069 |
| Scan 5, seconds | 348.872 | 825.249 |
| 18-entry refresh median of three, seconds | 27.931 | 0.0176 |
| Refresh range, seconds | 26.497–28.929 | 0.0169–0.0302 |
| Maximum sampled filesystem allocation during CLI commands above initial arm baseline, GiB | 2.954 | 11.339 |
| Maximum sampled named-file allocation during CLI commands, GiB | 2.954 | 9.761 |
| Retained DB allocation after scan 5, GiB | 1.657 | 5.877 |
| Maximum full-scan process RSS, MiB | 23.23 | 31.61 |
| Maximum refresh process RSS, MiB | 1107.15 | 8.14 |
| Arm-cumulative cgroup memory peak, GiB | 4.000 | 4.000 |

Both arms started with 264,269,824 bytes of fixture/filesystem overhead; the
filesystem peak subtracts that initial baseline, not each command's start.
V1 retained DB allocation includes current and previous files. Peaks are sampled
lower bounds. Both arms reached a 4 GiB cgroup memory peak despite low scan process RSS.
The artifacts do not include `memory.stat` for attribution of the remaining
charges. Sampling of disk peaks excludes setup, observers and fixture deletion. This demonstrates successful operation at
that budget, not a minimum RAM requirement. The refresh target is a fixed small
unchanged subtree; this does not bound the cost of refreshing a large folder.

V2 history grows from 7,202,464 to 36,012,320 version rows over five unchanged
scans, while live rows and identities remain 7,202,464. Its reported traversal
stays between 125.7 and 134.4 seconds, while publication transaction time rises
from 417.2 to 603.1 seconds. Additional time outside those phases rises from
16.2 to 87.9 seconds; it includes close/checkpoint and reporting and is not an
isolated checkpoint measurement. V1 scan times remain between 340.6 and 349.2
seconds in this sequence. Exact SQL-level attribution still needs instrumentation.

V2 loop-device read/write traffic for scans 1–5, GiB, is respectively
9.64/9.62, 8.10/7.55, 13.08/10.36, 17.36/13.09 and 23.37/15.82. V1 stays near
3.27–3.59 GiB read and 2.23–2.59 GiB written per scan. These are per-command cgroup deltas covering the CLI and sampling helper,
counting only loop `7:0`; adding the backing device would double-count overlapping I/O layers.

Each command is followed by full snapshot integrity reads and SQLite table/index
observations outside its timing. These warm or evict cache before the next
command. The sequence is instrumented and has one repeat; it establishes neither
ordinary uninterrupted cache behavior nor run-to-run variance.

## Passive observation of the scheduled local scan

The existing systemd timer started a scan at 02:00 Moscow on October 31. This
investigation did not launch it or change its configuration. A five-second
read-only observer followed process/cgroup counters and allocated file blocks
until the process exited. [Raw samples](benchmark-results/local-passive-scheduled-scan-2026-10-31.jsonl.gz).

| Local observation | Value |
| --- | ---: |
| Published generation / entries | 6 / 7,326,981 |
| Command duration from service log | 4491.2 s (74.9 min) |
| Preparation / publication transaction | 1870.7 / 2429.8 s |
| V2 DB allocation after completion | 8.848 GiB |
| Maximum sampled named + additional open file allocation | 18.021 GiB |
| Maximum sampled named allocation alone | 15.950 GiB |
| Allocation before scan / after completion | 9.895 / 10.929 GiB |
| Additional space above initial allocation at sampled peak | 8.126 GiB |
| Maximum sampled process RSS | 45.04 MiB |
| Maximum sampled cgroup memory.current | 6.155 GiB |

The allocation inventory covers regular files directly in the snapshot root,
`v2` and `sqlite-tmp`, plus open regular-file inodes under the snapshot root
not already counted, deduplicated by device/inode. It includes 2.080 GiB of
unchanged legacy v1 snapshots. Subtracting that known residue gives approximately
15.941 GiB attributable to the remaining observed allocation at peak, but this
is still not an exhaustive inventory of arbitrary nested temporary directories.
“Additional open” does not prove an inode is unlinked: link counts were not
recorded. Individual component maxima need not coincide. Five-second samples
are lower bounds; cgroup memory includes non-RSS charges and is not minimum RAM.

The service exited successfully but published an incomplete snapshot because
three temporary directories could not be read. This is observational evidence,
not a clean benchmark pass. Entry count is 1.73% above the preceding generation
used to size the runner fixture. The installation's binary revision, filesystem
shape/fullness, history and competing activity differ from the runner; absolute
timings cannot be compared as if only disk speed changed.

Observed local interval medians were 1,022 read IOPS and 4.806 MiB/s read;
p95 was 2,034 IOPS and 19.277 MiB/s. Write p95 was 353 IOPS and 29.995 MiB/s.
The limited runner's 1,000-read-IOPS cap is close to that median, but its
32/16 MiB/s and 1,000/500 IOPS ceilings suppress real local bursts. These are
achieved workload rates, not device capability measurements; loop/physical
request accounting and writeback batching differ. The limited profile remains
a sensitivity case, not a calibrated replica of this machine.

## Local evidence, without a new scan

On 2026-10-31, the installed 0.2.0 binary reported 7,202,464 entries in its
existing snapshot, generation 5, schema 2. The service configuration uses v2,
disk staging, previous-generation retention and (in service logs) one worker.
The filesystem is ext4 on a 450 GiB VMware virtual disk. The machine has eight
logical CPUs and approximately 16 GiB RAM. Virtual-device rotational metadata
does not establish actual disk performance.
The installed binary is a 10 MiB static-PIE ELF reporting 0.2.0; its exact source
revision was not established. Runner binaries use the existing GNU release
build protocol (Rust 1.99.1), so these comparisons also do not establish binary
target equivalence to the installation. A deployment-target confirmation is
required before predicting absolute production latency.

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
jobs. Full-scale runs require the workflow dispatch `full_scale` input or a
`[local-scale]` push marker; routine harness changes run only pilots. Separate
concurrency groups keep quick validation independent of an ongoing large run. Each profile compares current v1 and v2 from that same binary, with one
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
a 300-minute job budget, reserving a nominal 20-minute window for cleanup and
artifact upload. The measured revision has older unbounded cleanup, so timeout
artifact survival is not guaranteed. A deadline failure is an incomplete experiment, not a completed latency measurement.

No product optimization is accepted before these measurements identify the
cost and a counterfactual experiment verifies the proposed cause.

## Pilot evidence (2026-10-31 Moscow time)

[Run 41134701592](https://github.com/mikhailsal/free-space/actions/runs/41134701592)
at harness revision `5b62c148d14f5b3a1f947928d11ec2430b045a3c` completed both
100,120-entry pilots. All 48 CLI/observer results passed. The product source
is unchanged from master `afce6fd`. [Archived JSON](benchmark-results/representative-pilots-2026-10-31.json.gz)
contains manifests, raw per-command metrics, table/index observations and fio.
The GitHub build ran `cargo test --workspace`: 315 passed, zero failed, one
pre-existing ignored test. Ten harness tests passed in that build. Fourteen
additional mocked calibration and cleanup tests pass locally with the original
ten. These checks do not run scanner workloads or real Docker/fio/mount operations.

The limited runner achieved 16,772,184 write bytes/s, 33,520,911 read bytes/s,
499.70 random-write IOPS and 999.90 random-read IOPS. Requested cgroup limits
and charged loop-device I/O were verified. The native pilot on another runner
achieved roughly 246/399 MB/s sequential write/read and 5285/4222 random
write/read IOPS. These are runner measurements, not local disk capacity.
The native pilot used an Intel Xeon 6973P-C, while the limited pilot used an
AMD EPYC 7763. Between-profile elapsed-time differences therefore are not a
controlled disk-only effect. Within each profile, v1/v2 arms ran sequentially
on the same VM. Device limits themselves were verified independently by fio.
As a weak sanity check only, passive local cumulative device counters gave
0.925 ms mean read completion and 1.738 ms mean write completion. These combine
all workloads since boot and do not establish bandwidth, queue-depth behavior,
or a scan-specific service-time distribution.

A passive `stat` inventory also found retained v1 snapshots alongside v2:
`snapshot.db` uses 0.958 GiB, `snapshot.prev.db` 1.123 GiB, and
`v2/generations.db` 7.814 GiB of allocated blocks. These three known files total
9.895 GiB before a new scan. This is not a measured peak or a complete inventory
of every temporary directory. The controlled runner starts each backend with
fresh storage; it does not include the local installation's 2.080 GiB legacy
v1 residue. No local snapshot was removed or changed.

Medians of two paired repeats; seconds, except memory/storage in MiB:

| Metric | Native v1 | Native v2 | Limited v1 | Limited v2 |
| --- | ---: | ---: | ---: | ---: |
| First scan | 1.575 | 3.087 | 8.839 | 16.656 |
| Second scan | 1.010 | 2.346 | 2.388 | 12.381 |
| Third scan | 1.009 | 2.486 | 2.379 | 13.534 |
| First 18-entry refresh | 0.308 | 0.028 | 1.012 | 0.012 |
| First scan RSS | 25.0 | 25.6 | 24.7 | 25.6 |
| First refresh RSS | 30.7 | 8.1 | 30.4 | 8.1 |

V2 `version_rows` counts are 100,120, 200,240, then 300,360, although the fixture
does not change. `identities` and `live_rows` remain 100,120. Version table size
grows from 6.15 to 18.36 MiB; its historical child index from 2.56 to 7.68 MiB.
This confirms per-scan historical storage growth. It does not isolate which
SQL operation causes the publication-time increase.
In the first limited v2 arm, charged device writes increase from 56.0 MiB on
scan two to 73.6 MiB on scan three; both have zero charged device reads. This
is direct write-amplification evidence on an unchanged, warm fixture, though
it still does not isolate a particular table/index operation.

For the limited pilot's third v2 scan, named files peak at 99.7 MiB, whereas
whole-filesystem use above the arm's pre-scan baseline peaks at 110.0 MiB in
both repeats. The latter includes allocated temporary/metadata space omitted
by named-file sampling. Do not label the 10.3 MiB difference entirely as one
specific SQLite temporary file without further tracing. Both are sampled
lower bounds. The same v1 comparison is approximately 41.7 MiB by both methods.

The large 7,202,464-entry jobs were admitted after these successful pilots;
their results are separate from this pilot evidence.

## Follow-up counterfactuals

The first intervention to test is existing retention maintenance between scans,
with the same current/previous retention. Compare complete scan-plus-maintenance
cycles, not just the next scan: reclamation has its own time, WAL and temporary
space costs. A 100,000-record pass counts examined records, not deletions; at
least 73 such passes are needed merely to delete 7,202,464 obsolete versions.
Protected rows and other tables increase actual work. Deletion makes SQLite
pages reusable and does not necessarily reduce the file's allocated size.
Measure freelist, live/history occupancy and subsequent growth separately.

The existing collector protects retained generations and identities still
referenced by versions. Its exclusive lease skips collection when readers are
active; new readers may receive a maintenance-lock retry error during a pass.
Automatic invocation therefore needs availability and failure semantics, even
when using unchanged retention rules. Do not invoke it while holding the CLI's
own reporting reader or report a committed scan as unpublished if later cleanup
fails (`core/src/snapshot/v2/{mod,retention}.rs`).

Then isolate destination cache pressure: compare the existing 8 MiB publication
cache against 32/64 MiB, keeping staging, durability and history depth constant.
Instrument retirement, live-table clearing, identity lookup, inserts, commit and
close separately before attributing time to a particular SQL operation. A third
experiment can compare the current insert-on-conflict-then-select identity lookup
with select-first/insert-on-miss, testing fresh seeds, stable rescans and churn.

Reusing unchanged versions is a broader representation change. New scans update
observable `scanned_at` and subtree minimum freshness even without file changes;
hardlink ownership and charged aggregates can also change. Any reuse scheme must
preserve those values, current/previous diff, refresh semantics and reader
snapshot consistency. No such product change is part of this investigation.

## Harness validation after the measured revision

The subsequent cleanup and summary changes do not change product code or timed
scanner operations. Run [41147002015](https://github.com/mikhailsal/free-space/actions/runs/41147002015)
validated bounded cleanup with 24 harness tests and both pilot profiles. Run
[41147838067](https://github.com/mikhailsal/free-space/actions/runs/41147838067)
validated the stricter summarizer and both pilot profiles. All 35 harness tests
also pass locally using tiny or mocked inputs. The stricter checker was rerun
against the native full-scale artifacts from the original measured revision.

Incomplete metrics are retained as diagnostics but rejected as a completed
experiment. Acceptance requires successful terminal status, expected commands,
completed observers, healthy final cgroup state and exited successful Docker
state. New cleanup bounds producer termination, container removal, unmount and
loop detach; older running jobs retain their original cleanup behavior.
