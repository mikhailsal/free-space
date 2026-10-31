# V1 combined SQL candidate screen — 2026-10-31

[Run 41227699497](https://github.com/mikhailsal/free-space/actions/runs/41227699497) completed three paired scans and three paired refreshes on x86_64 Linux musl with local I/O and a 4 GiB cgroup limit. The 1,000,000-file fixture had 1,004,499 expected entries and a 100-entry refreshed subtree. Both ordinary paired summaries and the prospective refresh profile-v2 structure passed. **H2 remains inconclusive for both operations; the candidate is not promoted.**

The baseline source was `e8265778ef78ec4acd9da1375eedb0be37a123f5` and candidate `d1c9f014ed185d663947590b388b444c08da7fda`. Their binary SHA-256 values were `674faf7ab978502a7d17674599083d575dd57fc9914703a2673154f93d790aa9` and `e0302ebce40463283ee94562f706c502f0df3d63eed966a898bf4231603923d1`, respectively. The earlier A/A control binary hash differed across builds despite source identity; matched A/A artifact identity is unestablished, so its noise band is provisional for this comparison. This difference alone does not invalidate these paired receipts.

| Measurement | Pair 1 | Pair 2 | Pair 3 | Median |
| --- | ---: | ---: | ---: | ---: |
| Scan elapsed change | −31.468764% | −31.295512% | −31.329744% | −31.329744% |
| Scan RSS change | −8 KiB | +80 KiB | −52 KiB | −8 KiB |
| Scan application disk change | +4,096 B | +4,096 B | +4,096 B | +4,096 B |
| Refresh elapsed change | +0.461511% | −0.010999% | +0.151413% | +0.151413% |
| Refresh RSS change | +132 KiB | −12 KiB | −16 KiB | −12 KiB |
| Refresh application disk change | 0 B | 0 B | 0 B | 0 B |

Scan's repeated 4,096-byte disk increase leaves its H2 assessment inconclusive despite the elapsed-time decrease. Refresh's worst elapsed regression, +0.461511%, exceeds the earlier A/A maximum of 0.209636%, leaving refresh H2 inconclusive. Percent changes from separate historical runs cannot be added to infer a combined gain.

The original monotonic high-water-mark contract failed: all six refresh traces contain a pointwise decrease. The prospective profile-v2 structural pass preserves those anomalies rather than changing the original verdict. Only a 4 GiB limit was tested; minimum required RAM is unknown.

The build passed 82 Python tests and 316 Rust tests, with zero Rust failures and one ignored. Build and paired jobs took 235 and 1,193 seconds respectively. These job durations include setup and observers.

The [receipt](v1-combined-sql-screen-2026-10-31.json) preserves all per-arm metrics and profile data. The [archive](v1-combined-sql-screen-2026-10-31.tar.gz) contains 256 raw artifact files, four supporting analysis and GitHub files, and an internal `SHA256SUMS`. Every archived payload was verified against its original before this report was written. The [SHA-256 index](v1-combined-sql-screen-2026-10-31.sha256) lists each payload and the archive itself.
