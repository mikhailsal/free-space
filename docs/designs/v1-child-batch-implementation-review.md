# W4 child-batching source reviews

Requested and reported model: **claude-opus-5-5**, through the gstack
`claude-code` skill with tools disabled. Three source-review results completed.
The first two pass the structured parser; the final response explicitly says
**NO_FINDINGS**, closing the prior P2 issues, but its trailing observations do not
match the parser's required concluding sentence. A format-only provider
confirmation returned bare `NO_FINDINGS` and passed the unchanged parser; this
restates the existing verdict without another source inspection. Original
responses and the formatting issue are preserved. This is source review only, not
Rust-test execution or performance/resource acceptance.

The experiment is on branch `perf/v1-child-batches`: initial code f6baca7,
name-headroom fix f228764; documentation-only cherry-pick147fa29 does not change
reviewed source. Final source hashes are retained in the machine-readable receipt.
The branch includes none of the rejected W3 leaf-query/UPDATE shortcuts.

CLAUDE CODE SAYS (review, final verdict format confirmation):

```tool-output
NO_FINDINGS
```

The [machine-readable receipt](../benchmark-results/v1-w4-source-reviews-2026-10-31.json)
retains all provider model identities, results, final source hashes and the parser
disposition. No global gstack parser, approval rule or Git hook was changed.

Parent dispositions: ancestor starvation was fixed by shallowest-buffer eviction,
then its name-byte gap was fixed with full-batch planning headroom. A real
pre-push hook rejected one86-line test; extracting its fixture preserved assertions
and fixes the80-line limit. The normal push then passed compilation/Clippy/format,
size, shell, TypeScript and Biome. No hook was bypassed. Mount concerns were closed from
staged walk/visitor source; no mount semantics changed. Optional production
profile counters were not added to this bounded experiment. Actual Rust workspace
execution and performance screening remain pending a new runner allocation.


## Initial source review

