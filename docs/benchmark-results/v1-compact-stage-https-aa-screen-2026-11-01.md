# Compact-stage HTTPS matched A/A screen

Run 41253374855 completed successfully on control source `4f22bf8e3b67f7f59049a72116b90c061fb1328b` against baseline `e8265778ef78ec4acd9da1375eedb0be37a123f5`. All 12 measured commands used the same binary SHA-256 `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`. Strict recomputation matched both raw summaries, and all 12 independent observers validated 1,004,499 current and previous entries. Each operation used three pairs on one million files; refresh targeted an unchanged 100-entry subtree.

Across the A/A pairs, maximum absolute differences were 0.3312% scan time, 192 KiB scan RSS and zero scan disk bytes; refresh differences were 0.3318% time, 80 KiB RSS and 4,096 disk bytes. These describe this control's variability and do not establish a code regression or a confidence interval. The complete paired metrics are in the JSON and archive.

Full-scan worker cgroup peaks were 1,943,457,792–1,944,035,328 bytes under a 4 GiB cap, distinct from process RSS. All 12 workers had no swap and no hierarchical max or OOM events; full-scan PSI and endpoint memory statistics are preserved in the receipt. Refresh profile-v2 structural validation passed, while the original HWM monotonic contract was false; raw traces and anomalies remain preserved. Setup and observer budgets were 4 GiB.

Runner preparation succeeded with 91856949248 available bytes and no deletion actions; low-space cleanup remains untested. The build and paired jobs used 284 and 1,256 seconds. GitHub reported 307 Rust tests passed, one ignored, and 121 Python tests passed before measurement.

The archive contains 257 original artifact files and supporting receipts, validation, log and exact workflow; all 265 indexed payloads were verified. Archive SHA-256: `41be382e4d3f511ae85bc284c1e71a735b62391fef3f29a9e9ba6c247a01023b`.
