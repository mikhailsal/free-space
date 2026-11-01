# Canonical cache4 candidate screen

Run 41279047267 passed the frozen helper's validation. Independent recomputation matched both saved ordinary summaries, and all 12 observers validated 1,004,499 current and previous entries. Refresh profile-v2 structure passed; the original HWM monotonic contract was false and the raw traces remain preserved. The observed musl baseline binary was `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980` from source E826; the candidate binary was `e422489b9cd94b65e83950b7a308504bf1608e2c8cc0eefd0e3d73760a963006` from source 1af549a8. Both used epoch 1793464183.

| Operation | Median elapsed | Median process RSS | Median sampled app disk | Median total worker-cgroup peak |
| --- | --- | --- | --- | --- |
| Scan | −15.9895% | −10,488 KiB / −29.2797% | −4,567,040 B / −0.99338% | −14,393,344 B / −0.742525% |
| Refresh | +0.000788364 s / +0.00773713% | −90,264 KiB / −23.9506% | 0 B in all pairs | −3.13665% |

The positive refresh-time median leaves H2 **inconclusive** under the frozen rule, with no promotion or qualification advancement. The worst refresh increase of 2.0196% was inside the matched A/A maximum absolute spread of 3.6294%; this is not a confirmed slowdown, and the spread does not waive the positive-median rule. One of three scan pairs improved only 14.877%, near the 15% floor; no fully confirmed benefit is claimed. The small total scan-cgroup change is not a meaningful whole-memory saving.

This is a 1m-file, 4 GiB, three-pair-per-operation screen, not full-scale qualification. GitHub reported 326 Rust tests passed, one ignored, and 121 Python tests; build and paired jobs used 239 and 881 seconds. The frozen [gate assessment](v1-cache4-canonical-candidate-2026-11-01.json) is reproduced in the JSON and archived as its exact original payload, SHA-256 `343822a4722cb640c02252303d74a5a542a5df8709e8cc708fe9ef2d492539bb`.

The archive includes all 204 frozen manifest evidence entries, including both actual binaries, content-addressed immutable dispatch and live-log sidecars, current dispatch receipt, validation, calculator, metrics and assessment. The virtual `github_live_log_utf8_sha256` key maps to its verified content-addressed log. The manifest is a point-in-time snapshot. `RELOCATION.json` maps original evidence paths and `SHA256SUMS` indexes all 282 payloads; each archived payload was verified.

Archive SHA-256: `67cfbd6b284a59acb4bf7eefe9b5d6d6936c59bb4966b5869edaaf0e3218319a`.
