# Joint candidate 4 GiB screen — 2026-10-31

[Run 41237971343](https://github.com/mikhailsal/free-space/actions/runs/41237971343) completed three scan and three refresh pairs on the 1,000,000-file fixture (1,004,499 expected entries; refreshed subtree 100 entries), musl, local I/O and a 4 GiB no-swap measured-worker cap. Strict protocol-v4 summaries recomputed exactly from raw receipts and were valid for both operations. **H2 no-regression is not established and remains inconclusive; the candidate is not promoted.** Scan application disk increased in all three pairs while the matched A/A control observed zero change.

Baseline source `e8265778ef78ec4acd9da1375eedb0be37a123f5` used binary SHA-256 `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`; joint candidate `4591856f7e1f66a00a8ef70874fb1046fa8c428d` used `f0c25acd428e150a0ad5e1139f5ef7b14cf6404dacd1cd55e0d03cec166a6e74`. The [matched A/A screen](v1-joint-aa-screen-2026-10-31.md), run 41237970872, used the identical baseline binary in both arms. Its observed maxima were 0.694% scan time, 340 KiB scan RSS and zero scan disk; refresh maxima were 9.522% time, 236 KiB RSS and zero disk. The control variation is descriptive, not a confidence bound.

| Measurement | Pair 1 | Pair 2 | Pair 3 | Median |
| --- | ---: | ---: | ---: | ---: |
| Scan elapsed change | −31.2403% | −31.1361% | −30.8266% | −31.1361% |
| Scan process RSS change | −100 KiB | +36 KiB | −28 KiB | −28 KiB |
| Scan application disk change | +4,096 B | +4,096 B | +8,192 B | +4,096 B |
| Refresh elapsed change | −0.1522% | +0.0192% | −0.4152% | −0.1522% |
| Refresh process RSS change | −23.8443% | −23.8447% | −23.8916% | −23.8447% |
| Refresh application disk change | 0 B | 0 B | 0 B | 0 B |

The one-million-entry refresh RSS reduction falls short of the preregistered 25% floor. The positive scan disk changes exceed the observed A/A zero-byte band, so the scan-time improvement does not waive the unresolved H2 result. Percent medians are medians of paired percentage changes, not ratios of marginal medians.

Scan process peaks were approximately 37,800 KiB. Total scan cgroup peaks were 1,942,892,544–1,943,089,152 B baseline and 1,942,249,472–1,942,491,136 B candidate: essentially unchanged in this fixture and distinct from process RSS. All six scan arms reported zero `memory.events` max/OOM and zero memory PSI totals. Only 4 GiB was tested, so minimum successful RAM remains unknown. The prospective refresh profile-v2 structure passed; the original HWM monotonic contract failed with six recorded anomalies, which remain in the archive.

Runner preparation succeeded without action (`acted: false`), with 91,855,609,856 bytes available before and after and duration 0.000208 s. The SDK low-space removal branch was not exercised. Build and paired jobs took 274 and 1,194 seconds. Build logs show 110 Python tests and 320 Rust tests passed, with zero Rust failures and one ignored.

The [receipt](v1-joint-screen-2026-10-31.json) retains per-arm measurements and the matched-control description. The [archive](v1-joint-screen-2026-10-31.tar.gz) contains 257 raw files and seven supporting validation, profile, description and GitHub files, plus an internal `SHA256SUMS`. Every payload was verified against its original before this report was written. The [SHA-256 index](v1-joint-screen-2026-10-31.sha256) lists each payload and the archive itself.
