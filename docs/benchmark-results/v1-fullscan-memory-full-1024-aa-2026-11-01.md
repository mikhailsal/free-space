# Full-scale 1 GiB scan A/A control — 2026-11-01

[Run 41252744694](https://github.com/mikhailsal/free-space/actions/runs/41252744694) completed one paired staged-v1 scan on 7,264,383 files / 7,297,076 entries, using musl, local I/O, two CPUs and one worker. Both measured workers ran without swap under a 1,024 MiB cgroup cap. Strict protocol-v4 recomputation was valid and exactly matched the original summary. This is **one exploratory A/A pair**, not D11 confirmation, a source RAM gain, or a minimum-memory determination.

Baseline source `e8265778ef78ec4acd9da1375eedb0be37a123f5` and control harness source `e0299a6385e27fef4f78b682e7a5d25b969a02a1` both produced the identical actual binary SHA-256 `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`, verified against both build metadata and downloaded binary bytes. The archive retains both binaries and their metadata. The timed CLI used staged low-memory scan; this does not characterize ordinary in-memory mode.

| Measurement | Baseline arm | Control arm | Paired difference |
| --- | ---: | ---: | ---: |
| Elapsed | 1,116.836 s | 1,303.922 s | +187.085 s (+16.751%) |
| Process peak RSS | 34,992 KiB | 35,000 KiB | +8 KiB |
| Sampled application disk peak | 3,356,524,544 B | 3,356,532,736 B | +8,192 B |
| Worker cgroup peak | 1,073,741,824 B | 1,073,741,824 B | Both reached the cap |

Both commands exited successfully with no OOM or OOM-kill events. Hierarchical `memory.events max` counts were 26,349 and 26,554; local events were not collected. Memory PSI `some` totals were 6,449,131 and 7,029,968 μs; `full` totals were 6,448,934 and 7,029,929 μs. These pressure counts are not a peak decomposition. At the post-command endpoints, baseline/control respectively had 8,564,736/8,564,736 B anonymous, 214,904,832/183,922,688 B file, 377,028,608/495,456,256 B kernel, and 376,645,456/495,065,312 B reclaimable slab (included in kernel). Endpoint values are not simultaneous peak components. Cgroup memory includes charged cache and kernel memory and is distinct from the CLI's process RSS.

Both observer receipts validated current and previous snapshots at 7,297,076 entries, semantic digest `b06f1a0eaaf63ba102876ffce5595bae1f5ca5ea3dd80f378ad8e3e83781dee8`, unchanged logical content and integrity checks. Both seed scans and observers used separate 4 GiB no-swap limits; measured observer cgroup peaks were 1,683,378,176 and 1,714,012,160 B. They are not included in the worker peak. Scan profile-v2 diagnostics do not apply. Success at this 1 GiB limit demonstrates only this tested case; it does not establish a minimum or imply source-code savings.

Runner disk preparation succeeded without deletion, with 91,856,920,576 B available before and after. The preparation removal path was not exercised. The measured step took 4,129 s against its 7,200 s cap; the whole paired job took 4,136 s against 7,800 s, and the build took 258 s against 1,500 s. The preregistered whole-step forecast was 4,984 s, 6,479.2 s with margin; maximum aggregate reservation was 9,300 s. Build logs report 112 Python and 307 Rust tests passed, with zero Rust failures and one ignored.

The [receipt](v1-fullscan-memory-full-1024-aa-2026-11-01.json) preserves the strict result, descriptive values, metadata, dispatch and forecast. The [archive](v1-fullscan-memory-full-1024-aa-2026-11-01.tar.gz) contains 81 raw and supporting payloads, including both binary artifacts, plus `SHA256SUMS`. Each payload was verified against its original before this report. The [external index](v1-fullscan-memory-full-1024-aa-2026-11-01.sha256) lists every payload and the archive itself.
