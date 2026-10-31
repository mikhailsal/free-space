# V1 optimization: measured progress

The current joint candidate is `4591856f7e1f66a00a8ef70874fb1046fa8c428d`, compared with the unchanged v1 engine at `e8265778ef78ec4acd9da1375eedb0be37a123f5`. All measurements below use musl and disk staging (`low_memory=true`), one worker, two cgroup CPUs, no swap and the calibrated local I/O envelope. Ordinary in-memory scans are outside this evidence.

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

The initial [1 GiB A/A attempt](v1-fullscan-memory-1024-aa-2026-10-31.md) failed in the build's cleanup unit test before any worker ran. It is invalid evidence about application capacity. The fixture allowed real host filesystem sync through its mocked timeout wrapper; a test-only repair now mocks sync and checks flags, ordering and failure status. Production application and measurement source are byte-identical to the earlier control and joint branches. Exact Opus review is still pending provider quota; the host inspected the repair and nine isolated tests passed. The separate [100k-file protocol smoke](v1-runner-cleanup-repair-smoke-2026-10-31.md) passed with actual baseline binary identity, 111 Python tests and 307 Rust tests (one ignored).

The distinct [repaired 1 GiB A/A control](v1-fullscan-memory-repaired-1024-aa-2026-10-31.md) then passed all six scans and observers on **one million files**. Every worker reached the 1 GiB cap; memory-limit events and 0.35–0.50 seconds of cumulative memory PSI were recorded, with no OOM. This is the baseline's lowest tested successful budget at this scale, not a theoretical minimum or a source optimization. The [joint 1 GiB comparison](https://github.com/mikhailsal/free-space/actions/runs/41245753697) is running. The 512 MiB cases remain conditional. No failed attempt is silently retried or used as capacity evidence.

Each remaining rung requires its own A/A control before the joint comparison. OOM, timeout, incomplete receipts or semantic failure stops descent. No scan-only profile-v2 or allocation trace is claimed: that diagnostic is specific to refresh. The collector records hierarchical memory events; it does not currently collect `memory.events.local`.

For the earlier 7,264,383-file / 7,297,076-entry full scan, both the baseline and the **different SQL-only candidate** reached their 4 GiB cgroup ceiling while completing successfully. Lower full-scale budgets remain untested. The flat ceiling does not establish equal unconstrained demand or a 4 GiB requirement. Endpoint cache and kernel counters are not a decomposition of the earlier peak. See the [full-scan memory audit](v1-fullscan-total-memory-audit-2026-10-31.md).

## Earlier separate full-scale candidates

The [SQL-only full-scale pair](v1-combined-sql-full-2026-10-31.md) reduced scan time from 639.785 to 477.140 seconds (−25.42%). The [exact-capacity refresh pair](v1-refresh-capacity-full-2026-10-31.md) reduced process RSS from 1.723 to 1.175 GiB (−31.84%, about 562 MiB), with a small positive refresh-time delta and an inconclusive no-regression result. Each has one full-scale pair and measures a different candidate. Their gains cannot be added or attributed to the current joint source.

## Remaining work

Complete the accepted memory ladder, then separately admit the selected full-scale memory cases using matching forecasts. Resolve no-regression findings and run the required changed-folder and scale confirmations before promotion. Compact temporary indexes, optional 4/8 MiB staged-cache arms, and comprehensive mutation integration are prepared but unpublished and await the requested outside review. Neither the compact-index nor cache variant has measured benefits.

The separate [staged-refresh prototype](v1-staged-refresh-isolated-smoke-2026-10-31.md) completed a valid 100k-file smoke after its two preserved invalid attempts. In one pair it reduced process RSS by 39.49%, increased total worker-cgroup peak by 3.94%, increased complete refresh time by 130.22% and increased sampled app disk by 47.90%. This bounded experiment is rejected; its conditional one-million-file screen was cancelled. The result does not qualify the prototype or characterize all disk-backed designs. The requested Claude Opus 5.5 review of the published test-isolation repair remains pending after its provider usage limit.

The [campaign ledger](v1-w1-campaign-2026-10-31.json) records actual completed and observed active job time, failed attempts and reserved envelopes under the owner-approved 60 aggregate standard-runner-hour cap. No installation, master merge or product backend switch has been made.
