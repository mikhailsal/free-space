# Compact-stage HTTPS candidate screen

Run 41255297627 passed strict summary recomputation and all 12 current/previous semantic observers for the 1,000,000-file / 1,004,499-entry fixture. It compares source `471c4d14ed4881106ce1d0b6cd5d6361839b2bc8` with baseline `e8265778ef78ec4acd9da1375eedb0be37a123f5`, three pairs each for scan and unchanged 100-entry refresh. Observed binary hashes matched the preregistered candidate `2d2fd14fc97dcdc3737523a0ec24616448b2315f2c6ae2d9cb95718515d18c5b` and baseline `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`.

Median paired scan changes were −27.910% elapsed time, −1.081% process RSS and −0.992% sampled app disk peak. Median paired refresh changes were +0.235% elapsed time, −23.824% RSS and zero disk. Refresh's positive time median leaves H2 inconclusive even though the fresh A/A maximum absolute time spread was 0.3318%; the spread does not waive the rule. The RSS change is below the 25% screening floor. These results do not promote the candidate.

Full-scan worker-cgroup peaks were 1,943,556,096–1,943,826,432 bytes for baseline and 1,939,075,072–1,939,255,296 bytes for candidate, separately from GNU-time process RSS. All workers had a 4 GiB cap, no swap, zero hierarchical max/OOM and zero memory PSI totals. Same-cap results establish neither minimum RAM nor a capacity gain. Refresh profile-v2 structural validation passed; the original HWM monotonic contract was false, with raw traces retained.

Scan walk phases were approximately 34.19 seconds in both arms; insertion phases ranged 27.61–27.81 seconds baseline and 9.19–9.33 seconds candidate. Refresh phase values are retained in the receipt; timing marker boundaries differ, so phase labels alone do not establish a load-only change. Per-device `io.stat` values are preserved separately for loop device 7:0 and underlying device 8:0; they are not added, because they can represent the same I/O at different layers.

Runner preparation succeeded with 91856830464 available bytes and no cleanup actions. GitHub recorded 325 Rust tests passed, one ignored, and 121 Python tests passed. Build and paired jobs used 153 and 1,183 runner seconds.

The archive contains 257 original artifact files and supporting evidence, fresh A/A description, build metadata and exact workflow; all 270 indexed payloads were verified. Archive SHA-256: `f92bdd582b0fac5d2b884b64cfb36d1f9736b08e67c9e77e087e632cf72ad042`.
