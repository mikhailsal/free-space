# W4: one bounded child-query batching experiment

Status: source candidate prepared in `perf/v1-child-batches`, based on d8b6046.
No W4 Rust tests, GH jobs, timing or resource improvements are claimed. Runner
allocation is pending; D17 authorized W3 only. The candidate does not include the
rejected W3 leaf SELECT/UPDATE changes.

Normal pre-push checks passed after fixing a test-function length violation:
Rust compilation/Clippy/format, tracked-file size, shell, TypeScript and Biome.
No hook was bypassed. These checks compile the prepared Rust tests; they do not
execute them. The final Opus source review reports no findings; see
[review history](v1-child-batch-implementation-review.md).

## Problem and selected treatment

The baseline staged output writer executes a keyset `SELECT ... LIMIT 1` for each
child and an empty query at the end of each node. Preparing statements is already
cached; the proposed saving is fewer SQLite query executions. The W3 full-scale
pair measured 365 s walking/staging and 153 s inserting in baseline. Output
traversal is only part of total latency, so a 15% full-command gain is uncertain.
This remains a bounded experiment, not an accepted optimization.

Change only the output writer's child fetching, using the existing
`raw_children(parent,name,id)` index and order:

- Query at most 32 ordered rows per batch. Store a consumed `(name,id)` cursor;
  prefetch does not advance it.
- Share one pending-cache budget across every active DFS frame: at most 256
  allocated row slots and 64 KiB of charged retained capacity. Charge actual
  `VecDeque.capacity() * size_of::<Raw>()`, retained name capacities and a
  64-byte bookkeeping allowance per allocation. Free drained buffers immediately.
- Existing baseline cursor/path/frame memory, SQLite memory, allocator rounding
  and one in-flight decoded row remain outside this extra-cache budget. The byte
  counter is not a physical allocator or process-RSS ceiling. Compare measured
  RSS and successful memory budgets under the unchanged owner gates.
- When the shared cache budget is unavailable, use the original `LIMIT 1` query.
  An oversized first row returns directly; a later row that does not fit is
  discarded without cursor advancement and fetched again after retained rows
  are consumed. No over-budget row remains in a pending buffer.
- Before a directory refills an empty buffer with insufficient shared space,
  evict the shallowest pending ancestor buffers until a full batch has logical
  headroom, including 256 name bytes plus the allowance per row. This is a cache
  planning target, not a filename limit. Their consumed
  cursors permit safe refetch on return. Leaf lookups do not trigger eviction;
  otherwise a deep leaf could repeatedly discard useful parent prefetch.
- Query and decode the first child before allocating a batch. Empty leaf queries
  allocate no row buffer. Preserve baseline leaf queries and aggregate UPDATEs,
  DFS IDs, parent folding, root order, hardlink ownership and side tables.

No SQLite cache-size, journal, mmap, durability, publication or public CLI change.
Private diagnostic counters compile only under `cfg(test)`; production adds no
per-entry tracing. Files: staged `write.rs`, private `write/batch.rs` and staged
differential tests. This is one candidate with fixed internal limits, not a sweep.

## Prepared verification

Private tests compare actual child-query counts on 75 synthetic raw-name rows:
the expected result is four batch queries versus 76 original fallback queries,
including terminal empty queries. This expectation has not yet been executed.
Other tests cover the shared slot limit at many active depths, capacity release,
byte exhaustion inside equal-name/id tie groups, discarded-prefetch recovery,
an oversized row and shallowest-ancestor eviction with cursor resumption.
Synthetic SQL names can exceed filesystem basename limits; the filesystem fixture
uses valid long names.

A real filesystem differential fixture has 10 nested directories and 40 long-name
files at each level (411 entries including root), with the next directory sorting
first. This exercises DFS descent while ancestor buffers remain pending. Existing
staged tests retain raw-name, hardlink, root-order, denied/mount and reserve-failure
coverage. Full Rust workspace tests and normal repository gates must pass on GH
before any performance interpretation. A source review cannot replace execution.

## Proposed runner allocation — not dispatched

Use only the unused portion of the same initial six aggregate runner-hours:
13,482 seconds remain after W1/W3. Reserve **two screening jobs, 5,700 aggregate
runner-seconds each** (25-minute build plus 65-minute measured step and 5-minute
paired-job margin). Maximum additional reservation is 11,400 seconds; total
committed campaign time would be 19,518 seconds, leaving 2,082 seconds contingency.
Every build, observer, failure and cleanup counts. No paid/larger runner is used.

1. Current-observer baseline/base control: protocol v3, `local` I/O, 1m files,
   three unchanged scan pairs and three unchanged 100-entry-folder refresh pairs.
   Capture the same fixture/seed and full provenance rules as W1. No product
   change is inferred from the older v2 controls.
2. Only after a valid control and renewed remaining-budget admission: the same
   six pairs for baseline d8b6046 versus the single batching candidate, built with
   identical compiler/target/profile/flags. Freeze exact candidate SHA and run
   manifest before dispatch. Run Rust tests in the candidate build.

Stop on invalid evidence, reserve/cleanup failure, time admission failure or a
failed resource/latency gate. No automatic retry, alternate batch-size sweep or
full-scale expansion is authorized by this proposal. Even a promising 1m screen
cannot qualify release: full-scale controls/confirmation, resource-budget sweeps,
changed/large refresh guards and sampling sensitivity remain separate obligations.
Do not pool the two source candidates' hashes or move acceptance thresholds.

## Owner decision D18 — pending

The only new owner choice is scope allocation: permit these two bounded W4
screening jobs within the existing remainder, or stop the first campaign after
W3. Internal batch limits and fallback mechanics are reviewable implementation
choices within approved W4 scope. This proposal does not request a larger total
time cap or any relaxation of the 15% benefit/no-regression gates.
