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

The joint comparison at the same 2 GiB cap is [running](https://github.com/mikhailsal/free-space/actions/runs/41241552473). The preregistered next rungs are 1 GiB and 512 MiB, each with its own A/A control before the joint comparison. OOM, timeout, incomplete receipts or semantic failure stops descent. No scan-only profile-v2 or allocation trace is claimed: that diagnostic is specific to refresh.

For the earlier 7,264,383-file / 7,297,076-entry full scan, both the baseline and the **different SQL-only candidate** reached their 4 GiB cgroup ceiling while completing successfully. Lower full-scale budgets remain untested. The flat ceiling does not establish equal unconstrained demand or a 4 GiB requirement. Endpoint cache and kernel counters are not a decomposition of the earlier peak. See the [full-scan memory audit](v1-fullscan-total-memory-audit-2026-10-31.md).

## Earlier separate full-scale candidates

The [SQL-only full-scale pair](v1-combined-sql-full-2026-10-31.md) reduced scan time from 639.785 to 477.140 seconds (−25.42%). The [exact-capacity refresh pair](v1-refresh-capacity-full-2026-10-31.md) reduced process RSS from 1.723 to 1.175 GiB (−31.84%, about 562 MiB), with a small positive refresh-time delta and an inconclusive no-regression result. Each has one full-scale pair and measures a different candidate. Their gains cannot be added or attributed to the current joint source.

## Remaining work

Complete the accepted memory ladder, then separately admit the selected full-scale memory cases using matching forecasts. Resolve disk H2 and run the required changed-folder and scale confirmations before promotion. The compact temporary-index candidate and comprehensive mutation integration are prepared but unpublished and await the requested outside review. The separate staged-refresh prototype has no valid performance measurements: its two smoke attempts stopped at correctness or test-isolation gates. The cleanup fixture correction awaits Claude Opus 5.5 after its provider usage limit resets.

The [campaign ledger](v1-w1-campaign-2026-10-31.json) records actual completed and observed active job time, failed attempts and reserved envelopes under the owner-approved 60 aggregate standard-runner-hour cap. No installation, master merge or product backend switch has been made.
