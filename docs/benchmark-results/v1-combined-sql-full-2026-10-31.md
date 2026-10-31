# V1 combined SQL full-scale exploratory scan — 2026-10-31

[Run 41230966330](https://github.com/mikhailsal/free-space/actions/runs/41230966330) completed one paired scan on x86_64 Linux musl with local I/O and a 4 GiB no-swap cgroup limit. The fixture contained 7,264,383 files and 7,297,076 expected entries. The strict protocol-v3 summary is valid and recomputes identically to the recorded summary. This single pair is exploratory, not a three-pair confirmation or promotion. The refresh screen's H2 assessment remains open; the separately refined joint branch was not measured by this run.

Baseline source `e8265778ef78ec4acd9da1375eedb0be37a123f5` produced binary SHA-256 `674faf7ab978502a7d17674599083d575dd57fc9914703a2673154f93d790aa9`; candidate source `d1c9f014ed185d663947590b388b444c08da7fda` produced `e0302ebce40463283ee94562f706c502f0df3d63eed966a898bf4231603923d1`. Both were built with the recorded toolchain in the same run. The earlier A/A binary differed across builds despite source identity, leaving its noise band provisional for cross-build comparisons.

| Measurement | Baseline | Candidate | Paired change |
| --- | ---: | ---: | ---: |
| Elapsed | 639.785249 s | 477.139766 s | −162.645483 s (−25.421887%) |
| Process peak RSS | 35,272 KiB | 35,052 KiB | −220 KiB |
| Application disk peak | 3,363,758,080 B | 3,363,733,504 B | −24,576 B |
| Cgroup memory peak | 4,294,967,296 B | 4,294,967,296 B | Both reached the cap |

Cgroup accounting is separate from process RSS: both arms reached the 4 GiB limit, with `memory.events max` counts of 15,297 and 15,525 respectively and zero OOM or OOM-kill events. Endpoint memory pressure `some` and `full` totals were 5,219,440 μs for baseline and 4,956,433 μs for candidate; direct reclaim scanned 913,579 and 924,037 pages. These counters and the approximately 34 MiB process peaks do not establish a minimum RAM requirement; only 4 GiB was tested.

The admission forecast was 2,952 seconds, or 3,837.6 seconds with its 30% margin, against the 7,200-second measured-step cap. The entire paired job took 2,829 seconds, including setup and observers, under its 7,500-second job cap; the build job took 162 seconds under its 1,500-second cap. The forecast is an admission estimate, not a measured command duration. Build logs record 82 Python tests and 316 Rust tests passed, zero Rust failures, and one ignored Rust test.

The [receipt](v1-combined-sql-full-2026-10-31.json) retains the full description, validation, GitHub metadata and exact per-arm values. The [archive](v1-combined-sql-full-2026-10-31.tar.gz) contains 65 original result files, four supporting analysis and GitHub files, and an internal `SHA256SUMS`. All 69 payloads were verified against their originals before this report was written. The [SHA-256 index](v1-combined-sql-full-2026-10-31.sha256) lists every payload and the archive itself.
