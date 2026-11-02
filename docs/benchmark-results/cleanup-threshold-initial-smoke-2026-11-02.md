# Cleanup threshold: initial 4 GiB pilot

Date: 2026-11-02. This is a successful protocol pilot, **one comparison per case**, not a million-entry result or a production qualification. The prospective 8 GiB follow-up is a separate protocol and must not be pooled with these observations.

Run [41476363033](https://github.com/mikhailsal/free-space/actions/runs/41476363033), source `d718f11fd3e64e7c8874c49d575d60067a0c9aee`, 724 aggregate runner seconds. Both product controls use the unchanged master implementation (`afce6fdfc007a2c6914cab61aab4d84daccbb99b`) in this source tree. 16 Python tests and 318 Rust tests passed; one Rust test was ignored. All 30 writer commands, six independent oracles and cleanup passed. No cap/reclaim flags were observed.

Each profile contains 100,276 actual entries on a 500 GiB logical ext4 loop device. The directory profile contains 50,107 directories and 50,169 regular files; the files profile contains 107 directories and 100,169 regular files. Fixed controls include files with real allocated bytes around the threshold, a sparse apparent-only file, 64 individually subthreshold files that collectively exceed the threshold, deep ancestors and a raw non-UTF-8 name.

The actual filesystem capacity was 536,166,531,072 bytes and threshold 536,166,532 bytes (0.1%). Four preallocated controls occupied 2,144,686,080 inner bytes while growing outer allocation by only 98,304 bytes in the files profile and 86,016 bytes in the directories profile. These are real inner `st_blocks`, not apparent-only lengths. The outer backing high-water mark across the pilot was 1,079,255,040 bytes; minimum outer/results headroom was 87,527,833,600 bytes.

Every arm starts from its own two-full-scan seed. Cases are unchanged trees, with cold-cache preparation, 2 CPUs, 4 GiB/no swap, the same 32 MiB/s and 2,200/3,600 IOPS envelope, and a fresh private writer cgroup. Prototype time includes both metadata passes. Observers and setup are outside writer time/RAM but inside job accounting.

| Profile | Operation | Arm | Wall s | Process RSS MiB | Total cgroup peak MiB | Sampled output peak lower bound MiB | Final snapshot allocation MiB |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| files | full | v1 | 4.8771 | 36.910 | 242.895 | 42.95312 | 23.97266 |
| files | full | v2 | 11.0420 | 37.000 | 334.441 | 111.65625 | 62.25391 |
| files | full | probe | 3.5228 | 8.836 | 192.543 | 0.03125 | 0.03125 |
| files | retained | v1 | 1.1672 | 59.098 | 99.164 | 35.33984 | 23.34766 |
| files | retained | v2 | 0.1652 | 14.953 | 45.094 | 51.98438 | 51.82812 |
| files | retained | probe | 0.0649 | 8.863 | 36.312 | 0.03125 | 0.03125 |
| files | omitted | v1 | 1.1672 | 58.957 | 99.176 | 35.33984 | 23.34766 |
| files | omitted | v2 | 0.1651 | 15.020 | 44.812 | 51.83203 | 51.82812 |
| files | omitted | probe | 3.5232 | 8.816 | 192.070 | 0.03125 | 0.03125 |
| directories | full | v1 | 27.9331 | 36.699 | 447.664 | 43.08984 | 23.96484 |
| directories | full | v2 | 33.7413 | 36.785 | 537.383 | 111.82031 | 62.23438 |
| directories | full | probe | 26.5768 | 8.836 | 396.672 | 0.03125 | 0.03125 |
| directories | retained | v1 | 1.1678 | 59.125 | 99.434 | 35.32812 | 23.33984 |
| directories | retained | v2 | 0.1656 | 14.965 | 45.316 | 51.78906 | 51.78516 |
| directories | retained | probe | 0.0654 | 8.934 | 36.273 | 0.03125 | 0.03125 |
| directories | omitted | v1 | 1.1678 | 59.117 | 99.199 | 35.32812 | 23.33984 |
| directories | omitted | v2 | 0.1658 | 14.992 | 45.070 | 51.78906 | 51.78516 |
| directories | omitted | probe | 26.5757 | 8.926 | 396.516 | 0.03125 | 0.03125 |

`retained` refresh rescans the 74-entry `large` subtree (148 prototype metadata visits). `omitted` requests the 100-entry `small` subtree: v1/v2 scan those 100 entries; the prototype has discarded its old contribution and rescans the entire root (200,552 visits). Exact work scopes and final totals were checked, not inferred from a successful exit.

The prototype retained seven named nodes per profile. Its two final coarse database images occupy 32 KiB combined. That dramatic storage reduction deliberately discards small-object identities; it is not lossless compression or an isolated SQLite change. Full-scan total RAM fell from 242.895 to 192.543 MiB in the files case and 447.664 to 396.672 MiB in the directory case. Full time was 4.8771→3.5228 s and 27.9331→26.5768 s respectively. A single directory comparison does not establish a reliable 5% speed improvement.

The major counterexample is refresh of an omitted target. In the directory case it took 26.5757 s versus 1.1678 s for v1 and 0.1658 s for v2: approximately 23×/160× slower. The information loss therefore changes the update contract materially, even though refreshing a retained candidate is faster in this pilot. It cannot be called a universal improvement across the four metrics.

Peak output is sampled every 50 ms: initial snapshot allocation plus maximum observed inner filesystem growth. It includes filesystem metadata/open-unlinked output and excludes the initial fixture. It is a lower bound, particularly for short coarse publication; individual file maxima are not summed. Total memory includes the writer wrapper, page cache and kernel memory, separately from command RSS. History semantics also differ: v1 preserves its previous full baseline during refresh, while this research example saves its immediately preceding coarse generation. No production API/recovery equivalence is established.

The original conditional screen was **not dispatched**. Its prospective host projection was 7,574.33 s, 9,846.63 s after 30% margin, which fails the original 5,100 s step/7,200 s job caps. The space projection passed. The rejection remains recorded; a new 8 GiB/4-hour-ceiling follow-up requires a fresh reviewed source, smoke and admission.

Evidence: [strict analysis JSON](cleanup-threshold-initial-smoke-analysis-2026-11-02.json), [raw selected artifact capsule](cleanup-threshold-initial-smoke-2026-11-02.tar.gz), [original screen rejection](cleanup-threshold-original-screen-rejection-2026-11-02.json), [source review and freeze](cleanup-threshold-source-freeze-2026-11-02.json), [nine product integration work units](cleanup-threshold-integration-audit-2026-11-02.md). The capsule preserves raw metrics, source/provenance, oracles, Actions job data, logs and a hash index. ZIP members were retrieved with HTTP ranges and CRC checks; large image/binary/snapshot payloads were omitted locally and their inventory is preserved. Runtime image identity/package metadata is checked; the image itself was not loaded or byte-verified locally.
