# V1 refresh capacity candidate screen — 2026-10-31

[Run 41223904260](https://github.com/mikhailsal/free-space/actions/runs/41223904260) completed three paired scan and refresh measurements on x86_64 Linux musl with a 4 GiB no-swap limit. Strict paired summaries passed for both operations and the prospective profile-v2 structure passed for all six refresh arms. The candidate is **not promoted**: its median refresh RSS reduction was 23.8824%, below the preregistered 25% floor. The H2 no-regression assessment remains **inconclusive**.

The baseline source revision was `e8265778ef78ec4acd9da1375eedb0be37a123f5` and the candidate was `f04a109124c3747d937547e4fc1f6d37ad59392a`; the paired builds recorded matching compiler and flags. The current baseline binary SHA-256 is `4c2ad151c828b60050602bb88ec7297b436bdfd817cfb04503156abd8a72a8ca`, differing from the earlier A/A binary `083bde706e142cede8d400b0c5c6639257c1bf1ca36f9d181b5e30180ac966bb`. Source identity is verified, but matched A/A artifact identity is not established and is under investigation. This difference alone does not invalidate the current paired receipts. The earlier A/A noise band is provisional for this comparison.

| Measurement | Paired changes, candidate versus baseline | Median |
| --- | --- | --- |
| Refresh peak RSS | −23.8824%, −23.8848%, −23.8512% | −23.8824% |
| Refresh wall time | −0.6595%, +0.1131%, −0.3827% | −0.3827% |
| Refresh app disk peak | 0, 0, 0 bytes | 0 bytes |
| Scan wall time | +0.1082%, −0.0805%, +0.7684% | +0.1082% |
| Scan peak RSS | +48, +112, −264 KiB | +48 KiB (+0.1267%) |
| Scan app disk peak | 0, 0, +4,096 bytes | 0 bytes |

Refresh median process peak RSS was 377,944 KiB baseline and 287,676 KiB candidate. The largest refresh wall-time regression is inside the provisional earlier A/A band. In every refresh pair, baseline node capacity grew from 1,004,499 to 2,008,998 with a changed pointer; candidate capacity stayed at 1,004,598 with no change. Baseline RSS rose 90,116 KiB between the name and node markers in each pair; candidate rose 0 KiB. Name capacity remained unchanged. Boundary RSS is sampled and does not establish exact phase peaks or physical overlap.

The profile records seven pointwise decreases in `/proc` VmHWM across six traces. The original monotonic-HWM contract fails; profile-v2 describes the anomaly and does not change the old verdict or the product gate. All raw values, per-arm flags and anomalies are preserved in the [receipt](v1-refresh-capacity-screen-2026-10-31.json) and archive. Only 4 GiB was tested; minimum viable memory remains unknown.

The GitHub build passed 310 Rust tests, with zero failed and one ignored, including the three new capacity tests, and 82 Python tests. Candidate compilation and tests ran on GitHub; local pre-push hooks ran checks but did not run Rust tests. Build and paired jobs used 259 and 1224 runner-seconds respectively. Any exploratory full-scale counterfactual remains a separate decision under the preregistration and is not promotion.

The [archive](v1-refresh-capacity-screen-2026-10-31.tar.gz) contains all 256 original files, four supporting analysis and GitHub files, and `SHA256SUMS`. The [SHA-256 index](v1-refresh-capacity-screen-2026-10-31.sha256) lists every archived input and the archive itself; the embedded index lists the archived inputs. The [machine-readable receipt](v1-refresh-capacity-screen-2026-10-31.json) retains the full descriptive metrics and profile.
