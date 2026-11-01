# Canonical cache4 A/A control

Run 41277463149 passed the frozen helper's validation. Independent recomputation matched both saved ordinary summaries, and all 12 observers validated 1,004,499 current and previous entries. Refresh profile-v2 structure passed; the original HWM monotonic contract was false and its raw traces remain preserved. Both independently built musl binaries matched ED2 (`ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`), with baseline source E826, control source 4f22 and common epoch 1793464183.

The maximum absolute same-app paired differences were:

| Operation | Time | Process RSS | Sampled app disk | Total worker-cgroup peak |
| --- | --- | --- | --- | --- |
| Scan | 1.549327% / 0.986325 s | 196 KiB / 0.517697% | 0 B | 196,608 B / 0.010114% |
| Refresh | 3.629407% / 0.401021 s | 168 KiB / 0.044451% | 0 B | 23,621,632 B / 5.412127% |

These are descriptive maxima from three pairs, not confidence bounds; the refresh and whole-cgroup spreads do not provide tight calibration. The outlying raw pair remains intact. This A/A control does not establish a code gain or candidate promotion. GitHub reported 307 Rust tests passed, one ignored, and 121 Python tests; build and paired jobs used 159 and 1,224 seconds.

The archive includes all 204 frozen manifest evidence entries, including both actual binaries, the content-addressed immutable dispatch and live-log sidecars, plus the current registered dispatch receipt, validation, calculator and metrics. The virtual `github_live_log_utf8_sha256` key maps to the verified content-addressed live log. The included manifest is a point-in-time snapshot, not the final state of the candidate case. `RELOCATION.json` maps original evidence paths and `SHA256SUMS` indexes all 281 payloads; each archived payload was verified.

Archive SHA-256: `765963cc7dedd716a72834b3dcc5a61cdefed55a491605fc5ec6a1bc5c0d1d46`.
