# Opt-in disk-backed full scan (#11)

Implementation plan, based on origin/master 47a76ac.

1. Add a separate `scan --low-memory` engine and config setting; leave the fast
   in-memory walker and refresh unchanged. The mode uses one enumerator regardless
   of `workers`, and bounded SQLite caches. Do not promise a hard RSS limit.
2. Persist individual directory entries, pending directory jobs, and hardlink
   candidates to an explicit SQLite staging file in the snapshot directory.
   Never collect or sort a whole directory in RAM. Share filesystem classification
   semantics with the existing walker and preserve raw byte names.
3. Traverse the indexed staging rows in sibling-name DFS order with one frame per
   depth. Assign final IDs on entry; update directory aggregates on exit using
   `SubtreeAgg`. Choose hardlink owners by full byte path, not DFS rank. Create
   final indexes while empty to avoid a large external sort.
4. Reuse generation/previous-snapshot selection and atomic publication. Check the
   free-space reserve during staging and output, clean failed temporary files,
   and keep the previous published snapshot readable after any pre-publish failure.
5. Spool arbitrarily many denied paths to disk for streaming JSON, retaining only
   five warning examples in memory. Keep schema v1 and complete unknown_subtrees.
6. Test differential snapshot semantics, hardlinks, names, wide directories,
   refresh/query compatibility, failure cleanup and publication. Add a repeatable
   scaling benchmark and report RSS/time/disk tradeoffs. A constrained-memory run
   is required before claiming support for a numerical memory budget.

The working set should scale with SQLite caches and traversal depth rather than
entry count. Temporary disk space scales with entries, directory path lengths,
and hardlink paths. `temp_store=FILE` and a cache target are not hard memory limits;
all SQL traversals must have supporting indexes and bounded result consumption.

Implemented in `e89c298`: both engines share snapshot v1 publication and metadata
classification. Differential and crash/retry tests pass. The constrained
wide-directory benchmark documented in `docs/benchmarks.md` completed one million
files under a 128 MiB/no-swap cgroup while the default engine was OOM-killed.
Per-phase RSS attribution remains outside this implementation.

## D18 allocation inventory

This opt-in engine trades repeated short-lived allocations and database I/O for
entry-count-independent retention. It does **not** eliminate per-entry allocation.

| Site | Allocation/copy | Lifetime and bound |
|---|---|---|
| `staged/walk.rs::read_entry` | `file_name()` creates names for path construction and insertion; `job.path.join(...)` creates a full `PathBuf` for each entry. | One entry at a time. Paths/names are dropped after SQLite insertion. No whole-directory vector or sort. |
| `staged/walk.rs::next_job` | SQLite reads one directory-path BLOB into a `Vec<u8>`/`PathBuf`. | One active directory job. Pending jobs and their paths stay in SQLite. |
| `staged/write.rs::next_child` | Formats a small SQL string for each lookup and reads one name BLOB into `Raw.name: Vec<u8>`. | One result at a time; prepared statements are cached. There is no result `collect()`. |
| `staged/write.rs::write_root` | Clones the child name into its parent's keyset cursor; grows the DFS frame vector and current byte-path vector when a greater depth/path length is encountered. | One cursor name per active frame. Capacity follows maximum depth/path length, not total entries or sibling count. Root path cloning occurs once per root. |
| `staged/write.rs::enter` | Reads path/name strings for a mount row when encountered. | One mount row, immediately inserted and dropped. Ordinary entries keep scalar metadata on the stack. |
| `denied.rs::push` | Invalid UTF-8 paths allocate a lossy string; the first five warning paths are copied. | One current path plus five retained examples. Complete reports are written to the unlinked spool. |
| `denied.rs::serialize` | Allocates a byte vector/string for each spooled report path. | One path at a time while streaming JSON; no full report collection. |
| SQLite staging/output | Copies bound names, paths, keys and rows into SQLite storage and caches. Hardlink paths and pending directory paths consume disk space proportional to the tree. | Two 8 MiB cache targets during output, mmap disabled, indexes maintained from empty tables and indexed single-row queries. These targets are not a hard process RSS cap. |

Root preparation retains root metadata, canonical paths and mount/bind-boundary
information. Those allocations scale with the configured roots and mount table,
not with discovered entries. The root-only `Tree` explicitly disables the used-
inode preallocation estimate. The optional profiler allocates a small status string
and JSON record at phase boundaries, not per entry. Test helpers intentionally
collect small differential fixtures; production staging does not.
