# V1 staged full scan: total-memory audit

## Scope and measured evidence

The requested joint source is `4591856f7e1f66a00a8ef70874fb1046fa8c428d`. The saved one-pair full SQL run [41230966330](https://github.com/mikhailsal/free-space/actions/runs/41230966330) actually measures baseline `e8265778ef78ec4acd9da1375eedb0be37a123f5` against candidate `d1c9f014ed185d663947590b388b444c08da7fda`, **not** joint `4591856`. `git diff d1c9f014 4591856 -- core/src/scan/staged/write.rs core/src/scan/staged/write/batch.rs` shows actual batching differences, including ancestor eviction/refill and budget logic. Both use the staged architecture described below, but the run does not establish `4591856`'s exact memory, time, or parity. Its successful ordinary validation applies to the measured pair only.

The full run has 7,264,383 fixture files / 7,297,076 entries, one worker, two cgroup CPUs, no swap, 4,096 MiB `memory.max`, cold cache before each isolated worker and separately cgrouped observer. Loop-device caps were 32 MiB/s read and write, 2,200 read IOPS and 3,600 write IOPS. The worker is `/binary --backend v1 scan /scratch/tree --workers 1 --json --low-memory`. Measurements and interpretation below come from `/tmp/freespace-combined-sql-full-description.json` and both raw `scan/measurements/p1-{baseline,candidate}-v1/{spec,metrics}.json` receipts.

| Worker-cgroup or CLI measure | Baseline | Measured candidate d1c9 | Meaning |
| --- | ---: | ---: | --- |
| GNU time CLI max RSS | 35,272 KiB (34.445 MiB) | 35,052 KiB (34.230 MiB) | Process RSS, not total cgroup RAM |
| `memory.peak` | 4,294,967,296 B | 4,294,967,296 B | Both hit the configured 4 GiB cgroup ceiling |
| 20ms sampled `memory.current` peak | 4,294,967,296 B | 4,294,967,296 B | Includes CLI, time and sampler; observer excluded |
| `memory.events` max delta | 15,297 | 15,525 | Repeated boundary pressure, not distinct OOMs or a duration |
| oom / oom_kill delta | 0 / 0 | 0 / 0 | Both commands completed; no OOM evidence at this cap |
| Complete wall time | 639.785 s | 477.140 s | One-pair descriptive timing, not an independent RAM result |

These receipts show a small process-RSS difference, not a demonstrated total-memory reduction. The flat peak is ceiling-censored: two workloads both reading 4 GiB at a 4 GiB limit do not establish equal unconstrained demand or required minimum. The high `max` counters and successful no-OOM exit establish repeated reclaim/limit pressure with successful execution at this setting, not that 4 GiB is necessary. Only the 4 GiB budget was tested successfully for these full-scale commands. It is the lowest tested success so far, with no lower-budget boundary established; neither 34 MiB RSS nor 4 GiB is a demonstrated required minimum.

### Exact post-command endpoints, not peak decomposition

`cgroup_after` is read after the timed command exits while the worker/helper remains alive. These are endpoint `memory.stat` values, **not** attribution at the earlier 4 GiB peak. Values overlap by definition: `slab_reclaimable` is part of `kernel`, and `inactive_file`/`file_mapped` are parts of `file`; do not add them as independent categories.

| Endpoint field | Baseline bytes | Candidate d1c9 bytes |
| --- | ---: | ---: |
| `memory.current` | 3,710,185,472 | 3,716,202,496 |
| `anon` | 8,556,544 | 8,556,544 |
| `file` | 1,371,611,136 | 1,319,223,296 |
| `kernel` | 2,329,579,520 | 2,388,242,432 |
| `slab_reclaimable` | 2,329,144,336 | 2,387,856,440 |
| `slab_unreclaimable` | 226,656 | 230,448 |
| `inactive_file` | 1,360,404,480 | 1,308,069,888 |
| `file_mapped` | 3,088,384 | 2,695,168 |
| `file_dirty` | 45,056 | 12,288 |
| `file_writeback` | 0 | 0 |
| `pgscan` | 913,579 | 924,037 |
| `pgsteal` | 913,491 | 923,945 |
| `workingset_refault_file` | 465,733 | 461,215 |

Before the command, `memory.current` was 27,582,464 / 27,308,032 B, `file` 17,604,608 / 17,379,328 B, and `kernel` 954,368 / 950,272 B, baseline/candidate respectively. End-of-command memory pressure `total` was 5,219,440 / 4,956,433 microseconds for both `some` and `full`; this is cumulative stall accounting, not exact peak attribution. The large endpoint reclaimable slab and file-cache charges motivate examining filesystem metadata and page-cache working sets. They do not prove which inode, process, scan phase or allocation caused the peak. Reclaimable does not mean free or instantaneously reclaimable without I/O/latency, and cgroup accounting/ownership is not the same as private CLI RSS.

Sampled output receipts show both arms' staging file peaked at 555,778,048 allocated B and final snapshot at about 936 MB allocated; the measured operation also has the fixture, prior/current snapshots, SQLite reads/writes, and filesystem metadata. `file` cannot be assigned to stage versus snapshot or fixture from these endpoint fields. Nor does `slab_reclaimable` isolate dentries/inodes from all other reclaimable kernel slabs. The harness sampler calls `occupancy`/`file_occupancy` and `statvfs` on the snapshot tree every 20 ms and shares the worker cgroup; those checks can themselves access filesystem metadata. No receipt separates their contribution from the application's, so identify and measure that potential observer effect rather than assuming it is negligible.

## Actual allocation paths

At `4591856`, `core/src/job.rs::walk_scan` uses `StagedTree::walk` for the benchmark's `--low-memory` scan and writes through `ScanData::Staged`; it does not retain the full `Tree.nodes`/`names` in RAM. Stage jobs, raw nodes, indexes, links and mount information are in SQLite. `core/src/scan/staged/walk.rs::read_directory` streams `read_dir` entries into the stage, unlike the ordinary in-memory walker `core/src/scan/walk/read.rs`, which collects and sorts a directory in memory. A current path, metadata, root/mount lists and DFS stack remain; bounds are not a hard total RSS guarantee, particularly for unusually deep paths, root sets or denied-path output.

Both staged SQLite connections use `core/src/scan/staged.rs::configure_database`: `cache_size=-8192` (8 MiB target **per connection**, not a hard cap on all SQLite allocations), `temp_store=FILE`, `mmap_size=0`, `cache_spill=ON`, `threads=0`, journal off, exclusive locking. A stage connection stays open while the output connection is built through the **same staged `open_database` configuration**; this is not the separate in-memory snapshot writer's `cache_size=-65536; temp_store=MEMORY` in `core/src/snapshot/writer.rs::open_snapshot`, and the reader's 32 MiB cache/256 MiB mmap pragma is not evidence about these timed staged writer connections. The source sets file-backed temporary storage, but it does not prove a particular SQLite temp file was used or account for all kernel page cache. Indexes are maintained incrementally from empty output tables; the output is one transaction followed by `ANALYZE`, close and fsync, with reserve guards. A 0 `finalize_seconds` reflects finalization fused into staged write; do not compare phase labels as if the work vanished.

`4591856`'s `core/src/scan/staged/write/batch.rs` uses batches of at most 32 and a shared extra-pending budget of 256 slots / 64 KiB over active DFS frames, with fallback to single-row retrieval. Its code explicitly excludes baseline path/cursor/DFS allocations, SQLite caches, in-flight row, allocator overhead and process RSS from that budget. The measured `d1c9f014` also declares 32/256/64 KiB but differs in eviction/headroom implementation. These bounds describe **specific application buffers**, not the kernel file/slab working set and not a total-memory floor. The measured binary uses musl's configured allocator path; allocations, SQLite's page cache, threads/stacks, `/usr/bin/time`, and Python sampler can be cgroup-charged. No source evidence makes their endpoint `anon` a process peak. Temporary file paths/guards, deny spool, output fsync, reserve and atomic publish are part of the full command, not removable measurement noise.

## Smallest discriminating investigation

1. Use the already planned **1m fixture at 512 / 1,024 / 2,048 / 4,096 MiB**, fixed case/seed/history, no swap, same CPU/I/O, isolated fresh worker cgroups and 4 GiB observer. Run both exact compared binaries under a preregistered order and failure policy; record each attempt, process exit, memory.max/swap, local and hierarchical events, time, RSS, cgroup peak, PSI/reclaim and full semantic/integrity/current+previous observer results. Stop or label any timeout/OOM/space/observer failure honestly. A successful lower cap characterizes a tested budget, not an optimization or a theoretical minimum. Lower limit alone forces reclaim; it is a **capacity/pressure sensitivity experiment**, not evidence of fewer bytes demanded by code.
2. To attribute a real improvement, add a low-overhead, explicitly frozen **cgroup time series** in a controlled diagnostic: timestamped `memory.current`, `memory.stat` (`anon`, `file`, `kernel`, `slab_reclaimable`, `slab_unreclaimable`, active/inactive file, dirty/writeback, workingset refault, pgscan/pgsteal), `memory.events.local`, memory/IO PSI and phase markers for walk/insert/index/sync; retain raw before/after, external `memory.peak` and sample cadence/missed/error counts. Use a modest fixed cadence and compare instrumented versus uninstrumented controls for overhead. `memory.peak` remains whole-cgroup; sample correlations cannot prove an individual allocation's cause. Record stage/output allocated bytes and disk I/O in the same window. If kernel exposes per-cgroup dentry/inode slab breakdown, use it; otherwise host `slabinfo` is host-global and cannot by itself attribute slab to the worker. Do not drop shared caches mid-run or use unapproved tracing as if it were free.
3. Use these data to rank falsifiable hypotheses: (a) stage/output page-cache working set drives `file` and reclaim, suggested only if file rises during those writes and correlates with I/O/refault; (b) directory/inode metadata drives reclaimable slab, supported only by per-cgroup metadata evidence or a well-controlled fixture-shape comparison at matched entry/payload count; (c) harness snapshot traversal materially contributes, tested with a separately preregistered low-overhead sampler comparison, identical engine, cgroup, seed and output semantics. One cannot subtract process RSS from cgroup peak and call the rest "waste," nor call a lower cap a source optimization.
4. Select the lowest promising **tested** 1m rung and the next rung for the existing full-scale check, under its admission and predeclared stop rules. Use the exact `4591856` binary/target and matching semantic oracle, not `d1c9` as a proxy; keep artifacts and disk occupancy and consider that fixture shape, file cache and reclaim can scale differently. A genuine total-RAM optimization needs a source change with matched baseline/candidate measurements at the same cap and/or unconstrained conditions, semantic/durability preservation, disk/time gates and repeatable memory/pressure evidence; capacity success must be demonstrated separately under the applicable D11 confirmation protocol. Do not increase SQLite cache budgets, add a public tuning flag or introduce another storage engine to manufacture a conclusion.

## Invariants and open questions

Preserve D3/D4/D5 freshness/mtime semantics, stable traversal IDs and raw bytes, hardlink/mount/denied behavior, same current/previous history, integrity and atomic publish/reserve. Whether the large reclaimable slab is predominantly fixture dentries/inodes, stage/output activity, sampler walks, or other kernel metadata is not shown. Whether file-backed cache is mostly useful working set or avoidable churn is not shown. Whether `4591856` succeeds at any lower rung, and its minimum required total cgroup memory, remain untested. The outcome can be slower at a low cap despite successful completion; report CPU/I/O/PSI and complete-command wall time alongside memory and disk.