```tool-output
# Review: W4 child batching (`core/src/scan/staged/write/batch.rs`)

Nothing was built, tested or benchmarked. Everything below comes from reading the supplied sources only.

## Verified by reading (no finding)

- **Ordering, ties and non-UTF-8 names.** Both the batch and `single` paths use the same `(name,id)>(?2,?3) ORDER BY name,id` predicate as baseline, with the cursor bound as a BLOB.
  - The cursor always records the last *consumed* row (`write.rs` `write_root`).
  - A refill happens only once `pending` is empty, so it resumes right after the last row in the buffer.
  - A row discarded at the byte break, and everything after it, was never buffered, so the next query refetches it.
  - An oversized first row is returned directly and advances the cursor normally.
  - Result: no dropped or duplicated rows on any path. Unit test 1 has a 3-row tie group spanning the 32-row boundary (positions 30–32) with `0xff` names.
- **DFS ids, intervals, aggregates, hardlinks, denied entries and mounts.** Visit order is unchanged, so `enter` and the `UPDATE`/merge logic see the same sequence.
  - `enter`'s reads on `self.conn` (`attribute_link`, `mount_info`) run after the batch `Rows` and statement are dropped and reset, so no statement is active during them.
  - The staging data is immutable during output, so prefetched rows cannot go stale.
- **Budget accounting.**
  - `pending.bytes` = buffer charge + name charges. `pop` subtracts each name charge from both counters, and `release` removes the remaining buffer charge and the capacity.
  - A frame is popped only when `next` returns `None`, which requires an empty `pending`, and an empty `pending` has always been released (capacity 0). So no budget leaks across frames.
  - On error paths, frames are dropped with live `pending`, but `Budget` is local to `write_root` and the whole write aborts, so nothing persists.
  - Counters are bounded by 256 slots and 64 KiB, so there is no overflow risk at current constants.
- **Statement cache.** This adds 2 cached statements on `self.conn` (4 child selects plus the links select). That is well under rusqlite's default capacity of 16, *if* the walk phase didn't leave many cached statements on the same connection. That context wasn't supplied.

## Findings

**[P2] The shared budget is held by shallow ancestors for their whole subtree, so the deep directories that hold most files fall back to `LIMIT 1`** (`batch.rs` `next` budget checks; `write.rs` `write_root`).
- An ancestor's buffer is released only after all of its buffered children *and their subtrees* have been walked.
- Each buffer costs about `32·size_of::<Raw>()+64` bytes (≈3.9 KB if `Raw` is ~120 B) and 32 slots, so at most 8 frames can hold buffers.
- On real trees, `/`, `/usr`, `/usr/share` and similar ancestors will usually occupy those slots. Deep, wide leaf directories then run the old per-row `LIMIT 1` path.
- The new integration test is built exactly this way (`0-next` sorts first at every level).
- Impact: the benchmark may measure a crippled variant, and the result depends on tree shape, so 1m and 7.3m trees can disagree.
- Possible fix within scope: since the cursor only records consumed rows, dropping an ancestor's `pending` (call `release`) is always safe; the ancestor simply refetches up to 31 rows later. Evicting the shallowest buffer when a deeper frame needs budget would point the cache at the active frame without changing the memory ceiling.
- Alternatively, accept this explicitly as the experiment's definition and let the gate decide. In that case, record the fallback rate (for example, in profile output) so a rejection can be read correctly.

**[P3] The SQL hard-codes `LIMIT 32` independently of `const BATCH`** (`batch.rs` `next`, both SQL strings).
- If `BATCH` is ever lowered without editing the SQL, `push_back` grows the `VecDeque` past the capacity that was charged.
- `release` then subtracts the larger capacity: a panic in debug builds, and wraparound in release builds that corrupts or disables the budget.
- Suggested fix: use `LIMIT {BATCH}` in the `format!`, and add `debug_assert_eq!(pending.rows.capacity(), charged)` before release.
- Also, both SQL strings are rebuilt with `format!` on every refill and every `single` call (baseline did the same per call). `concat!`/`const` strings would avoid the allocation and remove the duplication between `next` and `single`.

**[P3] File size and function limits** (`batch.rs`).
- The inline `mod tests` is ~170 lines and counts toward the 500-line *source* limit, because only `*/tests.rs` counts as a test file. The project CLAUDE.md says to put long Rust tests in a `tests.rs` submodule; suggest moving them to `write/batch/tests.rs`.
- `next` is roughly 75–80 lines, at the 80-line limit. Splitting out the fill loop (from `pending.rows = buffer` through the `while`) into `fill(...)` gives headroom.
- `core/src/scan/staged/write/` is untracked. The size check counts only tracked files, so it won't see `batch.rs` until it is added.

**[P3] Some assertions are brittle or cannot detect the behaviour they target.**
- `shared_budget_falls_back_and_releases_drained_capacity` asserts `slots == 256`. That relies on `VecDeque::with_capacity(32)` returning exactly 32 (the docs say "at least") and on 8 buffers fitting in 64 KiB at the current `size_of::<Raw>()`. If `Node` grows, the test fails for a reason unrelated to correctness. Deriving the expected frame count from the constants and asserting it would make the dependency explicit.
- `disk_scan_matches_deep_wide_tree_with_pending_ancestor_batches` cannot confirm that fallback actually happened, because `selects` is `#[cfg(test)]` and invisible to integration tests. As a differential correctness test it is fine; just don't read it as fallback coverage.
- The unit tests' `take` re-implements the cursor update from `write_root`. Only the integration tests check that the real call site matches.
- Not covered: a byte-limit break in the middle of a tie group (same `(name)` with different ids). It uses the same cursor logic, so this is low risk.

**[P3] The ceiling is exceeded transiently before the second check** (`batch.rs`, the `VecDeque::with_capacity(BATCH)` block). The buffer is allocated *before* the slot/byte recheck and dropped if rejected, and the row discarded at the break is fully materialized. Both are short-lived and arguably "in-flight". Still, the doc comment on `next` should say that the 64 KiB ceiling covers retained extra cache, not the peak.

No correctness P1s found.

## Is a bounded 1m screen reasonable?

Yes, as a cheap kill screen, but not as promotion evidence; accepting W4 without the full gate is not an option. At the large size, the 15% full-command gate needs about 80 s off a ~534 s total, while insert plus index is only ~169 s. That means W4 must remove roughly half of that. W3 cut only ~56 s there, and its gain decayed from 14.29% at 1m to 11.92% at 7.3m. W4 leaves the per-entry empty leaf SELECT untouched and is further limited by the ancestor-starvation issue above, so its ceiling is plausibly below W3's. That makes a pre-registered stop rule sensible: if the 1m screen doesn't clear 15% with clear margin, stop. A 1m pass would still need the full three-pair confirmation at the large size, and the single W3 pair's walk difference of 365 vs 358 s shows walk noise alone can be several seconds per pair.
```

## Eviction follow-up

```tool-output
# Follow-up review: W4 child batching (source only)

Nothing was built, tested or benchmarked. These findings come only from reading the supplied `write.rs`, `batch.rs`, `batch/tests.rs` and `core/tests/staged.rs`.

## Previous findings: resolution check

