# Repaired-source 512 MiB full-scan A/A control

The [GitHub run](https://github.com/mikhailsal/free-space/actions/runs/41248061322) completed three scan pairs on the 1,000,000-file fixture with musl, the local I/O envelope and a **512 MiB worker cgroup cap**. Strict recomputation passed and exactly matched the raw summary. Both arms recorded binary SHA-256 `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`; the repaired harness source was `55d6e3460f34710e2520b77da66e659d410f6c25`.

All six workers recorded a 536,870,912-byte memory limit, zero swap and a `memory.peak` equal to the limit. Hierarchical `memory.events` recorded 2,100–2,511 `max` events and zero `oom`/`oom_kill`; `memory.events.local` was not collected. Recorded PSI `some` and `full` totals ranged from 694,577 to 848,266 microseconds. The commands succeeded with measurable memory pressure. Endpoint charges were approximately 8.56 MB anonymous, 128–225 MB file and 33–299 MB kernel; reclaimable slab, about 33–298 MB, is included in kernel. Cgroup charges include cache and kernel memory and differ from process RSS.

Setup scans and all six separately metered observers used 4,096 MiB limits; observers had zero swap and validated 1,004,499 current and previous entries plus semantic and integrity checks. Profile-v2 allocation diagnostics apply to refresh only and were disabled for this scan; there is no refresh-profile or HWM anomaly verdict here.

| Identical-binary paired metric | Observed changes |
| --- | --- |
| Scan elapsed time | +1.516%, −3.062%, −0.269%; largest absolute difference 2.597 s |
| Process peak RSS | −3.484%, +0.319%, −2.371%; largest absolute difference 1,228 KiB |
| Sampled app disk peak | −12,288, +20,480, −36,864 bytes |

These three pairs are descriptive controls, not a p95 estimate or a source-code effect. Preparation was skipped with 91,857,059,840 bytes available both before and after and no SDK actions. Build and paired jobs used 176 and 908 runner-seconds; 307 Rust tests passed, zero failed and one was ignored, and 111 Python tests passed.

This demonstrates six successful commands at the tested cap and fixture. As the lowest tested rung, the result is floor-censored: it does not establish a minimum RAM requirement or a source-code memory gain. The [receipt](v1-fullscan-memory-repaired-512-aa-2026-10-31.json), [raw artifacts and supporting evidence](v1-fullscan-memory-repaired-512-aa-2026-10-31.tar.gz), and [hash index](v1-fullscan-memory-repaired-512-aa-2026-10-31.sha256) preserve the measurements, validation, dispatch receipt and GitHub log.
