# Compact-stage HTTPS changed-full smoke

Run 41252735964 passed on source `471c4d14ed4881106ce1d0b6cd5d6361839b2bc8`; the repaired HTTPS musl installation succeeded. The earlier failed setup remains separately preserved. This was a single paired refresh smoke on 100,000 files / 100,409 entries, with a 10,000-entry changed subtree; it is not qualification evidence. Strict summary recomputation matched the raw summary and passed.

The observed baseline binary SHA-256 was `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`; the candidate was `2d2fd14fc97dcdc3737523a0ec24616448b2315f2c6ae2d9cb95718515d18c5b`. Both arms used the same seed SHA. Their independent observers validated 100,409 current and previous entries and matched semantic and previous-snapshot hashes. Bounded link checks observed promotion of the outside old link after its inside owner disappeared and the new inside alias of the outside-owned inode. Seed ownership, device/inode facts and the complete balanced mutation receipt are retained in the archive and JSON.

The baseline and candidate refresh times were 1.220 and 1.139 seconds, process RSS 74,708 and 62,284 KiB, and sampled application disk peak 36,732,928 bytes in both. These one-pair results do not establish a performance gain. Both worker cgroups had 1 GiB limits, no swap or hierarchical OOM events; setup and observers used 4 GiB. Profile-v2 trace structure passed, while the original HWM monotonic contract was false for both arms; raw anomalies are preserved.

GitHub reported 325 Rust tests passed, one ignored, and 121 Python tests passed; the compact-index query-plan tests passed before measurement. Build and paired jobs used 291 and 83 seconds, respectively. Runner preparation succeeded with 91,856,834,560 bytes available and no deletion actions, so cleanup under low disk remains untested.

The archive preserves 68 raw artifact files and supporting receipts, full logs, validation and exact workflow source; all 76 indexed payloads were verified. Archive SHA-256: `8e9135e9fb0b4350d6ba7ee1a485991ef0ab42e0c19f86c83aaee5ef8f94e98d`.
