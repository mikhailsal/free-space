# Coarse image: tiny actual-format check

Supporting observation from successful roomy-v2 smoke run `41482637607`; this is not the million-entry screen or a writer performance/compression benchmark. Only two tiny snapshot members were fetched by bounded HTTP ranges, decoded under a 64 KiB limit and inspected read-only. Both databases pass integrity checks and retain eight nodes.

| Profile | Current DB bytes | Table record payload bytes | Parent-index payload bytes | Schema payload bytes | Other page bytes | gzip bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| files | 16384 | 851 | 370 | 969 | 14194 | 926 |
| directories | 16384 | 855 | 370 | 969 | 14190 | 945 |

Each current image has four 4,096-byte pages: schema, meta table, nodes table and parent index. Two 16 KiB images total 32 KiB logical bytes; the worker separately measured allocated endpoint bytes. Table payload includes SQLite record headers and encoded data; it is an upper bound on semantic field bytes, not a raw-data minimum. Index payload repeats routing keys. Other page bytes combine unused space and page/cell/header structure.

The small database therefore has a large percentage of page/format overhead but a tiny absolute footprint. gzip reduces the actual file to about 0.9 KiB; that optional evidence compression ran outside the writer timing and is not used by the prototype. No general compression ratio or RAM/time improvement on real user data is inferred. The main storage reduction comes from discarded fine-grained identity, while retaining Rust and SQLite.

The nominal equalish control becomes retained because its actual allocated bytes exceed its block-aligned apparent length by 4 KiB. Its apparent length is 536,166,400 bytes, allocated size 536,170,496 bytes and threshold 536,166,532 bytes. This is why the fixture and oracle use actual allocation instead of assuming that file length equals occupied bytes.

See the [raw format JSON](cleanup-threshold-small-image-format-2026-11-02.json) and [tiny image evidence](cleanup-threshold-small-image-format-2026-11-02.tar.gz).
