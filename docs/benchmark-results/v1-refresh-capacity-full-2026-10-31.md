# V1 refresh capacity full-scale counterfactual — 2026-10-31

[Run 41227692011](https://github.com/mikhailsal/free-space/actions/runs/41227692011) completed one exploratory pair of full-scale refreshes with a 4 GiB no-swap limit. The fixture had 7,264,383 files, 7,297,076 entries and a 100-entry refreshed subtree. The strict paired summary and profile-v2 structure were valid. This is one observation, and the candidate is **not promoted**.

| Measurement | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Wall time | 74.188183195 s | 74.585005117 s | +0.396821922 s (+0.5348856%) |
| Process peak RSS | 1,806,980 KiB | 1,231,700 KiB | −575,280 KiB (−561.797 MiB; −31.836545%) |
| App disk peak | 2,738,307,072 B | 2,738,298,880 B | −8,192 B |

The one-pair RSS result meets the 25% and 256 MiB thresholds descriptively. Wall time increased by 0.396822 s, above the prior full-scale A/A maximum absolute time difference of 0.251153 s (about 0.3365% of this baseline). H2 no-regression is therefore **inconclusive**, and no promotion follows. Only 4 GiB was tested; minimum required memory is unknown.

The baseline node capacity grew from 7,297,076 to 14,594,152 and its pointer changed. Candidate capacity stayed at 7,297,175 with no pointer change through grafting; the 99 additional nodes correspond to the actual subtree descendants. Boundary samples are not exact phase peaks and do not alone establish physical overlap.

The two traces have one decreasing `/proc` VmHWM reading each (−268 and −68 KiB). The original monotonic-HWM contract fails; the prospective profile-v2 records these observations without rewriting the old verdict or product gate. The [machine-readable receipt](v1-refresh-capacity-full-2026-10-31.json) and raw archive preserve all readings, flags and anomalies.

The baseline source was `e8265778ef78ec4acd9da1375eedb0be37a123f5`, binary SHA-256 `4c2ad151c828b60050602bb88ec7297b436bdfd817cfb04503156abd8a72a8ca`; the candidate was `f04a109124c3747d937547e4fc1f6d37ad59392a`, binary SHA-256 `54aa94b40e85d69f58d90855087e18951f73f45052d7e71057e4f18768a2d55b`. The build passed 310 Rust tests, with one ignored and zero failed, including the three new tests, and 82 Python tests. Build and paired jobs used 171 and 1916 runner-seconds respectively.

The [archive](v1-refresh-capacity-full-2026-10-31.tar.gz) contains the 67 original artifact files, four analysis and GitHub support files, and `SHA256SUMS`. The [external SHA-256 index](v1-refresh-capacity-full-2026-10-31.sha256) covers the 71 input files and archive; the embedded index covers the inputs. Original files are unchanged.
