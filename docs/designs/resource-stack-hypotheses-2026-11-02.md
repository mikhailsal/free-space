# Resource hypotheses, including storage-stack changes

Date: 2026-11-02. Status: research proposals only; no implementation, benchmark,
qualification, backend selection or promotion is claimed by this document.

## Workload and causal boundary

The [v1 plan](v1-performance-plan.md) and
[progress report](../benchmark-results/v1-optimization-progress-2026-10-31.md)
remain authoritative for accepted gates and experimental state. Their full-size
fixture has 7,297,076 entries. The accepted goal concerns peak allocated disk,
total RAM, full-scan time and folder-refresh time together. A low process RSS
alone does not meet the total-RAM goal. In the existing 1m composition trace,
reclaimable slab and file cache dominated the sampled anonymous memory; this
was neither a decomposition of the actual peak nor attribution specifically to
dentries. The canonical 4 MiB candidate remains held by inconclusive H2.

Local source anchors, read for this research:

* [schema](../../core/src/snapshot/schema.rs): dense DFS preorder `id`, inclusive
  `subtree_end`, raw BLOB names, stored aggregates, hardlink rows and five
  secondary indexes. `INTEGER PRIMARY KEY` already uses SQLite's rowid key.
* [refresh](../../core/src/refresh.rs): load published tree, graft replacement,
  regenerate preorder, resolve hardlink ownership, recompute the fold and write
  a new snapshot. Tiny refreshed subtrees therefore still incur whole-tree work.
* [writer](../../core/src/snapshot/writer.rs) and
  [publish](../../core/src/snapshot/publish.rs): fresh file, disposable unpublished
  output, sync, current/previous publication protocol. Fresh-file construction
  already differs from updating a live journaled database.
* [disk staging](../../core/src/scan/staged.rs): raw rows and work queues avoid
  retaining the full full-scan tree in the Rust heap, but consume staging disk
  and cached file pages.
* [v2 schema](../../core/src/snapshot/v2/schema.rs): identities, live rows,
  version rows, retention and GC. Versioning is not synonymous with sparse
  change storage: rewriting/versioning every observed entry on an unchanged
  scan still produces O(N) new versions. A new engine with the same write set
  would preserve that cause of growth.

Dense DFS IDs are an especially important constraint. Inserting entries near the
front changes later IDs, parent references, subtree ranges and index references.
Replacing the engine while preserving this physical layout cannot promise
sublinear refresh. Stable identity plus a separate ordering/query representation
would be a semantic-storage design change requiring its own correctness proof.
Hardlinks can cross the refreshed boundary; changed ownership affects other
paths and ancestors. Removing a former maximum/minimum requires examining the
remaining contributing children or maintaining an exact supporting index.
Neither work is bounded solely by the refreshed entry count.

## Primary evidence and comparison matrix

Links below are primary project documentation, project source, or Linux manual
pages maintained by the Linux man-pages project. They establish mechanisms,
not measured results for free-space. All resource improvements are hypotheses.
No external benchmark numbers are imported.

