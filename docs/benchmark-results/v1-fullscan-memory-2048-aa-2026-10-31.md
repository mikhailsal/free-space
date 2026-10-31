# Full-scan 2 GiB A/A control

The [GitHub run](https://github.com/mikhailsal/free-space/actions/runs/41240117136) completed three scan pairs on the 1,000,000-file fixture with the musl binary, local I/O envelope and a **2,048 MiB worker cgroup cap**. Strict recomputation passed and exactly matched the raw summary. Both arms recorded binary SHA-256 `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980` from the product baseline, with harness source `e0669801ef5dc0fb8cf3bef89f72aaec06489bf7`.

All six worker cgroups recorded a 2,147,483,648-byte memory limit and zero swap. Their peaks ranged from 1,942,683,648 to 1,943,085,056 bytes (about 1.81 GiB); recorded `memory.events` showed zero `max`, `oom` and `oom_kill`, and recorded memory PSI `some` and `full` totals were zero. Endpoint charges were approximately 8.6 MB anonymous, 463 MB file and 1,359 MB kernel; the approximately 1,359 MB reclaimable slab is included in the kernel figure. These are cgroup charges, including cache and kernel memory, separate from process RSS.

The 4,096 MiB setup scans and each separately metered 4,096 MiB observer had zero swap. Every observer validated 1,004,499 current and previous entries and passed the semantic and integrity checks. Profile-v2 allocation diagnostics apply to refresh only and were disabled for this scan, so there is no refresh profile or HWM anomaly verdict for this run.

| Identical-binary pair metric | Largest absolute paired difference |
| --- | ---: |
| Scan elapsed time | 2.034% (1.339 s) |
| Process peak RSS | 0.212% (80 KiB) |
| Sampled app disk peak | 12,288 bytes |

The positive sampled disk difference was 12,288 bytes in each pair; it is an observed A/A spread, not a candidate effect. Runner preparation was skipped: 91,856,879,616 bytes were available both before and after, with no SDK actions. Build and paired jobs used 168 and 808 runner-seconds; 307 Rust tests passed, zero failed and one was ignored, and 110 Python tests passed.

This establishes successful operation for these six commands at the tested cap and fixture. It does not establish a minimum RAM requirement or a source-code memory improvement. The [receipt](v1-fullscan-memory-2048-aa-2026-10-31.json), [original artifacts and supporting evidence](v1-fullscan-memory-2048-aa-2026-10-31.tar.gz), and [file hash index](v1-fullscan-memory-2048-aa-2026-10-31.sha256) preserve the raw metrics, independent validation, dispatch receipt and GitHub log.
