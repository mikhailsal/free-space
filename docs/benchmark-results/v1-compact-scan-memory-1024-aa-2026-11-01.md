# Compact scan memory: 1 GiB A/A control

[Run 41256220919](https://github.com/mikhailsal/free-space/actions/runs/41256220919) passed independent strict recomputation and all six scans and observers on 1,000,000 files / 1,004,499 entries. Workflow comparison was `candidate`; scientifically this is A/A: baseline `e8265778` and control `4f22bf8` produced identical downloaded binaries with SHA-256 `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`. Each observer validated 1,004,499 current and previous entries and the same logical digest.

| Arm | Scan time (s) | Peak RSS (KiB) | Worker cgroup peak (bytes) | Sampled app disk peak (bytes) |
| --- | ---: | ---: | ---: | ---: |
| p1-baseline-v1 | 79.367 | 37,408 | 1,073,741,824 | 459,837,440 |
| p1-candidate-v1 | 78.527 | 36,564 | 1,073,741,824 | 459,829,248 |
| p2-baseline-v1 | 79.740 | 37,132 | 1,073,741,824 | 459,841,536 |
| p2-candidate-v1 | 78.022 | 37,328 | 1,073,741,824 | 459,853,824 |
| p3-baseline-v1 | 85.844 | 35,508 | 1,073,741,824 | 459,837,440 |
| p3-candidate-v1 | 71.809 | 37,344 | 1,073,741,824 | 459,849,728 |

Paired time changes were -1.059%, -2.154% and -16.350%; they are descriptive A/A variability, not confidence intervals. All worker cgroups reached the 1 GiB cap, with hierarchical `memory.events` max counts 1,570–1,708, memory pressure and no recorded OOM, OOM-kill or swap. Endpoint `memory.stat` fields in the JSON and raw receipts do not decompose the earlier peak. `memory.events.local` was not collected.

This establishes successful baseline/control operation at the tested cap under reclaim; it does not establish an exact minimum, new candidate capacity, a code RAM gain or full-scale behavior. Profile-v2 is not applicable to these scan-only runs; allocation diagnostics were disabled for workers and seeds. The observer and setup cap was 4 GiB. Runner preparation succeeded without SDK deletion and cleanup succeeded.

GitHub ran 307 Rust tests with 1 ignored and 121 Python tests; build and paired jobs took 151 and 822 seconds. The archive includes original measurement and binary artifacts, GitHub logs/metadata, admission receipt, frozen source and helper/forecast context.
