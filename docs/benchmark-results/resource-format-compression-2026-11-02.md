# Storage-format overhead and compression: measured supplement

Date: 2026-11-02. This supplements the
[causal report](resource-causes-2026-11-02.md) with offline arithmetic,
not a new application run, engine comparison or compression benchmark.

## What is measured

Source: run 41373680085, unchanged E826/ED2, 1,004,499 entries. The raw
`original-artifact/measurements/disk-pin/disk.json` inside
[the retained archive](resource-cause-screen-2026-11-02.tar.gz) has SHA-256
`24b46dfbe35c12c93a751a231f8325b5ae2d58b1f034237e0f80d87d235d1f8c`.
The [derived arithmetic](resource-format-overhead-2026-11-02.json) preserves
per-object bytes and denominators. Percentages below apply to this fixture,
not directly to the user's filesystem or every v1/v2 generation.

One published snapshot is 128,012,288 B / 122.082031 MiB:

| Mutually exclusive component | MiB | Percent of file |
| --- | ---: | ---: |
| Table record payload, including SQLite record headers | 57.114496 | 46.783704% |
| Index record payload, including repeated keys and record headers | 38.245041 | 31.327330% |
| Remaining B-tree/cell structure, including rowid keys | 17.713701 | 14.509671% |
| Unused space within allocated B-tree pages | 9.006639 | 7.377530% |
| Schema record payload | 0.002154 | 0.001765% |

Index **pages**, including their own structure/unused space, occupy 55.796875
MiB / **45.704412%**. This overlaps the categories above and must not be added
to them. Tables' non-payload page bytes are 9.166754 MiB / 7.508684% of the file.
Outside table record payload lies **53.216296%** of the file; file size is
2.137496 times that payload, or 113.749642% extra relative to it. These are
explicit accounting definitions, not a claim that 53.2% is dispensable waste
caused solely by SQLite. Query indexes are an application choice, and rowid
keys carry information needed by this layout.

The stage is 72.207031 MiB logical, including 31.777344 MiB index pages
(44.008656%). Its table record payload is 34.271978 MiB (47.463492%). Stage
allocation exceeds its page image by 4,096 B, as separately documented in the
causal report. `freelist_count=0` in both databases. In-page unused bytes are
not equivalent to immediately truncatable free pages.

## Table payload is not the semantic raw minimum

SQLite payload contains its serialized record header and body. Index records
repeat indexed values plus the corresponding row key. Rowid keys and cell
framing can also lie outside record payload.
[SQLite file format](https://www.sqlite.org/fileformat.html#record_format),
[dbstat definitions](https://www.sqlite.org/dbstat.html).

The exact E826 `entries` schema has 16 columns. Fresh rows supply all of them;
the last column is NOT NULL, and INTEGER PRIMARY KEY still contributes its
NULL serial-type placeholder. At least one header-size byte plus 16 serial-type
bytes gives **at least 17 bytes per row**, or 17,076,483 B / **16.285403 MiB**.
Subtracting this bound from `entries` payload gives an encoded-body upper bound
of 42,790,099 B / **40.807818 MiB**. This is a source/format bound, not decoding
actual headers. It excludes the rowid key stored elsewhere, and still includes
repeated/derived values such as hashes, subtree ranges and aggregates. It is
therefore not a complete raw-data size or a minimum possible representation.

The schema stores names as components plus parent IDs, not full paths for every
entry. SQLite already packs integers by value, including zero-byte bodies for
0/1. A naive fixed-width binary array need not be smaller. Precise semantic
value lengths and the entropy of real names/times were not exported. Retained
artifacts contain page aggregates, not complete SQLite images/record samples;
neither exact raw minimum nor Zstd/LZ4 compression ratio can be recovered here.

## Compression mechanisms worth testing

* **Structural encoding:** place schema/type information once per block;
  encode shared scan times, repeated flags and aggregate defaults collectively;
  use dictionary/prefix and delta/run-length encodings where data supports them.
  These techniques can remove repetition before a general compressor. Their
  existence is established, for example, by the
  [Parquet encoding specification](https://parquet.apache.org/docs/file-format/data-pages/encodings/);
  this is not a recommendation to use Parquet as the application's mutable index.
* **Block compression:** compare Zstd and LZ4 on bounded independently readable
  blocks, including directory/index metadata and decoder buffers. The proposed
  priority is Zstd for a space-focused candidate and LZ4 as a low-CPU control,
  an experimental choice rather than a measured application result.
  [Zstd](https://github.com/facebook/zstd), [LZ4](https://github.com/lz4/lz4).
* **Avoid whole-file decompression as a hidden cost:** keeping a compressed
  archive plus a fully expanded working SQLite file can increase peak disk.
  Archives alone measure compressibility, not a usable query/refresh backend.
  A bounded-block reader or compression VFS needs its own correctness and cost
  proof. SQLite's official [ZIPVFS](https://sqlite.org/zipvfs/doc/trunk/www/readme.wiki)
  supports compressed databases under a separate paid source license; it is
  not a compression flag in this project's current bundled SQLite.

No 2x/5x/10x improvement is claimed. Fixed mtimes in the synthetic fixture can
make compression more favorable than on real metadata. A representative name/
timestamp distribution is needed before estimating product savings.

## Stack recommendation and next controlled comparison

Keep Rust. The storage alternatives deserve a concrete representation test:

| Candidate | Reason to test | Cost that must remain visible |
| --- | --- | --- |
| Current SQLite | Low-change control; improve costly indexes/duplicated fields or introduce a bounded compressed-block data layer | Equivalent queries/indexes, per-block cache, schema/publication changes; compressed BLOBs do not preserve indexed SQL fields automatically |
| Packed immutable blocks + Zstd/LZ4 + bounded update delta | Can amortize record headers and encode repeated fields while supporting selective reads | Query/inode/order indexes, delta lookups, readers, old/new bases and complete merge peaks; all benefits unmeasured |
| RocksDB with block compression | Existing LZ4/Zstd compression for data/index blocks and sparse writes | WAL, memtables, caches, filters, compaction and retained checkpoints; no app-specific size/RAM advantage measured |
| LMDB | An alternative sparse B-tree/representation control | mmap does not itself compress bytes; application codecs, indexes and reader-pinned COW pages still count |

RocksDB's own docs describe
[compression](https://github.com/meta/rocksdb/wiki/Compression),
[memory components](https://github.com/meta/rocksdb/wiki/Memory-usage-in-RocksDB)
and [amplification/compaction](https://github.com/meta/rocksdb/wiki/RocksDB-Tuning-Guide).
These mechanisms do not establish that a replacement beats SQLite here.

Proposed next step: on GitHub, preserve an exact SQLite image and a canonical
logical export, then compare uncompressed/encoded/compressed representations
with a decoded-content digest. Report a raw-size reference separately from a
usable format with all equivalent indexes. Only then compare end-to-end full
scan, folder refresh, total RAM and peak disk including compression, queries,
maintenance, old/new representations and crash recovery. No new run is admitted
or dispatched by this document; existing acceptance/budget gates still apply.

Compression can reduce storage and I/O and sometimes file-cache residency. It
does not itself remove the metadata-lookup kernel charge proved by the causal
experiment; decoder/cache memory is an additional quantity to measure.
