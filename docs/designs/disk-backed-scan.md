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
