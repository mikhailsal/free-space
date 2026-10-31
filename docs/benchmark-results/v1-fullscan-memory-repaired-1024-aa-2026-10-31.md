# Repaired-source 1 GiB full-scan A/A control

The [GitHub run](https://github.com/mikhailsal/free-space/actions/runs/41244336962) completed three scan pairs on the 1,000,000-file fixture with musl, the local I/O envelope and a **1,024 MiB worker cgroup cap**. Strict recomputation passed and exactly matched the raw summary. Both arms recorded binary SHA-256 `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`; the repaired harness source was `55d6e3460f34710e2520b77da66e659d410f6c25`. This separately registered successful run does not change the earlier failed case's invalid status.

All six worker cgroups recorded a 1,073,741,824-byte limit, zero swap and a `memory.peak` equal to that limit. Hierarchical `memory.events` recorded 990–1,758 `max` events, with zero `oom` and `oom_kill`; `memory.events.local` was not collected. Recorded PSI `some` and `full` totals ranged from 349,527 to 504,159 microseconds. The successful commands therefore encountered measurable memory pressure. Endpoint charges ranged around 8.55 MB anonymous, 213–307 MB file and 652–742 MB kernel; reclaimable slab, about 651–741 MB, is included in kernel. Cgroup usage includes cache and kernel memory and differs from process RSS.

Setup scans and all six separately metered observers used 4,096 MiB limits; observers had zero swap and validated 1,004,499 current and previous entries plus semantic and integrity checks. Profile-v2 allocation diagnostics apply to refresh only and were disabled for this scan; there is no refresh-profile or HWM anomaly verdict here.

| Identical-binary paired metric | Observed changes |
| --- | --- |
| Scan elapsed time | −1.312%, +1.401%, +3.663%; largest absolute difference 2.141 s |
| Process peak RSS | +7.666%, −0.203%, +9.976%; largest absolute difference 3,368 KiB |
| Sampled app disk peak | −4,096, −4,096, −16,384 bytes |

These are descriptive matched controls from three pairs, not a p95 estimate or a source-code effect. Runner preparation was skipped with 91,856,969,728 bytes available both before and after and no SDK actions. Build and paired jobs used 179 and 645 runner-seconds; 307 Rust tests passed, zero failed and one was ignored, and 111 Python tests passed.

The run demonstrates that these six commands succeeded at the tested cap despite pressure. It does not establish the minimum required RAM or a source-code memory gain. The [receipt](v1-fullscan-memory-repaired-1024-aa-2026-10-31.json), [raw artifacts and supporting evidence](v1-fullscan-memory-repaired-1024-aa-2026-10-31.tar.gz), and [hash index](v1-fullscan-memory-repaired-1024-aa-2026-10-31.sha256) preserve the run, independent validation, dispatch receipt and GitHub log.