- **Prior P2, ancestor starvation: mostly resolved.** `evict_for_directory` runs before each `next`. It does nothing for non-`Dir` frames or when the current directory still has buffered rows. Otherwise it walks ancestors from the shallowest and calls `release` until `can_buffer()` is true.
  - Eviction is safe. The cursor records only consumed rows, and the test `eviction_prefers_shallow_ancestor_and_resumes_cursor` shows the root resuming at id 2.
  - Eviction cannot thrash. An empty or denied directory may evict once, but it buffers nothing, so the freed budget stays available for its siblings. Extra refetches are bounded by roughly one per directory entry.
  - Popped frames always hold released buffers, so budget never leaks across frames.
  - Two gaps remain; see the first two findings below.
- **Prior P3, `LIMIT 32`: resolved.** Both SQL strings use `LIMIT {BATCH}`, and `release` has `debug_assert_eq!(capacity, slots)`. The charge uses the actual `buffer.capacity()`, so the assert holds even if `with_capacity` rounds up. In that case the second check only falls back for that refill.
- **Prior P3, file and function limits: resolved.**
  - The tests are in `batch/tests.rs`.
  - `next` is about 75 lines.
  - `write/` is still untracked, so the size check cannot see it until it is added. You are presumably aware of this.
- **Prior P3, brittle assertions: resolved.**
  - The two-buffer test derives its limit from the actual capacity.
  - The equal-name tie test exercises a byte break inside a tie group.
- **Prior P3, transient ceiling: resolved as documentation.** Both comments now limit the claim to retained cache, not RSS.

## Findings

**[P2] Eviction stops at the fixed minimum and ignores name bytes. The active directory can therefore stay on partial batches, or on the single-row path, for its whole width.** (`write.rs` `evict_for_directory`; `batch.rs` `can_buffer` and the first and fill checks in `next`)

- **Root cause.** `can_buffer()` checks only `bytes + BATCH·size_of::<Raw>() + 64 ≤ max`. Eviction stops as soon as that holds, but `next` also needs room for the names.
- **Single-row window.** After eviction, the headroom can sit between `minimum` and `minimum + first_bytes`, a window of up to about 319 bytes. In that window the first check returns `first` with no buffer, every time.
- **Why it persists.** On the next call, `pending` is empty but `can_buffer()` is already true, so nothing more is evicted. In a flat, wide directory, nothing else changes the budget. Every row then goes through the `LIMIT 32` query returning one row, which is the original P2 failure mode.
- **Partial-batch case (more common).** The headroom only slightly exceeds `minimum`, so the fill loop breaks after a few names. Ancestors that hold large buffers are never evicted further. With 64 KiB, `Raw` around 110 B and names around 200 B, several buffers fit, and the active frame can get, for example, 5–15 rows per refill indefinitely.
- **Impact.** The gain depends on tree shape and name length, and it is invisible in the benchmark. That is the same validity concern as before, in a narrower form.
- **Fix options, within scope:**
  - Make the eviction target include name headroom, for example `minimum + BATCH·(NAME_MAX + ALLOC_OVERHEAD)`, or at least `minimum + first_bytes`.
  - Or have `next` report "needs eviction" when either byte check fails, and retry after evicting.
- **Test gap.** The eviction unit test is driven only by slots (`max_slots = budget.slots`, `max_bytes = MAX`). Nothing covers byte-driven eviction.

**[P3] `Kind::Mount` frames never trigger eviction** (`evict_for_directory`: `current.kind != Kind::Dir`).

- The finding applies only if mount entries have children in `raw`. That is plausible with `one_file_system: false`, but the supplied code doesn't show it.
- If they do, a mount point's children fall back to `LIMIT 1` whenever ancestors fill the budget. Results stay correct; only the speed suffers.
- The same question applies to root frames whose kind is `Mount`.
- Fix options: treat "kind may have children" as eligible, or state the exclusion explicitly.

**[P3] Fallback and partial-batch rates are still unobservable outside unit tests.** `selects` is `#[cfg(test)]`.

- This doesn't block anything, given the stated split between the differential integration test and the fallback/eviction unit tests.
- But if the 1m screen fails, it can't distinguish "W4 can't help" from the P2 above. One profile counter for single-row versus batched refills, plus the mean rows per refill, would let a result be read correctly.

## Verified, no new bugs

- **Error paths.** In the fill loop, the budget is charged only after `Raw::from_sql` succeeds, and each push matches its charge. On error, the write aborts and `Budget` is local, as before.
- **Ordering.** Ordering, cursor resume and tie handling are unchanged. Eviction preserves the "refill only from the consumed cursor" invariant.
- **Leaf gating.** Leaf frames still run their child query and `UPDATE`, and they don't evict. The test asserts this.
- **Constants.** `can_buffer` and `minimum` use the same constants, so there is no mismatch between the eviction target and the first check, apart from the name-byte gap above.

