# Cleanup threshold prototype: integration impact

This experiment retains the root and each entry whose attributed allocated size reaches a configurable fraction of the filesystem capacity. A directory uses its entire subtree allocation. Smaller immediate children are represented by an anonymous aggregate in their retained parent; their descendant counts, sizes, and errors still contribute to the parent. A large apparent size alone does not qualify a sparse file. The experiment does not establish product suitability or whole-catalog compatibility.

A missing detailed record means that detail was omitted, not that the path was absent. An object may cross the threshold between generations without being created or deleted. Consumers must present that uncertainty explicitly:

- `ls` and tree navigation need a distinct anonymous remainder with totals and counts; it cannot expose individual omitted names.
- `top` and `find` can report retained candidates but cannot promise a complete list of matching small objects or rank them individually.
- `diff` must distinguish changes to retained records from changes inside an aggregate. A record appearing or disappearing at the threshold is not necessarily a filesystem creation or deletion.
- `status`, schema readers, web views, and the skill's instructions and output handling need to describe the coverage and avoid interpreting omitted records as missing files. Existing consumers expecting one row per entry require adaptation.

A refresh aimed inside an omitted subtree must rescan from the nearest retained ancestor, because the old snapshot does not contain the intervening structure or per-object contributions. Hardlinks can affect ownership outside the requested subtree; the prototype falls back to a full-root scan when global attribution may be needed. Exact aggregate freshness requires carrying minimum scan time and newest modification time through omitted buckets and recomputing these extrema when their contributing subtree is replaced. Independent scans have different scan timestamps, so cross-snapshot timestamp equality is not an appropriate correctness test.

`scripts/cleanup_oracle.py` compares a prototype snapshot with a separately captured full v1 snapshot of the same state. It verifies the threshold, retained paths and parent links, allocated and apparent sizes, inclusive counts and errors, and each anonymous bucket. It can also compare current v2 root totals. It streams the v1 tree with memory proportional to depth and runs outside the timed writer. This is a correctness observer, not a measurement of application resource usage. Tests use small synthetic SQLite fixtures; actual performance results belong to the benchmark output once collected.
