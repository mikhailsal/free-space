# Root-reuse candidate screen

Run 41260421846 passed strict recomputation for scan and refresh and all 12 independent current/previous semantic observers for 1,004,499 entries. Three pairs per operation compared source `63d98cc8ff88a54ab9687c5fb93fffd930e33d26` with baseline `e8265778ef78ec4acd9da1375eedb0be37a123f5`. Both observed binary hashes matched their frozen values: baseline `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`, candidate `3b96c71434f981cfef9c23dd77f41136c18ac04c59e3f6211a437721b3cf35ac`.

Median paired scan changes were −32.202% elapsed, +4.607% GNU-time process RSS and −0.986% sampled app disk peak. The positive RSS median leaves H2 inconclusive; each RSS increase exceeds the fresh A/A maximum absolute spread of 192 KiB and 0.5112%. Median refresh changes were −0.354% elapsed, −23.852% RSS and zero sampled disk; the RSS improvement is below the 25% benefit floor. No candidate promotion follows from this screen.

Scan worker-cgroup peaks were 1,941,229,568–1,941,430,272 bytes baseline and 1,938,735,104–1,939,140,608 bytes candidate, distinct from process RSS. All workers had 4 GiB caps, no swap and zero hierarchical max/OOM events and memory PSI totals. These same-cap measurements do not establish minimum RAM or a capacity improvement. Refresh profile-v2 structure passed; the original HWM monotonic contract was false, and raw anomalies are preserved.

Refresh wall-time paired differences were −37.654, −18.012 and −142.977 ms. In the earlier 2d2 candidate run on another VM, they were +8.913, +25.007 and +37.347 ms. In this run paired load differences were −111.096/−105.848/−101.471 ms and walk +95.849/+92.829/+92.369 ms; previous-run load differences were −82.430/−62.415/−81.782 ms and walk +91.780/+94.064/+93.886 ms. These are descriptive comparisons across separate runs, not proof attributing a 25 ms change to the query removal. Full phase timings and per-device `io.stat` are retained; device layers are not summed.

Runner preparation succeeded with 91856797696 available bytes and no cleanup actions. GitHub reported 326 Rust tests passed, one ignored, and 121 Python tests passed; the root-reuse regression test passed. Build and paired jobs used 176 and 1,203 runner seconds.

The archive contains 257 original artifact files and supporting evidence, fresh A/A description, build metadata and exact workflow; all 274 indexed payloads were verified. Archive SHA-256: `b55841caa6e12fce2671eef711820f758849f7b8082c36cdd9735bc2a9f17e8a`.
