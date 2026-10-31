# V1 optimization: measured progress

The measured joint candidate is `0adc796a99fd33abec85ab250ad9b2529507b49a` (production source equivalent to the earlier `4591856f7e1f66a00a8ef70874fb1046fa8c428d` screen), compared with the unchanged v1 engine at `e8265778ef78ec4acd9da1375eedb0be37a123f5`. All measurements below use musl and disk staging (`low_memory=true`), one worker, two cgroup CPUs, no swap and the calibrated local I/O envelope. Ordinary in-memory scans are outside this evidence.

## Current joint candidate: one-million-file screen

The [joint screen](v1-joint-screen-2026-10-31.md) has three paired comparisons for each operation, on 1,000,000 files / 1,004,499 entries and a 100-entry refreshed folder.

| Metric | Measured paired result | Interpretation |
| --- | --- | --- |
| Complete full-scan time | Median change −31.14% | Useful speed gain at this scale |
| Complete folder-refresh time | Median change −0.15% | No meaningful speed gain |
| Full-scan process RSS | About 36.9 MiB in both arms | No material reduction |
| Full-scan total worker-cgroup peak | About 1.81 GiB in both arms, under a 4 GiB cap | No material total-memory reduction; distinct from RSS |
| Refresh process RSS | Median change −23.84%, about 88 MiB saved | Below the 25% screening floor at this scale |
| Full-scan sampled app disk peak | +4,096 / +4,096 / +8,192 bytes | H2 remains inconclusive against the matched zero-byte control spread |
| Refresh sampled app disk peak | Zero paired change | No disk saving |

The speed gain does not establish qualification. The sampled disk cause remains unresolved, and three pairs on this smaller fixture do not replace the required full-scale confirmations.

## Full-scan total-memory investigation

The [2 GiB A/A control](v1-fullscan-memory-2048-aa-2026-10-31.md) completed all six full scans and independent semantic observers on the one-million-file fixture. Worker peaks were 1,942,683,648–1,943,085,056 bytes, about 1.81 GiB. All six had zero memory-limit/OOM events, zero memory PSI totals and no swap. This demonstrates successful operation at the tested 2 GiB cap for the baseline binary and this fixture; it does not establish a theoretical minimum or a source optimization.

The [joint comparison at the same 2 GiB cap](v1-fullscan-memory-2048-joint-2026-10-31.md) also passed all six commands and observers, without recorded OOM, memory-limit events or memory PSI. Median paired scan time improved by 19.765%; process RSS increased by 0.415%, so H2 remains inconclusive. Total worker-cgroup peaks remained about 1.81 GiB. These results and the 4 GiB screen are separate paired experiments on separate runners; their timing differences do not prove an effect caused by the cap.

The initial [1 GiB A/A attempt](v1-fullscan-memory-1024-aa-2026-10-31.md) failed in the build's cleanup unit test before any worker ran. It is invalid evidence about application capacity. The fixture allowed real host filesystem sync through its mocked timeout wrapper; a test-only repair now mocks sync and checks flags, ordering and failure status. Production application and measurement source are byte-identical to the earlier control and joint branches. Exact Opus 5.5 review later identified an exact-target cleanup P3. The correction and focused NO_FINDINGS review apply to integrated future worktrees; the frozen measured sources remain unchanged. Nine isolated tests passed before the smoke. The separate [100k-file protocol smoke](v1-runner-cleanup-repair-smoke-2026-10-31.md) passed with actual baseline binary identity, 111 Python tests and 307 Rust tests (one ignored).

The repaired [1 GiB A/A](v1-fullscan-memory-repaired-1024-aa-2026-10-31.md) and [joint](v1-fullscan-memory-repaired-1024-joint-2026-10-31.md) runs, followed by [512 MiB A/A](v1-fullscan-memory-repaired-512-aa-2026-10-31.md) and [joint](v1-fullscan-memory-repaired-512-joint-2026-10-31.md) runs, all completed six scans and semantic observers. Workers reached their caps and experienced reclaim pressure, with no recorded hierarchical OOM; swap was disabled. At 512 MiB, the joint comparison's median time changed −19.779%, RSS +192 KiB and disk +28,672 bytes. Positive RSS and disk medians leave H2 inconclusive; the A/A spread does not waive that rule.