| Alternative | Peak disk, amplification and retention | Resident RAM, mappings and heap | Full-scan throughput | Small folder refresh, aggregates and hardlinks |
| --- | --- | --- | --- | --- |
| Improved SQLite layout/transaction strategy | Keep fresh immutable output as the low-change control; inspect duplicate stage/final indexes and lifetime overlap before adding live WAL. WAL adds changed-page frames and checkpoints; readers and a large transaction can prevent reset. Deletion may leave reusable free pages; VACUUM adds a rebuild peak. [WAL](https://www.sqlite.org/wal.html), [VACUUM](https://www.sqlite.org/lang_vacuum.html). | Bounded SQLite cache does not bound OS file cache or traversal slab. Hypothesis: reduce overlapping cached stage/final data and application structures, not just cache size. [temporary storage](https://www.sqlite.org/tempfiles.html), [cgroup accounting](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html). | Existing writer already bulk-inserts then builds indexes. Hypothesis: one narrowly justified staging/layout change reduces redundant writes or index build work; timing must include final indexes and publish. No generic claim that WAL speeds this fresh-file path. [SQLite file format](https://www.sqlite.org/fileformat.html). | Retaining current dense IDs and fold leaves O(N). Sparse updates need stable identity/order indirection and exact aggregate/inode indexes; compare that algorithm in SQLite before crediting an engine switch. `WITHOUT ROWID` is a possible composite-key tool, not a blanket optimization for current integer-key entries. [layout guidance](https://www.sqlite.org/withoutrowid.html). |
| LMDB via heed, mmap copy-on-write B+tree | Free pages are reused, but readers prevent reuse and can cause growth. Compacting copy needs another output and does not eliminate its source. Holding a transaction open is not a durable previous-generation format across process exits; explicit persisted previous state/copy needs budgeting. [LMDB API/source](https://raw.githubusercontent.com/LMDB/lmdb/mdb.master/libraries/liblmdb/lmdb.h). | Reads expose mapped bytes without a private database page cache. This may reduce decoding/copy allocations; mapped virtual size is not resident size. Accessed file pages still consume RAM, and write transactions need dirty-page bookkeeping. heed wraps LMDB, rather than changing its storage mechanics. [LMDB](https://raw.githubusercontent.com/LMDB/lmdb/mdb.master/libraries/liblmdb/lmdb.h), [heed](https://docs.rs/heed/latest/heed/), [Linux accounting](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html). | Ordered bulk load can use `MDB_APPEND` only when keys are correctly ordered. Extra hardlink/order/aggregate indexes have their own sorts/writes; single writer and large transaction size are constraints. No throughput advantage is established. [LMDB API/source](https://raw.githubusercontent.com/LMDB/lmdb/mdb.master/libraries/liblmdb/lmdb.h). | Hypothesis: with stable keys, local updates copy affected B+tree paths rather than rewrite the full dataset. Dense-ID renumbering defeats this. Exact ancestor extrema and cross-subtree hardlink owners still need queries/index maintenance. COW history held for readers raises peak disk; it is not free retention. [LMDB transaction and reader caveats](https://raw.githubusercontent.com/LMDB/lmdb/mdb.master/libraries/liblmdb/lmdb.h). |
| RocksDB, LSM | WAL, flushes, SST levels and compaction create write/space amplification. Snapshots preserve visible versions; ordinary snapshots do not survive restart. Durable checkpoints can hardlink SSTs on the same filesystem, thereby pinning old files even after live compaction. Count unique allocated blocks, not directory-size sums. [tuning](https://github.com/facebook/rocksdb/wiki/RocksDB-Tuning-Guide), [snapshots](https://github.com/facebook/rocksdb/wiki/Snapshot), [checkpoints](https://github.com/facebook/rocksdb/wiki/Checkpoints). | Budget memtables, block cache, indexes, filters and iterator-pinned blocks together. Buffered I/O also uses OS file cache. A small block cache alone is not a process or total-RAM bound. [memory documentation](https://github.com/facebook/rocksdb/wiki/Memory-usage-in-RocksDB). | Sequential write buffering is a plausible match for ingestion; external SST creation/ingestion is another bulk path, with sorting, index generation and deferred compaction included in cost. Compaction competes with traversal and publication. [SST ingestion](https://github.com/facebook/rocksdb/wiki/Creating-and-Ingesting-SST-files), [tuning](https://github.com/facebook/rocksdb/wiki/RocksDB-Tuning-Guide). | Stable keys permit a small logical write batch, but secondary indexes and ownership changes multiply writes. Tombstones and obsolete versions can survive until compaction/readers permit removal. Dense IDs still produce a large logical write set. [snapshot-aware compaction](https://github.com/facebook/rocksdb/wiki/Snapshot), [tuning](https://github.com/facebook/rocksdb/wiki/RocksDB-Tuning-Guide). |
| Packed immutable base plus bounded delta and streaming compaction | **Proposed representation, not an existing engine's guarantee.** A packed base may avoid generic row/index overhead; current/previous references can share unchanged immutable segments. Delta accumulation and merging require old base + delta + new base + retained reader/previous data concurrently. An unchanged full scan must not append N versions merely because it observed N rows. [atomic rename](https://man7.org/linux/man-pages/man2/rename.2.html), [durability primitives](https://man7.org/linux/man-pages/man2/fsync.2.html) support publication mechanics only. | **Hypothesis:** stream records/names and use sparse on-disk indexes to avoid a full tree or full hash index in heap. mmap still faults file pages into RAM; delta merge indexes may erase the saving. [Linux memory categories](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html). | **Hypothesis:** sequential pack/merge reduces random index traffic. It still needs traversal, ordering, hardlink attribution, folds and query indexes. Compression, if proposed later, exchanges bytes for CPU and must have its own frozen setting. No speed estimate exists. | **Hypothesis:** delta can store only changed stable identities and affected indexes. Query overlay, deletions, reordering and exact extrema introduce new logic; dense preorder cannot remain a physical offset with cheap arbitrary insertion. Requiring O(N) compatibility export on every refresh would forfeit the proposed update saving. Compaction makes small-update latency periodically large unless charged explicitly. |

The four rows are alternatives, not four approved implementation tracks. redb
or fjall would be additional implementations of related storage mechanisms;
adding them now would enlarge the experiment matrix without resolving the key
stable-ID, write-set and kernel-memory questions. They are not evaluated here.

## Causal hypotheses and falsifiers

1. **SQLite first: unnecessary layout/lifetime overlap dominates avoidable
   disk.** Proposed intervention: select one observed duplicate intermediate
   structure or excessive lifetime and remove it while keeping the existing
   semantics and storage engine. Record per-file allocation and phase writes.
   Falsifier: the structure is insignificant at the peak, or removing it moves
   equal/larger cost into sort/index temporary files, reads or regeneration.
   The current `entries` integer key already aliases rowid; SQLite explicitly
   advises ordinary rowid tables for this case. A blanket `WITHOUT ROWID`
   conversion lacks a mechanism. [SQLite's key/layout explanation](https://www.sqlite.org/withoutrowid.html).
2. **The refresh algorithm, rather than SQLite itself, dominates tiny updates.**
   Proposed intervention: express the same minimal stable-key update model in
   SQLite and one replacement engine, keeping equivalent indexes and readers.
   Falsifier for an engine-specific claim: both improve similarly, or the
   replacement still rewrites the dense-ID suffix/full fold. This comparison
   distinguishes an algorithm win from an engine win. It does not preapprove
   replacing the current query model.
3. **LMDB removes enough application-side copying/cache duplication to help
   refresh.** Falsifier: file-backed residency plus dirty pages/retained pages
   offsets heap savings, or total-cgroup/capacity evidence fails despite lower
   allocations. Audit borrowed mapped-value lifetimes and heed decoding, rather
   than treating all application reads as zero-copy. LMDB's COW and reader
   reuse limitations are documented in its [own API header](https://raw.githubusercontent.com/LMDB/lmdb/mdb.master/libraries/liblmdb/lmdb.h).
4. **RocksDB amortizes actual sparse writes sufficiently to offset compaction.**
   Falsifier: post-refresh compaction/retained checkpoint files breach disk or
   RAM guards, or complete full scans/queries slow beyond accepted gates. A run
   ending before deferred work is charged cannot establish this hypothesis.
   [Official tuning](https://github.com/facebook/rocksdb/wiki/RocksDB-Tuning-Guide)
   describes the amplification tradeoffs, not a free-space performance result.
5. **Packed data saves enough representation bytes to justify custom logic.**
   Falsifier: equivalent query/inode/aggregate/order indexes, delta lookup and a
   forced merge remove the saving, or restart/reader/ENOSPC behavior cannot be
   made equivalent in a bounded prototype. A compact flat export without a
   usable refresh/query path is only an encoding measurement.

## What stack and language changes cannot establish

Linux accounts filesystem cache and kernel structures separately from anonymous
memory. Its `slab_reclaimable` includes reclaimable structures such as dentries
and inodes; mmap does not remove those traversal effects. [Kernel cgroup
documentation](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html).
Thus no database replacement alone explains how to remove the observed dominant
scan-time categories. A matched traversal-only diagnostic could estimate a
boundary, but it must preserve equivalent filesystem visits and cannot stand in
for the complete product benchmark. Any cache-control experiment needs separate
authorization and must not change only one arm's cache state.

An allocator intervention is plausible only after live allocation/retained
capacity evidence identifies a material anonymous-memory cause. The project is
already Rust; a language rewrite does not change the filesystem metadata visits,
the physical write set, required indexes, retained generations or cgroup cache
accounting by itself. There is no cited workload-specific evidence for a
language/allocator rewrite benefit, so none is recommended as the first
prototype. This is an evidence limit, not a claim that allocators never matter.

Atomic visibility and durability are separate requirements. Rename atomically
replaces a name while existing open descriptors remain usable; syncing file
contents alone does not necessarily persist the containing directory entry.
[rename manual](https://man7.org/linux/man-pages/man2/rename.2.html),
[fsync manual](https://man7.org/linux/man-pages/man2/fsync.2.html).
An engine transaction is not by itself proof of the application's whole
current/previous, mount, progress and error-state publication protocol.

## Minimal fair GitHub prototype proposal

This research authorizes no dispatch or code change. Before implementation,
choose **one** question and one challenger; freeze a short manifest, source
hashes, exact engine versions/options, admission envelope and stop rules. Keep
the existing H2 and source/promotion gates unchanged. Do not launch the Cartesian
product of engines, allocators, cache sizes, compression settings and workloads.

Proposed first question: can stable-key sparse refresh remove whole-tree work
without worsening peak disk or total memory? Start with SQLite as the algorithm
control; only admit LMDB as one challenger if the control leaves a specific
engine cost to test. If stable identity/order compatibility is too large for a
bounded prototype, first measure packed encoding or stage lifetime in isolation,
explicitly limiting the conclusion to that mechanism.

Proposed stages, each conditional on the prior stage and separately admitted:

1. A disposable GitHub smoke with the existing small fixture scale. Validate
   current/previous after a full scan and multiple refreshes; raw non-UTF-8
   names, root order, rename/delete, denied paths, mount handling, cross-subtree
   hardlinks and loss of extrema must remain equivalent. Include a held reader
   across publication, process restart, writer interruption and ENOSPC. Stop on
   semantic or recovery failure; do not performance-screen an incomplete model.
2. A fresh matched A/A calibration and bounded paired 1m screen. Same fixture,
   traversal, compiler/profile, worker count, durability, current/previous policy,
   query indexes and cgroup limits. Include complete unchanged scan, unchanged
   100-entry refresh and changed subtree refresh, selected before execution.
   Observe current and previous with the independent existing semantics oracle
   or an explicitly reviewed equivalent adapter. A serializer microbenchmark
   can diagnose a phase; it cannot claim end-to-end superiority.
3. Only after a valid promising screen: separately propose the 7.3m case and
   matched lower-memory cases within the authoritative ledger. There is no
   inferred reservation or authority for them in this document.

Record complete command and phase wall time; process RSS and mapping breakdown;
worker-cgroup peak/current/stat/events, PSI, swap and OOM; per-file logical size
and allocated blocks, unique-inode/block accounting for links; all staging,
WAL/log, temporary indexes, old reader and previous-generation allocations;
physical I/O counters where supported. Time flushing/checkpointing/compaction
through a preregistered quiescence criterion and report command latency separately
from amortized maintenance. Force at least one bounded merge/compaction in a
retention sequence rather than hiding it after the timed interval. Do not sum
overlapping kernel counters or infer the composition of an earlier peak from
post-command values. A successful low-cap run is a tested capacity point, not
the exact minimum.

Decision bars remain those accepted in the v1 plan: no confirmed disk/RSS
regression or higher minimum tested successful budget; no confirmed refresh
slowdown per scenario; speed-only benefit at least 15%; disk/RSS benefit at
least 25%; capacity alternative at least 25% lower repeatedly successful budget
with matched baseline OOM evidence. Any allowed full-scan slowdown is at most
15%, with at least twice its percentage improvement in the other metric and,
for a RAM exchange, at least 256 MiB absolute savings. Report total-cgroup RAM
separately and require evidence directed at that owner goal rather than treating
the RSS gate as a substitute. Inconclusive H2 remains a hold, including for a
new engine. A source-review pass or small smoke is not qualification.

The rational near-term ordering is to resolve the physical write set and peak
composition first, then test one storage mechanism with equivalent semantics.
LMDB is a plausible sparse-update challenger; RocksDB has a stronger compaction
and memory-budget burden to justify for this mostly full-snapshot workload;
packed base/delta has the largest application correctness burden. These are
research priorities inferred from mechanisms, not measured rankings.
