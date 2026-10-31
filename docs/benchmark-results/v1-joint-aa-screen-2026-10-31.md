# Matched 4 GiB A/A control screen

The [GitHub control](https://github.com/mikhailsal/free-space/actions/runs/41237970872) completed three scan and three refresh pairs on the 1,000,000-file fixture, using musl, the local I/O envelope and a 4 GiB worker limit. Both arms recorded binary SHA-256 `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`; the harness source was `e0669801ef5dc0fb8cf3bef89f72aaec06489bf7`. Independent strict recomputation passed and exactly matched both raw summaries. Profile-v2 structure was valid; the original HWM monotonic contract was false, and the raw anomalies are preserved.

| Operation | Largest absolute paired time difference | Largest absolute RSS difference | Largest absolute sampled app disk difference |
| --- | ---: | ---: | ---: |
| Scan | 0.694% (0.445 s) | 340 KiB | 0 bytes |
| Refresh | 9.522% (0.969 s) | 236 KiB | 0 bytes |

The 9.52% refresh difference occurred in one identical-binary pair. It is control variability in this run, not a measured code regression or a confidence bound. Scan cgroup peaks ranged from 1,940,656,128 to 1,941,217,280 bytes, about 1.81 GiB, under a **4 GiB cap**. Cgroup usage includes charged cache and kernel memory; it is distinct from process RSS and does not establish the minimum successful memory budget.

Runner preparation recorded 91,857,149,952 bytes available both before and after, with no actions and `acted: false`. The SDK deletion path was not exercised. Build and paired jobs used 163 and 1,112 runner-seconds; 307 Rust tests passed, zero failed, one was ignored, and 110 Python tests passed. This is a control screen, not candidate promotion or a product performance claim.

The [receipt](v1-joint-aa-screen-2026-10-31.json), [raw artifacts and supporting evidence](v1-joint-aa-screen-2026-10-31.tar.gz), and [file hash index](v1-joint-aa-screen-2026-10-31.sha256) preserve both operations, the independently recomputed summaries, profile, description, GitHub metadata and log.
