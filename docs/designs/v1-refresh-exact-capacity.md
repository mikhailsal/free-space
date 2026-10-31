# Refresh capacity counterfactual

Status: [source review](../benchmark-results/v1-refresh-exact-capacity-source-reviews-2026-10-31.json)
completed with the requested Opus model and `NO_FINDINGS`; normal hooks pass.
Published candidate `f04a109124c3747d937547e4fc1f6d37ad59392a` starts from
frozen release-target baseline `e8265778`. No candidate tests or measurements
have run yet.

The [musl control](../benchmark-results/v1-refresh-musl-screen-2026-10-31.md)
shows a repeatable approximately 88 MiB RSS step while appending 99 nodes to a
loaded 1,004,499-node tree. Its node buffer moves and doubles capacity; names
do not reallocate. The pinned allocator's copy-before-free path supports a
capacity counterfactual. It does not establish the candidate's benefit.

Read root metadata, walk the actual replacement subtree, and then allocate the
full tree's node vector with space for exactly its existing nodes plus the new
descendants. Reserve while the vector is empty, before loading node values.
Retain the public zero-extra loader, existing graft and hardlink rules, immutable
output, history, durability and backend defaults. Check capacity arithmetic and
raw-ID bounds. Avoid a post-load reserve and avoid using the old subtree count
as a bound on a possibly enlarged folder.

Real filesystem/snapshot preparation tests cover an empty folder growing to
128 files, 128 files shrinking to one, and folder removal. They inspect actual
preparation, graft pointer stability and finalized content counts; capacity
overflow has a separate check. Tests are prepared, not reported as passing.
`Vec` promises at least the requested capacity, so the tests do not impose an
allocator-specific exact capacity or an arbitrary rounding limit. The checked
requested-size arithmetic is tested separately.

The walked subtree remains live during full-tree loading, which can change peak
overlap for large folders. Filesystem errors can precede later corrupt-snapshot
load errors when both occur. Existing complete-command metrics and semantic,
failure, mount and raw-byte cases remain mandatory.
The folder is observed earlier relative to publication by the load duration;
changes after that walk require another refresh.

Progress also changes chronology: the filesystem walk finishes before loading
the old snapshot. Entries-seen may reach the existing 99% clamp while that load
continues, before write/publish. It never signals 100% completion; the existing
progress schema/stages are retained. This behavior must be disclosed in any PR.
Load timing includes the additional root-metadata read; walk timing covers the
actual walk, and external CLI wall time covers the entire operation.

The initial experiment is three 1m scan/refresh pairs against the same immutable
musl baseline and one conditional exploratory full-size refresh pair. Stop on
semantic/source/receipt failure or a confirmed disk, RAM or refresh regression.
An inconclusive screen is not promotion; a bounded full-size pair can investigate
scale-dependent memory or noise only under its frozen admission. Promotion still
requires the unchanged benefit floors, all required scenarios and repeated
confirmation; this initial experiment is insufficient by itself.