**Conclusion:** the prior P2 is not fully resolved. One new P2 remains: the eviction target excludes name bytes. No P1s.
```

## Final delta review

```tool-output
# W4 child batching: final source review

This review comes only from reading the supplied sources and diff. Nothing was built, tested or benchmarked.

**NO_FINDINGS**

## Prior P2 (eviction ignored name bytes): resolved

- **Target.** `evict_for_directory` (`write.rs`) now stops on `can_prefetch_full_batch()`, which requires two things:
  - `can_buffer()` (slots and the minimum) holds;
  - `bytes + minimum + 32·(256+64)` fits within `max_bytes`.
- **Which checks use it.** Only eviction uses this predicate. `next()` still uses `can_buffer()` together with its first-row and fill byte checks. Strict accounting is unchanged.
- **Termination.**
  - Eviction runs only when the current frame is a `Dir` whose `pending` is empty.
  - Releasing every ancestor brings `budget.bytes` and `budget.slots` to 0, because nothing else charges the budget.
  - With the default constants (64 KiB, 256 slots), the reserve is well below `BYTE_LIMIT`. So the loop always reaches a satisfiable state or runs out of ancestors.
- **No single-row trap.**
  - After eviction, the headroom covers the first row's charge for any name up to about 10 KiB.
  - The single-row window the previous review described can therefore occur only when a name exceeds the planning headroom. That was accepted as a smaller-batch or fallback case.
- **Thrash bound.**
  - Releasing a buffer frees budget that stays free until the active frame charges it.
  - A drained or empty directory can still evict once before its terminating query returns `None`. Its siblings or its parent then reuse the freed budget.
  - Each eviction costs at most one cursor refetch of 32 rows or fewer. This matches the bound from the previous review; the only change is a larger target.

## New test `byte_headroom_evicts_shallow_ancestor_and_resumes_cursor`

- **Setup.** Frames 0 and 1 each consume one row and keep 3 rows pending. `max_bytes = bytes + reserve − 1` makes `can_buffer()` true and `can_prefetch_full_batch()` false. This is exactly the "minimum fits, full headroom doesn't" case.
- **Shallowest first.** Releasing frame 0 lowers `bytes` by more than 1, so the predicate becomes true and the loop breaks before frame 1. Both assertions (`stack[0]` empty, `stack[1]` retained) follow from the source.
- **Cursor resume.** The root still has cursor `("000", 1)`, so refetching returns id 2. This holds whatever the name capacities are.
- **Independence.** The test computes `reserve` from the same constants as the predicate. It does not depend on `VecDeque` capacity rounding, on `size_of::<Raw>()`, or on rusqlite's `Vec` capacity.
- **Slots versus bytes.** `max_slots` is `usize::MAX`, so this test is driven by bytes only. The existing slot-driven test still covers the slot path.

## Fixture extraction in `shared_budget_falls_back_and_releases_drained_capacity`

- `populate_nested_wide` performs the same inserts and returns `parents`. Every original `take` call and assertion is still present.
- `&conn` became `conn`, which only adjusts for the borrow. Behavior is unchanged.

## Correctness, order and error paths

- **Unchanged code paths.** The changes do not touch:
  - the fill loop's charge-after-`from_sql`;
  - push and charge parity;
  - `pop` releasing on empty;
  - the `release` assertion on `capacity == slots`;
  - `(name,id)` cursor ordering;
  - the break that discards the prefetched row while keeping the consumed cursor.
- **Errors.** On error, the transaction aborts and the local `Budget` is dropped, as before.
- **Leaf frames.** Leaf frames still do not evict; the existing test asserts this.

## Mount concern: closed

- `walk.rs` `record_entry` gives crossing or bind-mounted directories the kind `Kind::Mount` and does not call `self.job`. They therefore never get children in `raw`.
- `visit.rs` descends only into frames with `is_directory` set (`Kind::Dir`).
- Excluding `Kind::Mount` from eviction therefore has no effect.

## Observations (not findings)

- **Constant coupling.** If `BYTE_LIMIT` or `ROW_LIMIT` were later reduced below the prefetch reserve, eviction would release every ancestor at every refill. This would remain correct but would degenerate. A `const _: () = assert!(...)` tying `BYTE_LIMIT` to the reserve would make the coupling explicit. This is optional and not required for this experiment.
- **Size check.** Earlier, `write/` was untracked, so the size check could not see it. Make sure it is added before the hook rerun, so the size check covers `batch.rs` and `tests.rs`.

**Conclusion:** the prior P2 is resolved within the accepted definitions. There are no new P1, P2 or P3 bugs in the supplied source.
```
