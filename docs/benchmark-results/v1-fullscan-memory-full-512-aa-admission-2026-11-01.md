# Full-scale 512 MiB A/A admission assessment

**NOT ADMITTED:** the conservative whole-step forecast is 6,340 seconds, and its 30% margin is **8,242 seconds**, exceeding the fixed 7,200-second step cap. Full-scale 512 MiB capacity remains unmeasured. Do not increase the timeout, reduce the fixture, waive a gate, or dispatch on this assessment.

The proposed case is one paired scan of 7,264,383 files / 7,297,076 entries, local I/O and musl, two CPUs and one worker, no swap, with 512 MiB worker and separate 4 GiB setup/observer limits. Case ID is `x86_64-unknown-linux-musl|local|scan|100|unchanged|512`. The intended control is `e0299a6385e27fef4f78b682e7a5d25b969a02a1` versus baseline `e8265778ef78ec4acd9da1375eedb0be37a123f5`, with both actual binaries required to match `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`.

The source 1m 512 MiB A/A run 41248061322 used control `55d6e3460f34710e2520b77da66e659d410f6c25`; its ordinary application and measurement code are unchanged in `e029`. Six worker times had a maximum of 85.3565 s; the largest observer was 28.5194 s, largest copy 0.5291 s, and slower seed 39.8870 s. Fixture setup was 61.9545 s. The file scale is 7.264383 and entry scale 7.264393. The 3-pair source has six measured workers/observers; this proposed one-pair case uses **two**, each conservatively costed at the larger per-arm observation.

The cross-check is the completed full-scale 1 GiB A/A run 41252744694, same actual binaries, on another VM. Its slower worker was 1,303.922 s, slower observer 227.750 s, slower copy 6.595 s, slower seed 350.562 s, and fixture setup 459.187 s. The two identical-binary workers differed by 16.751% in time. These cross-VM/cap observations are planning cross-checks, not causal estimates or 512 MiB measurements.

| Component | Seconds | Conservative construction |
| --- | ---: | --- |
| Fixture | 460 | Ceiling of maximum of 1m fixture × file scale and full 1 GiB observed fixture |
| Two seeds | 702 | Ceiling of twice the larger of scaled slower 1m seed and slower full 1 GiB seed |
| Two measured workers | 2,608 | Ceiling of twice the larger of scaled slowest 1m worker and slower full 1 GiB worker |
| Two observers | 456 | Ceiling of twice the larger of scaled slowest 1m observer and slower full 1 GiB observer |
| Two copies | 14 | Ceiling of twice the larger of scaled slowest 1m copy and slower full 1 GiB copy |
| Calibration, setup, cleanup | 600 | Same fixed allowance as the prior forecast |
| Lower-cap pressure uncertainty | 1,500 | Same explicit allowance; not reduced to force admission |
| **Whole-step total** | **6,340** | **8,242 with 30% margin; exceeds 7,200** |

The pre-allowance per-worker estimate is 1,303.922 s, below the 1,800 s command cap, but the 1,500 s pressure allowance is whole-step and does not establish a credible per-worker upper bound at full-scale 512 MiB. Thus command-cap feasibility is not assured either. The fixed 7,800 s paired-job cap plus 1,500 s build cap remains 9,300 runner-seconds; budget availability cannot cure the failed step-margin test. OOM, timeout, incomplete evidence or failed semantics would be invalid, not proof of a capacity threshold. No minimum RAM or source gain can be inferred from a cap-only change.

The accompanying JSON records exact component inputs, all six source arms and both full-scale arms, source provenance, and SHA-256 hashes of every raw receipt read. Any future proposal requires new supporting evidence and separate admission under unchanged constraints.