The complete preregistered 1m ladder therefore shows successful baseline and joint operation down to the lowest tested 512 MiB cap. This result is floor-censored: it establishes neither an exact minimum nor a candidate capacity saving. The original failed 1 GiB attempt remains invalid. The collector records hierarchical memory events, not `memory.events.local`; refresh-specific profile-v2 is not applicable to these scan-only runs. No lower-cap result is extrapolated to the 7.3m fixture.

For the earlier 7,264,383-file / 7,297,076-entry full scan, both the baseline and the **different SQL-only candidate** reached their 4 GiB cgroup ceiling while completing successfully. Lower full-scale budgets remain untested. The flat ceiling does not establish equal unconstrained demand or a 4 GiB requirement. Endpoint cache and kernel counters are not a decomposition of the earlier peak. See the [full-scan memory audit](v1-fullscan-total-memory-audit-2026-10-31.md).

## Earlier separate full-scale candidates

The [SQL-only full-scale pair](v1-combined-sql-full-2026-10-31.md) reduced scan time from 639.785 to 477.140 seconds (−25.42%). The [exact-capacity refresh pair](v1-refresh-capacity-full-2026-10-31.md) reduced process RSS from 1.723 to 1.175 GiB (−31.84%, about 562 MiB), with a small positive refresh-time delta and an inconclusive no-regression result. Each has one full-scale pair and measures a different candidate. Their gains cannot be added or attributed to the current joint source.

## Remaining work

Separately admit selected full-scale memory cases using matching forecasts. Resolve no-regression findings and run the required changed-folder and scale confirmations before promotion. The [new 8 MiB default candidate](v1-compact-stage-source-reviews-2026-11-01.md), `601ce381ac229923d2c791fd8e7c510f7b3f1dfc`, has compact temporary indexes and the comprehensive mutation harness; it is source-reviewed with dispositions but unmeasured. The [preregistered next cases](v1-compact-stage-manifest-2026-11-01.json) start with a real changed-full GitHub smoke and full new Rust tests, then conditional fresh matched screens. The 4 MiB variant remains a controlled experiment only. Old joint measurements do not qualify this new source.

The [first changed-full smoke](v1-compact-stage-changed-full-smoke-2026-11-01.md) failed in package setup: apt update timed out before tests, compilation, binary identification or measurement. It is invalid performance and semantic evidence. The [build-only mirror repair](v1-runner-apt-source-reviews-2026-11-01.json) received a focused validated Opus5.5 `NO_FINDINGS`; application and measurement code did not change. The replacement primary source is `471c4d14ed4881106ce1d0b6cd5d6361839b2bc8`, with control `4f22bf8e3b67f7f59049a72116b90c061fb1328b`. The [new preregistration](v1-compact-stage-https-manifest-2026-11-01.json) preserves the failed attempt and supersedes its unrun screens.

The independently [preregistered full-scale 1 GiB A/A case](v1-fullscan-memory-full-manifest-2026-11-01.json) concerns unchanged baseline capacity, not compact-source qualification. Its [forecast](v1-fullscan-memory-full-1024-aa-forecast-2026-11-01.md) totals 4,984 seconds, or 6,479.2 seconds with 30% margin inside the 7,200-second step cap. The new source `e0299a6385e27fef4f78b682e7a5d25b969a02a1` changes only build setup and tests from the matching 1m control; ordinary app and measurement source remains unchanged. No full-scale success below 4 GiB is claimed before completion.

The separate [staged-refresh prototype](v1-staged-refresh-isolated-smoke-2026-10-31.md) completed a valid 100k-file smoke after its two preserved invalid attempts. In one pair it reduced process RSS by 39.49%, increased total worker-cgroup peak by 3.94%, increased complete refresh time by 130.22% and increased sampled app disk by 47.90%. This bounded experiment is rejected; its conditional one-million-file screen was cancelled. The result does not qualify the prototype or characterize all disk-backed designs.

The [campaign ledger](v1-w1-campaign-2026-10-31.json) records actual completed and observed active job time, failed attempts and reserved envelopes under the owner-approved 60 aggregate standard-runner-hour cap. No installation, master merge or product backend switch has been made.

Completed campaign usage is 33,310 runner seconds (9h 15m 10s) of the approved 60 hours, including the failed package-setup attempt. The local `freespace.timer` was temporarily stopped for GitHub-only research; its `Persistent=yes` setting can cause catch-up on restoration. No installed backend or configuration switch was made.
