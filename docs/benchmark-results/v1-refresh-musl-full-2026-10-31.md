# Full-scale musl refresh control — 2026-10-31

[GitHub run 41222190491](https://github.com/mikhailsal/free-space/actions/runs/41222190491) succeeded at source revision `e8265778ef78ec4acd9da1375eedb0be37a123f5`. Both control arms used the same release binary, SHA-256 `083bde706e142cede8d400b0c5c6639257c1bf1ca36f9d181b5e30180ac966bb`, targeting `x86_64-unknown-linux-musl`. The fixture contained 7,264,383 files and 7,297,076 expected entries; the refreshed subtree had 100 entries. One refresh pair ran on one VM with local I/O and a 4 GiB cgroup limit. The ordinary summary and prospective profile-v2 structural validation both passed.

| Measurement | Baseline control | Candidate control |
| --- | ---: | ---: |
| Elapsed | 74.629683 s | 74.378530 s |
| Process peak RSS | 1,807,064 KiB | 1,806,996 KiB |
| Application disk peak | 2,743,390,208 bytes | 2,743,386,112 bytes |
| Cgroup peak | 2,616,336,384 bytes | 2,618,187,776 bytes |

The names baseline and candidate identify two runs of the same binary. This single pair is descriptive; it is not a three-pair confirmation or evidence of an optimization. Both arms completed under the 4 GiB limit with zero recorded cgroup OOM events; the minimum required RAM remains unknown. A RAM optimization candidate is pending separate evaluation and cannot be credited with improvement from another VM's control.

In the baseline profile, boundary RSS was 1,165,268 KiB after loading, 1,798,168 KiB after node insertion, and 1,807,268 KiB after finalization. Node capacity increased from 7,297,076 to 14,594,152 and the node buffer moved; name capacity did not change. These boundary samples and buffer movement do not establish copying, physical overlap, or a phase peak.

The original high-water-mark monotonicity contract failed: the index sample was 204 KiB below the prior baseline high-water sample and 268 KiB below the prior candidate sample. The anomalies remain in the archive. Profile-v2 is a prospective structural policy; its pass does not retroactively reclassify older GNU results or erase these anomalies.

The build job took 114 seconds and the paired job 1,760 seconds. The build log records 82 Python benchmark tests passed and 307 Rust workspace tests passed, with one Rust test ignored and none failed. Repository gates also passed. These durations include setup and observers; they are distinct from measured refresh elapsed time.

The accompanying JSON preserves the description, profile and job evidence. The deterministic archive includes all 72 original result and auxiliary files, including the forecast, GitHub metadata and log, plus an internal `SHA256SUMS` index. Its external digest is in the accompanying `.sha256` file. Every archived file was checked against its original after creation.
