# Root-reuse changed-full smoke

Run 41259537703 passed on source `63d98cc8ff88a54ab9687c5fb93fffd930e33d26`; the new multi-root, one-query Rust regression test passed on GitHub. This was a single paired refresh smoke on 100,000 files / 100,409 entries, with a 10,000-entry changed subtree; it is not qualification evidence. Strict summary recomputation matched the raw summary and passed.

The observed baseline binary SHA-256 was `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`; the candidate was `3b96c71434f981cfef9c23dd77f41136c18ac04c59e3f6211a437721b3cf35ac`. Both arms used the same seed SHA. Their independent observers validated 100,409 current and previous entries and matched semantic and previous-snapshot hashes. Bounded link checks observed promotion of the outside old link after its inside owner disappeared and the new inside alias of the outside-owned inode. Seed ownership, device/inode facts and the complete balanced mutation receipt are retained in the archive and JSON.

The baseline and candidate refresh times were 1.383 and 1.385 seconds, process RSS 74,664 and 62,348 KiB, and sampled application disk peak 36,732,928 bytes in both. These one-pair results do not establish a performance gain. Both worker cgroups had 1 GiB limits, no swap or hierarchical OOM events; setup and observers used 4 GiB. Profile-v2 trace structure passed, while the original HWM monotonic contract was false for both arms; raw anomalies are preserved.

GitHub reported 326 Rust tests passed, one ignored, and 121 Python tests passed; the root-reuse query-count and multi-root test passed before measurement. Build and paired jobs used 296 and 94 seconds, respectively. Runner preparation succeeded with 91,856,781,312 bytes available and no deletion actions, so cleanup under low disk remains untested.

The archive preserves 68 raw artifact files and supporting receipts, full logs, validation and exact workflow source; all 86 indexed payloads were verified. Archive SHA-256: `f580066e4f666578764b0eaa7609ed5f8db654f8d246a0ce3c7b63c6b0683f7f`.
