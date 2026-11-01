# Cache mechanism: 4 MiB versus 8 MiB — 2026-11-01

Run 41273349166 completed a valid research comparison of a 4 MiB staging-cache target against source 63's 8 MiB target: three 1m-file scan pairs and three unchanged 100-entry refresh pairs at 4 GiB, two CPUs, one worker, no swap and local I/O. The baseline binary `1ad28691243b2d2c52fee323c793d2d90323e57fc5ec52c4e80b8aba3344e35e` exactly matched the [fresh independently built A/A control](v1-cache4-mechanism-63-aa-2026-11-01.md). Candidate source `1af549a8f1d834fb292e78a6cbd48ad9d12252cd` produced binary `d7ce843b11edb96246a01b4e5fa1e3a4b7d356110ca37c2f47f0ad47dfc49879`; both builds used musl and `SOURCE_DATE_EPOCH=1793493808`.

| Operation | Pair | Wall delta (s, %) | RSS delta (KiB, %) | Total worker-cgroup peak delta (bytes, %) | Sampled app disk delta (bytes, %) |
|---|---:|---:|---:|---:|---:|
| scan | 1 | +1.3518 (+3.067%) | -12,224 (-32.723%) | -13,090,816 (-0.676%) | +0 (+0.000%) |
| scan | 2 | +0.6304 (+1.419%) | -12,376 (-32.894%) | -12,242,944 (-0.632%) | +0 (+0.000%) |
| scan | 3 | +0.3868 (+0.862%) | -12,472 (-33.188%) | -13,410,304 (-0.692%) | +0 (+0.000%) |
| refresh | 1 | +0.0035 (+0.033%) | -116 (-0.040%) | -319,488 (-0.072%) | +0 (+0.000%) |
| refresh | 2 | +0.0012 (+0.011%) | +68 (+0.024%) | -20,480 (-0.005%) | -4,096 (-0.001%) |
| refresh | 3 | +0.0562 (+0.531%) | +68 (+0.024%) | -28,672 (-0.006%) | +4,096 (+0.001%) |

The candidate's scan elapsed time increased in all three pairs, while scan RSS and total cgroup peak decreased in all three. The elapsed-time changes are much larger than the fresh A/A's maximum absolute 0.21673% scan spread; the spreads are descriptive and not confidence intervals. The approximately 12–13 MiB total-cgroup reduction is small relative to the roughly 1.8 GiB worker peak. Percentages are computed from exact decimal receipt values using rational arithmetic. This compares one cache mechanism with source 63, not the canonical e826 product baseline: it does not attribute earlier compound source gains to the cache change, establish an H2 waiver, or qualify either source. The shared cache constant also affects v2 disk staging, whose performance is unmeasured.

Both strict summaries recomputed equal to their stored summaries and all 12 observers validated 1,004,499 current and previous entries. Refresh profile-v2 structures were valid; the original HWM monotonic contract was false and the raw anomaly records are preserved. GitHub passed 326 Rust tests with one ignored and 121 Python tests. Build and paired jobs consumed 300 and 854 seconds, respectively: 1,154 aggregate runner seconds. The campaign ledger remains authoritative.

The frozen helper validated the case, and all 203 of its evidence hashes were independently checked. The archive contains complete downloaded measurements and binaries, logs, source freeze, fresh control report, validation and both dispatch receipts. The helper-input receipt was reconstructed by removing the subsequently written `measurement_validation` field from the post-decision receipt; its bytes match the helper-recorded input hash. Every archive member and checksum was verified.
