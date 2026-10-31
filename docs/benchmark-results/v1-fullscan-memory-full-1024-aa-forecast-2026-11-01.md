# Proposed full-scale 1 GiB A/A scan forecast

**Planning estimate: 4,984 seconds for the whole measured step; 6,479.2 seconds with 30% margin, below the 7,200-second step cap.** This is a cap-fit proposal, not an admission or dispatch. Before proceeding, independently review the assumptions, confirm the frozen case and remote heads, and verify the 9,300-second maximum aggregate job reservation fits the remaining campaign budget. This A/A capacity experiment is independent of candidate promotion and does not waive H2.

Case ID: `x86_64-unknown-linux-musl|local|scan|100|unchanged|1024`. Compare control harness `55d6e3460f34710e2520b77da66e659d410f6c25` with baseline `e8265778ef78ec4acd9da1375eedb0be37a123f5`; both expected binaries are SHA-256 `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`, to be verified from actual build receipts. The case has 7,264,383 files, 7,297,076 entries, one scan pair, two CPUs, one worker, no swap and a 1,024 MiB measured-worker cgroup; setup scans and observers use separate 4 GiB cgroups.

The source is repaired 1 GiB A/A run 41244336962: its valid 1m fixture used 1,000,000 files and 1,004,499 entries, three pairs, and the expected identical binaries. Worst of its six worker times was 64.237 s, worst observer 17.007 s, worst copy 0.593 s; seeds took 25.352 and 26.203 s and fixture setup 36.663 s. File scale is 7.264383; entry scale is approximately 7.264394. For full-scale one pair, the worker and observer multipliers are **two**, rather than the six arms in the original screen. Fixture scales by files; seeds, worker, observer and copy scale by entries. Per-arm maxima are used for both arms to avoid relying on pair order.

| Whole-step component | Planning seconds | Calculation and cross-check |
| --- | ---: | --- |
| Fixture | 459 | Max of 1m setup × file scale and older full observed 458.001 s, rounded up |
| Two seed scans | 701 | Max of 2 × worst 1m seed × entry scale and older full seed sum 700.673 s, rounded up |
| Two measured workers | 1,280 | Max of 2 × worst 1m worker × entry scale and 2 × older full slower arm 639.785 s, rounded up |
| Two observers | 430 | Max of 2 × worst 1m observer × entry scale and 2 × older full slower observer 214.959 s, rounded up |
| Two copies | 14 | Max of 2 × worst 1m copy × entry scale and 2 × older full slower copy 6.990 s, rounded up |
| Calibration, setup and cleanup | 600 | Explicit fixed allowance for orchestration, loop setup and teardown |
| Lower-cap/full-scale uncertainty | 1,500 | Explicit allowance for nonlinear reclaim, I/O and execution variability |
| **Total** | **4,984** | Sum; 30% margin gives **6,479.2** |

The full-scale cross-check is older run 41230966330 at 4 GiB, on another runner with different binary hashes and `d1c9` candidate source. It is used only to avoid understating component allowances, not as a measurement of this 1 GiB case or a claim about the joint candidate. Its whole paired job took 2,829 s, including setup and observers; the proposed 7,800-second paired-job cap includes 600 seconds beyond the measured step, and the build cap is 1,500 seconds. Together those caps reserve 9,300 runner-seconds. The two capped workers each additionally have an 1,800-second command timeout.

The uncertainty allowance is a planning judgment, not an empirically established upper bound. Full-scale file cache, kernel slab and reclaim can behave nonlinearly at 1 GiB; timeout, OOM, failed semantics, incomplete receipts or capacity failure make the case invalid rather than demonstrating a boundary or qualifying for a waiver. Separately metered setup and observer limits do not describe whole-host memory. A 512 MiB full-scale follow-up is conditional on a valid completed 1 GiB full case, a fresh case-matched forecast, and separate budget admission. Raw values, source provenance, parent inspection and SHA-256 hashes for every input used are in the [forecast record](v1-fullscan-memory-full-1024-aa-forecast-2026-11-01.json).
