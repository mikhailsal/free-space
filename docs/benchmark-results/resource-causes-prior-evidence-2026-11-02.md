# Prior evidence for resource causes — 2026-11-02

Existing source and archived receipts establish several representation costs. They do not yet provide a synchronized ordinary-run disk inventory or a complete causal decomposition of worker RAM. The new RAM/disk cause experiments have **not executed** at this documentation checkpoint. This document records prior evidence; it makes no qualification, promotion or goal-completion claim.

All work for this audit was read-only apart from this document: source reads, gzip JSON parsing and selected tar members read in memory. No application, build, workload, new benchmark or GitHub dispatch ran. MiB means 2²⁰ bytes; GiB means 2³⁰ bytes. Component maxima from different observations are never added to construct a simultaneous peak.

## Evidence identities and verification

| Evidence | Run and source | Archived binary SHA-256 and conditions |
| --- | --- | --- |
| Original repeated v1/v2 full scans | [41134701592](https://github.com/mikhailsal/free-space/actions/runs/41134701592); harness `5b62c148d14f5b3a1f947928d11ec2430b045a3c`; application source equivalent to `afce6fdfc007a2c6914cab61aab4d84daccbb99b` | Both backends: `8fd2da780c44c740373ee542d9a077ba35aa149b663add65a7dbc1be0ab49773`; GNU build, native runner I/O, 7,202,464 entries, five unchanged full scans, one sequence, disk staging, previous retained, 4 GiB/no swap, two CPUs/one worker. |
| Canonical v1 candidate comparison | [41279047267](https://github.com/mikhailsal/free-space/actions/runs/41279047267); baseline `e8265778ef78ec4acd9da1375eedb0be37a123f5` (E826); candidate `1af549a8f1d834fb292e78a6cbd48ad9d12252cd` (1af) | Baseline `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980` (ED2); candidate `e422489b9cd94b65e83950b7a308504bf1608e2c8cc0eefd0e3d73760a963006`; musl, epoch 1793464183, local runner I/O, 1,004,499 entries/1,000,000 files, three pairs, 4 GiB/no swap. |
| Full-scale v1 baseline A/A | [41252744694](https://github.com/mikhailsal/free-space/actions/runs/41252744694); E826 versus control `e0299a6385e27fef4f78b682e7a5d25b969a02a1` | Both ED2; musl, local runner I/O, 7,297,076 entries/7,264,383 files, one scan pair at 1 GiB/no swap. |
| Exact-capacity refresh strategy | Screen [41223904260](https://github.com/mikhailsal/free-space/actions/runs/41223904260), full-scale [41227692011](https://github.com/mikhailsal/free-space/actions/runs/41227692011); E826 versus `f04a109124c3747d937547e4fc1f6d37ad59392a` (f04) | Baseline `4c2ad151c828b60050602bb88ec7297b436bdfd817cfb04503156abd8a72a8ca`; candidate `54aa94b40e85d69f58d90855087e18951f73f45052d7e71057e4f18768a2d55b`; musl, local runner I/O, unchanged 100-entry target, 4 GiB/no swap, two CPUs/one worker. Screen: three pairs at 1,004,499 entries; full-scale: one pair at 7,297,076 entries. |

Binary identities above were checked against archived manifest provenance and existing reports. This audit did not rebuild binaries or independently rehash their executable payloads. The older exact-capacity baseline hash is **not ED2**, although both have application source E826. The earlier full-scale musl A/A used `083bde706e142cede8d400b0c5c6639257c1bf1ca36f9d181b5e30180ac966bb`; its observed spread must not be presented as a fully binary-matched control for f04.

Raw locations below are relative to this directory:

- `representative-local-scale-native-2026-10-31.json.gz`: `artifacts["measurements/manifest.json"].provenance` records the common GNU binary and source. For v2, command prefix `measurements/0001-candidate-v2-7195248-keep1-r1-`; for v1, `measurements/0002-candidate-v1-7195248-keep1-r1-`. Suffixes `0-scan/` through `4-scan/` identify scans 1–5. Read `metrics.json`, `dbstat.json`, `stdout.json` under each command.
- `v1-cache4-canonical-candidate-2026-11-01.tar.gz`: `artifacts/scan/measurements/manifest.json`; `artifacts/scan/measurements/p{1,2,3}-{baseline,candidate}-v1/metrics.json`. All seven cited payloads were rehashed and matched their embedded `SHA256SUMS` entries.
- `v1-fullscan-memory-full-1024-aa-2026-11-01.tar.gz`: `artifacts/scan/measurements/p1-{baseline,candidate}-v1/metrics.json`. Both cited payloads matched the embedded index.
- `v1-refresh-capacity-{screen,full}-2026-10-31.tar.gz`: `artifacts/refresh/measurements/manifest.json`; each paired command's `metrics.json` and `allocation-profile.jsonl`. All 13 screen and five full-scale cited payloads matched their embedded indexes.

The gzip archive was parsed directly, including its raw observations; it is not merely a transcription of the rounded [representative report](../representative-performance-investigation.md). Tar index matching checks internal payload consistency; it does not independently establish an external trust root for the archive.

## V1 disk: retained copies, replacement and stage

At E826, `core/src/scan/staged.rs:25–40` owns a staging SQLite connection and removes its path on drop. `core/src/scan/staged/write.rs:44–102` consumes that object, reads it throughout output generation, closes/fsyncs the output, and then drops the stage before returning. `core/src/job.rs:338–352` publishes only after this call. Thus the current, previous, full replacement and stage **overlap in lifetime while the output is built**. This is source-level proof of overlapping representations, not a measurement of every representation's allocated bytes at one instant.

`core/src/snapshot/publish.rs:105–131` rotates by hardlinking current to `snapshot.prev.tmp` and renaming that link over previous, then renames the completed replacement over current. The temporary previous name aliases current's inode; it is not another copied database. A reader can keep an overwritten inode allocated after its last pathname disappears. The normal no-reader endpoint retains two generations. Likewise, `scan/denied.rs:44–69` creates and immediately unlinks a reporting spool held by a descriptor, so directory-only inventory is structurally incomplete even when a clean fixture gives that spool no allocated data.

Output index pages are already part of each output/current/previous database's size. Stage index pages are already part of stage size. They must not be counted a second time. The staged connections use journal OFF/temp FILE (`staged.rs:101–106`); indexes are maintained from empty output tables (`staged/write.rs:54–63`). A WAL/rollback-journal multiplier cannot be assigned to this path without observing such allocation. Precheck/reserve formulas are admission policies, not measured storage peaks.

| Archived observation | Baseline | Candidate/control |
| --- | ---: | ---: |
| Canonical 1m first-pair sampled app-disk metric | 438.449219 MiB (459,747,328 B) | 434.093750 MiB (455,180,288 B) |
| Canonical 1m stage allocated maximum | 72.199219 MiB (75,706,368 B) | 67.843750 MiB (71,139,328 B) |
| Canonical 1m replacement logical maximum | 122.078125 MiB (128,008,192 B) | Same |
| Canonical 1m initial retained snapshot allocation | 244.160156 MiB (256,020,480 B) | Same |
| Full-scale baseline A/A sampled app-disk metric | 3.126007 GiB (3,356,524,544 B) | 3.126015 GiB (3,356,532,736 B) |
| Full-scale baseline A/A stage allocated maximum | 529.988281 MiB (555,732,992 B) | Same |
| Full-scale baseline A/A each snapshot logical size | 890.320312 MiB (933,568,512 B) | Same |

In the canonical screen, all three baseline app-disk values are 459,747,328 B; candidate values are 455,180,288/455,180,288/455,184,384 B. Stage allocated maxima decrease by 4,567,040 B, or 4.355469 MiB, in every pair. This matches the first two app-metric changes exactly; the third differs by 4,096 B. The source removes redundant explicit rowid columns from stage indexes, and recorded output logical size is unchanged. This is strong evidence for a smaller temporary representation. Because 1af also changes cache and writer behavior, and the old receipts independently maximize components, it does not isolate an exact ordinary-run disk contribution from that one mechanism.

The app-disk metric is sampled filesystem used-byte growth relative to command start, plus initial snapshot allocation (`scripts/bench-v2-container.py:168–195,250–264`). Named inventory and filesystem sampling occur separately; filesystem metadata and open-deleted allocations affect the latter. Samples are lower bounds on true maxima. The [older 4 KiB audit](v1-disk-peak-audit-2026-10-31.md) additionally shows why equal absolute peaks can produce different growth metrics when their starts differ; it is a different source/seed/job and supplies no synchronized attribution for this screen.

Conditional ceilings from the **observed representation sizes**, holding all other allocations fixed:

- Removing the entire recorded baseline stage would remove at most 72.199 MiB at 1m, about 16.47% of its sampled app metric. The full-scale analogue is 529.988 MiB, about 16.56%. These are model-based ceilings against sampled values, not bounds on unsampled true peaks or promises that an implementation can remove all stage cost.
- Removing one retained generation corresponds roughly to 122.078 MiB at 1m or 890.320 MiB at full scale, about 27.8% of those sampled metrics. Dropping previous changes public history/diff semantics. Moreover, `keep_previous=false` removes an existing previous only after publication (`job.rs:349–352`); starting with the same two-generation seed does not remove its write-phase occupancy.
- Moving stage drop merely to “before publish” saves nothing: the source already drops it there. Reclaiming stage progressively or replacing the representation requires a different algorithm. Deleting stage rows can create reusable pages without returning filesystem blocks.

The original 7,202,464-entry GNU sequence is separate: its maximum v1 filesystem allocation above the initial arm baseline is 3,172,356,096 B (2.954487 GiB), and scan-five retained DB allocation is 1,779,265,536 B (1.657070 GiB). Those values cannot replace the 7,297,076-entry musl observations in the table.

## V2: repeated unchanged full scans grow history

At measured source `5b62c148`, `core/src/snapshot/v2/seed.rs:130–144` marks every open version dead, clears live tables, inserts the complete staged scan and commits. Memory staging loops over every entry (`seed.rs:79–83`); disk staging visits every entry (`seed.rs:109–114`, `disk.rs:78–87`). `v2/mod.rs:328–335` writes each inserted entry to both `live_rows` and `version_rows`. Stable identities are reused; there is no unchanged-value test that reuses a prior version. No history collection occurs inside that full publication transaction.

The raw `dbstat.json` under each original v2 scan has `databases["v2/generations.db"].row_counts`; the corresponding `metrics.json` has `endpoint_disk.db.allocated_bytes`:

| Completed unchanged full scan | Identities | Live rows | Version rows | Retained DB allocation |
| --- | ---: | ---: | ---: | ---: |
| 1 | 7,202,464 | 7,202,464 | 7,202,464 | 3,259,580,416 B / 3.035721 GiB |
| 2 | 7,202,464 | 7,202,464 | 14,404,928 | 4,025,524,224 B / 3.749062 GiB |
| 3 | 7,202,464 | 7,202,464 | 21,607,392 | 4,788,604,928 B / 4.459736 GiB |
| 4 | 7,202,464 | 7,202,464 | 28,809,856 | 5,549,383,680 B / 5.168266 GiB |
| 5 | 7,202,464 | 7,202,464 | 36,012,320 | 6,310,023,168 B / 5.876667 GiB |

This directly verifies N→5N versions with constant N live/identity rows and 2.840946 GiB retained growth. Raw page categories locate much of that growth: `version_rows` grows from 446.671875 to 2,224.906250 MiB; `versions_children` from 188.941406 to 941.523438 MiB; `versions_gc` from one 4 KiB page to 370.074219 MiB. Conversely, the partial `versions_open` index is 78.738281 MiB in scans 2–5. It is incorrect to multiply every index by retained generation count or assume the `dead IS NULL` retirement predicate scans all history.

Across the original v2 commands, the sampled filesystem maximum above the initial 264,269,824-byte arm baseline is 12,175,544,320 B (11.339359 GiB). The independently sampled named maximum is 10,481,229,824 B (9.761406 GiB). Their difference cannot be uniquely labelled as a deleted SQLite sort file: metadata, timing and unnamed allocation all remain possible contributors. WAL, database and stage maxima likewise cannot be summed as though simultaneous. Transaction times increase in this sequence, but row/index growth does not isolate the SQL operation responsible for the timing trend.

Refresh follows a different path. `v2/writer.rs:118–147` retires/replaces the target subtree, affected hardlink entries and ancestor aggregates. The same raw sequence provides a direct check: after scan five's 36,012,320 versions, the three unchanged **18-entry** refreshes have 36,012,339, 36,012,358 and 36,012,377 versions, increments of 19 each. Anchors are suffixes `5-refresh/dbstat.json`, `6-refresh/dbstat.json`, `7-refresh/dbstat.json` under the v2 command prefix above; their `stdout.json` records `subtree_entries=18`. Full-scan N-version creation therefore must not be attributed to every tiny refresh. Large subtrees, ancestor depth and hardlink groups can increase refresh work.

Collection and file shrink are distinct. `v2/retention.rs:189–208,265–307` protects current/previous visibility intervals and still-referenced identities; `v2/mod.rs:168–203` runs bounded explicit maintenance, with readers able to defer it and checkpoint work separate. In this clean five-full-scan example, with no additional reader pins, preserving current/previous permits removal of the three obsolete version sets: 21,607,392 rows, 60% of version rows. That is a **row-count** ceiling, not a disk percentage: live rows, identities, protected history, metadata and index widths differ. DELETE makes SQLite pages reusable; this code does not guarantee physical file truncation. Existing observations did not run a matched maintenance/shrink counterfactual, so reclaimed filesystem bytes and complete maintenance-cycle cost remain unmeasured.

## V1 whole-tree refresh: allocation strategy counterfactual

The baseline loads all current entries (`E826:core/src/refresh/load.rs:21–46,102–115`), then grafts the selected subtree by appending descendants (`core/src/refresh.rs:93–106`). Finalization and the writer still rebuild the full snapshot. A small refresh target therefore does not imply a small in-memory tree or a small output write.

The f04 change rescans the subtree first, computes its extra descendant count, checks capacity and requests exact node headroom before loading (`f04:core/src/refresh/load.rs:24–44,70–99,175–180`). It still loads the whole tree and preserves the full replacement publication. This is a counterfactual for the **pre-sizing and preparation-order strategy**, not a storage-engine change or an isolated allocator implementation swap.

The raw screen traces show baseline node capacity 1,004,499→2,008,998 with a buffer-pointer change in every pair. Candidate capacity stays 1,004,598, with no pointer change through grafting; the additional 99 slots match the target descendants. Between `refresh_after_names` and `refresh_after_nodes`, baseline RSS rises 90,116 KiB (88.003906 MiB) in each pair; candidate rises zero. GNU-time median process peaks are 369.085938 MiB baseline and 280.933594 MiB candidate. These point samples and independent process peaks establish an observed strategy effect in those pairs, not exact phase maxima or a minimum usable RAM budget.

The full-scale raw pair provides the same mechanism at larger N:

| Full-scale unchanged 100-entry refresh | Baseline E826 | Exact-capacity f04 |
| --- | ---: | ---: |
| Node capacity after load | 7,297,076 | 7,297,175 |
| Node capacity after graft append | 14,594,152, pointer changed | 7,297,175, pointer unchanged |
| RSS after names → after nodes | 1,165,312 → 1,798,148 KiB | 1,165,256 → 1,165,256 KiB |
| GNU-time process peak | 1,806,980 KiB / 1.723270 GiB | 1,231,700 KiB / 1.174641 GiB |
| Worker-cgroup peak | 2,615,136,256 B / 2.435535 GiB | 2,427,670,528 B / 2.260944 GiB |
| Complete wall time | 74.188183195 s | 74.585005117 s |

The process-peak reduction is 561.796875 MiB (31.836545%); the distinct cgroup-peak reduction is only 178.781250 MiB (7.168488%). Do not describe the RSS difference as the total-RAM saving. The independently observed peaks do not identify which charges overlap. Pointer movement and nominal vector capacities also do not, by themselves, quantify physical overlap, copied resident pages or allocator-retained pages.

Raw anchors are `artifacts/refresh/measurements/p1-{baseline,candidate}-v1/{allocation-profile.jsonl,metrics.json}` in the full-scale capacity archive. The profile members' SHA-256 values are respectively `dd6dd9523430995c10178c85489e55356bc4ae9ff8f080ce1b99293b416cf77b` and `10cec9686a3708deae7cb6f574d8e7aadf3035227992d1b7b83d5db616f5f39b`. Both matched their embedded index entries during this audit.

The original VmHWM monotonicity contract fails: full-scale traces decrease by 268 and 68 KiB respectively, and the screen has seven decreases across six traces. Profile-v2 structural acceptance preserves those readings; it does not repair the failed original monotonic contract. GNU-time peaks, pointwise RSS and reported `/proc` HWM are therefore labelled separately. Only the 4 GiB cap was tested in this refresh comparison. One full-scale pair, a positive wall-time change, older control binary differences and these diagnostic limits prevent a no-regression or product-acceptance conclusion. These observations cannot be added to the later 1af candidate's improvements or used to predict the installed binary.

The pending cause experiments must supply their own source/binary identities, diagnostics and measured intervention receipts. Prior representation evidence supports hypotheses and bounds their plausible scope; it does not supply missing results for those experiments.
