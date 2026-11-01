# Cache mechanism: 63-source A/A control — 2026-11-01

Run 41271696304 completed a valid same-source control: three 1m-file scan pairs and three unchanged 100-entry refresh pairs at 4 GiB, two CPUs, one worker, no swap and local I/O. Both binaries were independently built from source `63d98cc8ff88a54ab9687c5fb93fffd930e33d26` with musl and `SOURCE_DATE_EPOCH=1793493808`; both actual SHA-256 values were `1ad28691243b2d2c52fee323c793d2d90323e57fc5ec52c4e80b8aba3344e35e`. The frozen helper validated the case and all 203 evidence hashes were independently checked.

| Operation | Pair | Candidate − baseline wall (ms) | RSS (KiB) | Total worker-cgroup peak (bytes) | Sampled app disk (bytes) |
|---|---:|---:|---:|---:|---:|
| scan | 1 | +51.178 | -64 | +348,160 | +0 |
| scan | 2 | -96.544 | +0 | +323,584 | +0 |
| scan | 3 | +101.344 | +36 | -151,552 | +0 |
| refresh | 1 | -75.340 | +88 | +7,340,032 | +0 |
| refresh | 2 | -20.454 | -124 | -495,616 | +0 |
| refresh | 3 | +14.107 | -76 | -135,168 | +0 |

Both strict summaries recomputed equal to their stored summaries. All 12 independent observers validated 1,004,499 current and previous entries. Refresh profile-v2 structures were valid; the original HWM monotonic contract was false for all six arms and raw anomaly records remain preserved. Maximum absolute paired elapsed differences were approximately 0.21673% for scan and 0.71219% for refresh. These are descriptive spreads, not confidence intervals or evidence of an improvement.

GitHub passed 326 Rust tests with one ignored and 121 Python tests. Build and paired jobs consumed 295 and 1,036 seconds, respectively: 1,331 aggregate runner seconds. Completed campaign usage after this run was 46,679 seconds (12h 57m 59s) against the 60-hour cap; the ledger is authoritative. Source 63 remains unqualified, and no cache-4 candidate performance result is claimed.

The archive contains the complete downloaded measurement and binary artifacts, GitHub logs and metadata, frozen source and preregistration evidence, independent validation, helper case record and both dispatch receipts. The helper-input receipt was reconstructed by removing the subsequently written `measurement_validation` field from the post-decision receipt; its bytes independently match the helper-recorded input hash. Every archive member and checksum was verified.
