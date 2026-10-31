# Repaired-source joint 1 GiB full-scan screen

The [GitHub run](https://github.com/mikhailsal/free-space/actions/runs/41245753697) completed three scan pairs on the 1,000,000-file fixture with musl, the local I/O envelope and a **1,024 MiB worker cgroup cap**. Strict recomputation passed and exactly matched the raw summary. The baseline binary SHA-256 was `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`; the candidate was `f0c25acd428e150a0ad5e1139f5ef7b14cf6404dacd1cd55e0d03cec166a6e74`, from source `0adc796a99fd33abec85ab250ad9b2529507b49a`.

All six workers recorded a 1,073,741,824-byte limit, zero swap and `memory.peak` equal to the limit. Hierarchical `memory.events` recorded 1,726–1,813 `max` events and zero `oom`/`oom_kill`; `memory.events.local` was not collected. Recorded PSI `some` and `full` totals ranged from 426,675 to 447,262 microseconds. Endpoint charges were approximately 8.56 MB anonymous, 209–221 MB file and 695–744 MB kernel; reclaimable slab, about 695–742 MB, is included in kernel. These cgroup charges include cache and kernel memory and differ from process RSS.

Setup scans and all six separately metered observers used 4,096 MiB limits; observers had zero swap and validated 1,004,499 current and previous entries plus semantic and integrity checks. Profile-v2 allocation diagnostics apply to refresh only and were disabled for this scan; there is no refresh-profile or HWM anomaly verdict here.

| Metric | Candidate paired changes | Repaired 1 GiB A/A reference |
| --- | --- | --- |
| Scan elapsed time | −25.443%, −29.124%, −26.372%; median gain 26.372% | Largest absolute paired spread 3.663% |
| Process peak RSS | −0.075%, +0.648%, −0.814%; median −0.075% | Largest absolute paired spread 9.976% (3,368 KiB) |
| Sampled app disk peak | 0, +12,288, +12,288 bytes; median +12,288 | Largest absolute paired spread 16,384 bytes |

The paired scan-time improvements are consistent here. **H2 disk no-regression remains inconclusive:** the candidate median is positive even though its magnitude is within the A/A's largest observed absolute spread. The A/A is a descriptive reference from a different VM, not a confidence interval or a waiver. This screen does not promote the candidate. Success under a lower cap is not itself a source-code RAM saving or proof of a minimum requirement.

Runner preparation was skipped: 91,856,842,752 bytes were available both before and after, with no SDK actions. Build and paired jobs used 279 and 768 runner-seconds; 320 Rust tests passed, zero failed and one was ignored, and 111 Python tests passed. The [receipt](v1-fullscan-memory-repaired-1024-joint-2026-10-31.json), [raw artifacts and supporting evidence](v1-fullscan-memory-repaired-1024-joint-2026-10-31.tar.gz), and [hash index](v1-fullscan-memory-repaired-1024-joint-2026-10-31.sha256) preserve the measurements, independent validation, dispatch receipt and GitHub log.
