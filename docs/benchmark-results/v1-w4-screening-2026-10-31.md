# W4 screening: bounded child-query batches

**Not promoted.** Batching reduced the complete scan time by a paired median
7.7104%, below the owner's fixed 15% speed-only floor. Process RSS increased and
there was no useful RAM/disk benefit. The two jobs authorized by D18 are complete;
no full-scale expansion or further W4 run is planned.

The experiment used 1,000,000 files / 1,004,499 entries and an unchanged
100-entry refresh folder, three pairs per operation. Both arms used one VM per
job, cold caches after verified seed copying, two retained seed generations,
one worker, two CPU quota units, 4 GiB memory and no swap. The `local` sensitivity
I/O envelope was unchanged. This is screening, not 7.3m-entry qualification.

## Results

Medians of scan CLI times were 54.7865 s baseline and 50.5797 s candidate.
The paired speed gains were 7.7104%, 7.6785% and 7.7341%; their median is used for
the gate. Ratios of separate medians are not substituted for paired changes.

| Metric | Changes in the three pairs | Outcome |
| --- | --- | --- |
| Full-scan CLI time | -7.7104%, -7.6785%, -7.7341% | Consistent gain, below 15% floor |
| Scan process peak RSS | +248, +36, +372 KiB; paired median +0.902% | No memory benefit; positive in every pair |
| Scan sampled app disk peak | +4,096, 0, 0 bytes | No saving; one extra sampled filesystem block |
| Small-folder refresh CLI time | +14.06, +19.18, +14.34 ms; paired median +0.1394% | Within observed control spread, but positive median does not establish no regression under H2 |
| Refresh process peak RSS | -268, -276, +36 KiB; paired median -0.1064% | Far below the 25% RAM-benefit floor |
| Refresh sampled app disk peak | 0, 0, 0 bytes | No saving |

The scan's median sampled app peak was about 0.42823 GiB in both arms; refresh
was about 0.35035 GiB. Sampled peaks are lower bounds. The single 4 KiB difference
does not establish a meaningful product disk regression, and zero observed
control spread does not prove exact parity or zero sampling uncertainty.

Median cgroup peaks were approximately 1.79878 GiB for scan and 0.3776/0.3778 GiB
for baseline/candidate refresh. These include charged cache/kernel memory.
Process RSS was around 27 MiB for scan and 246 MiB for refresh. Neither RSS nor
one successful 4 GiB trial establishes the minimum required memory budget.

The scan RSS median is positive, and two of three RSS deltas exceed the observed
control maximum of 188 KiB / 0.683%. The conservative no-regression gate is not
established. Refresh's tiny positive time median is inside the control's observed
0.777% maximum, so it is inconclusive under H2, not a confirmed slowdown. No
exception is needed: the independent speed floor already fails decisively.

## Control and provenance

[Control job](https://github.com/mikhailsal/free-space/actions/runs/41206306054):
source a48ba0d, product-identical to d8b6046, complete three scan and three refresh
pairs. [Candidate job](https://github.com/mikhailsal/free-space/actions/runs/41208041797):
baseline d8b6046 versus candidate147fa29. Both use protocol v3 with the stronger
streaming observer. Baseline binary SHA-256 is exactly
`8fd2da780c44c740373ee542d9a077ba35aa149b663add65a7dbc1be0ab49773`.

Control CPU was AMD EPYC 7763; candidate-job CPU was Intel Xeon Platinum 8370C.
The control's baseline median was 62.2836 s, while the candidate-job baseline
median was 54.7865 s. Those are different VMs: comparing 62.28 s directly with
50.58 s would falsely inflate the claimed gain. The 7.71% result compares only
same-job pairs. Control bands remain empirical references with a cross-CPU
limitation, not confidence bounds or proof of a disk-only causal comparison.

All 12 candidate commands and their read-only observers passed. Expected current
and previous counts, semantic/seed hashes, provenance, cache-flush/container states,
admission and terminal cleanup receipts match; no command/observer OOM or swap.
An independent internal receipt check found no concrete defect.

The GitHub build passed 65 Python benchmark-contract tests and 322 Rust tests,
with zero failures and one existing ignored test. The tests executed the
four-versus-76 child-query assertion, slot/byte limits, tie overflow, ancestor
eviction and the 411-entry deep/wide differential fixture. This verifies the
mechanism and parity, not a 15% complete-command benefit.

## Accounting and next investigation

D18 used 1,248 + 1,249 = 2,497 aggregate runner-seconds. The whole initial campaign
has used **10,615 seconds (2 h 56 min 55 s)** of its 21,600-second cap, leaving
**10,985 seconds (3 h 3 min 5 s)**. No job remains active. Unused time does not
authorize another scope automatically.

Evidence: [control JSON](v1-w4-control-2026-10-31.json),
[control original receipts](v1-w4-control-2026-10-31.tar.gz),
[candidate JSON and gate assessment](v1-w4-candidate-2026-10-31.json),
[candidate original receipts](v1-w4-candidate-2026-10-31.tar.gz),
[preregistered scope](v1-w4-screening-manifest-2026-10-31.json), and
[campaign ledger](v1-w1-campaign-2026-10-31.json). Each archive has an indexed hash
receipt; archive hashes are recorded in the result JSONs. Databases/binaries are
excluded from the durable archives.

Both SQL experiments remain unqualified on their experiment branches. The next
useful investigation is the large v1 refresh RAM peak: the
[allocation note](../designs/v1-refresh-allocation-investigation.md) describes a
testable vector-growth hypothesis and its uncertainty. A diagnostic source
variant is being prepared separately; it has no runner allocation under D18.
This does not replace the approved W5 staged-refresh experiment or claim that
reserving capacity fixes the peak.
