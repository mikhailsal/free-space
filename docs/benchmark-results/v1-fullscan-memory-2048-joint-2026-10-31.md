# Joint candidate full-scan 2 GiB screen

The [GitHub run](https://github.com/mikhailsal/free-space/actions/runs/41241552473) completed three scan pairs on the 1,000,000-file fixture with musl, the local I/O envelope and a **2,048 MiB worker cgroup cap**. Strict recomputation passed and exactly matched the raw summary. The baseline binary SHA-256 was `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`; the candidate was `f0c25acd428e150a0ad5e1139f5ef7b14cf6404dacd1cd55e0d03cec166a6e74`, from source `4591856f7e1f66a00a8ef70874fb1046fa8c428d`.

All six worker cgroups recorded a 2,147,483,648-byte limit and zero swap. Peaks ranged from 1,942,896,640 to 1,944,719,360 bytes (about 1.81 GiB). Recorded hierarchical `memory.events` showed zero `max`, `oom` and `oom_kill`; the collector did not record `memory.events.local`. Recorded memory PSI `some` and `full` totals were zero. Endpoint charges were approximately 8.6 MB anonymous, 463 MB file and 1,359 MB kernel; the approximately 1,359 MB reclaimable slab is included in the kernel figure. These cgroup charges are distinct from process RSS.

Both setup scans and all six separately metered observers used 4,096 MiB limits; observers had zero swap and validated 1,004,499 current and previous entries plus semantic and integrity checks. Profile-v2 allocation diagnostics apply to refresh only and were disabled for this scan; there is no refresh-profile or HWM anomaly verdict here.

| Metric | Candidate paired changes | Matched 2 GiB A/A reference |
| --- | --- | --- |
| Scan elapsed time | −19.765%, −20.451%, −19.320%; median gain 19.765% | Largest absolute paired spread 2.034% |
| Process peak RSS | +0.638%, −0.095%, +0.415%; median +0.415% | Largest absolute paired spread 0.212% |
| Sampled app disk peak | −36,864, −12,288, −16,384 bytes; median −16,384 | Largest absolute paired spread 12,288 bytes |

The scan time improvement is consistent in these pairs, but **H2 RSS no-regression is not established**: the candidate median is positive and exceeds the matched control's observed maximum spread. The control band is descriptive, not a confidence interval. This screen does not promote the candidate. Lowering the cap is not itself evidence of a source-code RAM gain or a minimum successful budget.

Runner preparation was skipped: 91,856,998,400 bytes were available both before and after, with no SDK actions. Build and paired jobs used 167 and 656 runner-seconds; 320 Rust tests passed, zero failed and one was ignored, and 110 Python tests passed. The [receipt](v1-fullscan-memory-2048-joint-2026-10-31.json), [raw artifacts and supporting evidence](v1-fullscan-memory-2048-joint-2026-10-31.tar.gz), and [hash index](v1-fullscan-memory-2048-joint-2026-10-31.sha256) preserve the measurements, independent validation, dispatch receipt and GitHub log.
