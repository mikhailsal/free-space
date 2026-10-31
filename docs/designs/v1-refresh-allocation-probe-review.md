# Refresh allocation probe source review

Requested/reported model: **claude-opus-5-5**, tools disabled. The final response
passed the unchanged structured-review parser and ends with **NO_FINDINGS**.
Source revision `274d412169ff344927a554769d8996b6b774b3de` is published on `perf/v1-refresh-allocation-probe`.

The initial P2 was repaired by moving the new refresh-start marker after lock
acquisition, immediately before the existing inner function. This preserves the
inner timer and large-object lifetimes and passes the actual Clippy80-line gate.
The P3 was addressed through outcome-dependent required phases: removed subtrees
do not append names/nodes; the unchanged100-entry workload requires those events.
Extra existing phase events are allowed. No ordinary semantic/cleanup check was
relaxed, and no hook was bypassed.

Normal pre-push checks passed;65 tiny Python contract tests passed under local
resource caps. Rust tests were compiled by Clippy but not executed for this
revision; the proposed future GH build runs the full suite. No optimization or
new GH run is claimed.

[Machine-readable receipt](../benchmark-results/v1-refresh-probe-source-reviews-2026-10-31.json)
retains raw reviews, model identities and reviewed source hashes. One local launch
failed before invoking the provider because its prompt file was still being
prepared; it supplied no outside coverage. The completed calls below are separate.


## Initial review

```tool-output
**Findings**

**[P2] `refresh_locked` now exceeds Clippy's 80-line limit, so `scripts/check.sh rust` fails.** In `core/src/refresh.rs`, Clippy's `too_many_lines` does not count physical lines. It counts the non-blank, non-comment lines of the body block, and that count includes the `{` line and the closing `}`.

- I count the new function at 90 physical lines: 7 blank, 2 comment-only (the "Grafting/finalization…" pair), so **81 counted lines**.
- The old function had 85 physical lines, or 76 counted. The four new `allocation`/`phase` lines in this function push it over 80.
- With `-D warnings` this fails the local pre-push gate and the workflow's `scripts/check.sh` step. The build job then stops before `cargo test` and before building either arm.
- **Fix:** move the graft/finalize span (`Paths::new` through `finalize_seconds`) into a helper that returns `(tree, fin, policy, finalize_seconds)`. Keep the same operation order and leave `loaded.raw_of`/`links` where they are, so the existing drop-before-write sequence is unchanged. Do not raise the limit or add an allow.
- This premise from the brief is wrong: "90 physical is not automatically a violation." Here it is a violation.

**[P3] The removed-subtree path never emits `refresh_after_names` or `refresh_after_nodes`.** Both markers are inside `replace_subtree`'s `Some(sub)` arm. When the target disappears from disk (`sub == None`), the trace has only `refresh_start`, `refresh_loaded`, `before_graft`, `after_graft` and `after_finalize`.

- The parent's "complete phases" check will then reject the trace as incomplete. That is correct-by-rejection, but it is indistinguishable from a broken probe.
- The current harness targets `/scratch/tree/small`, and I can't see whether `mutate()` in the `changed` scenario ever removes it.
- **Fix:** either define the required phase set per outcome (`removed` is in the CLI report), or emit both markers in the `None` arm as well. The second option is cheap and adds no per-entry work.

**Verified OK**

- **Disabled path:** `OnceLock` returns before any `/proc` read or JSON build, and the env var is read once as before. `&mut Tree` is reborrowed shared, never cloned.
- **Pointer state:** `previous_buffers` is updated only by allocation events, so phase events don't reset it. The first event reports `false`/`false`. Moving `loaded.tree` into `tree` keeps the heap pointers, so `after_finalize` compares correctly.
- **Locking:** the guard is `mut`; a poisoned mutex or failed `/proc` read returns silently. Index-assigning into a `json!` object can't panic. Write errors are ignored, so the command result is unchanged.
- **Ordering:** fault points, publication and `record_success` order are untouched. `refresh_start` runs after lock acquisition and before `cleanup_tmp`, and only creates a file under `/results`.
- **Harness gating:**
  - Seeds set `allocation_diagnostics=False` explicitly, and the observer never receives the env var.
  - The `is True` check survives the JSON round-trip.
  - The trace path is unique per arm id, `create_new` rejects reuse, and the file sits outside `/scratch/snapshot`.
  - `diagnostic_policy` matches the spec. The manifest's `operation == "refresh"` gating matches the per-arm specs.
- **Workflow:** `cargo test --workspace --locked` runs from the repo root, after the other checks, under the default `bash -e`.

**Missing context (not findings)**

- I can't see whether legacy `measured_command` runs any other `freespace` process in the worker container before the measured refresh. If it does, that process would inherit the env, claim the `create_new` file first, and leave the refresh process silently untraced.
- I can't see whether `summarize`/the pair validator rejects unexpected files in `/results/<id>/` or unexpected manifest keys.
- The full workspace tests plus two release builds may push the build job past its 25-minute timeout. I can't measure that here.

[P2] refresh_locked has 81 Clippy-counted lines (>80); check.sh rust fails and blocks the build job — extract the graft/finalize span into a helper.
[P3] after_names/after_nodes markers are missing when the subtree was removed (sub=None), so those traces fail the complete-phase check as indistinguishable from a probe failure.
```

## Repaired source review

```tool-output
**Findings**

None at P1, P2 or P3.

**Former P2 resolution**

- `refresh_locked` in `core/src/refresh.rs` now counts **80** Clippy lines: 89 physical lines, minus 7 blank lines, minus the 2-line "Grafting/finalization…" comment.
- `too_many_lines` fires only when the count is strictly greater than the threshold, so 80 passes.
- It is exactly at the limit. Any future line added to this function needs the graft/finalize extraction first.
- Moving `refresh_start` to after `_lock` and before `refresh_locked` changes no existing order:
  - `t_all` still starts inside `refresh_locked`, so `total_seconds` excludes the diagnostic line.
  - `cleanup_tmp`, the fault points, the drops before `write_snapshot`, publication and `record_success` keep their order.
  - The `record_error` path is unchanged.

**Timers**

- Every allocation marker sits inside an existing `Instant` span:
  - `refresh_loaded` is after `load_seconds` is read.
  - `before_graft`, `after_graft` and `after_finalize` are inside the finalize span.
  - `after_names` and `after_nodes` are inside `graft`.
- That diagnostic overhead counts toward `finalize_seconds`. This is only a concern for the measured diagnostic arms, and it applies equally to both arms. It is not a bug, but the contract should not compare `finalize_seconds` against non-diagnostic runs.

**Former P3 disposition**

- With outcome-dependent required phases, the `None` arm needs only the common boundaries.
- The fixed workload (`expected_subtree_entries=100`) would fail ordinary validation on `removed=True` anyway.
- Pointer state stays consistent in both arms. In the `None` arm, `after_graft` compares against `before_graft`, so it reports unchanged buffers unless `retain` reallocates, which it does not.

**Pointer fields**

- Only allocation events update `previous_buffers`.
- The first allocation event (`refresh_loaded`) reports `false`/`false`.
- Moving `loaded.tree` into `tree` keeps the heap buffers, so `after_finalize` is compared against `after_graft` correctly.
- Passing `&mut Tree` to `allocation(&Tree)` is an implicit shared reborrow, with no copy.

**File gating**

- The env var is added only when `mode == "worker"` and `allocation_diagnostics is True`.
- Seeds pass an explicit `False`, and observers never get the var.
- Per your context, `measured_command` launches exactly one `/binary` under `/usr/bin/time`, so no earlier `freespace` process can claim the `create_new` file.
- The trace lives in the per-arm results directory. `summarize` only rejects unexpected *directories* at the root and nested `spec.json` files, so `allocation-profile.jsonl` does not invalidate a run.
- Both arms of a pair carry the same `allocation_diagnostics` value, so the pair configuration-equality check holds.

**Scope**

- Only the `screen` stage runs `refresh`, so the smoke and local-scale stages are unaffected.
- No reserve or optimization code changed.

**Missing context (not findings)**

- I can't see `core/src/scan/mod.rs`. `scan::profile::allocation` must be reachable from `crate::refresh`, and `scan::tree::Node` must be visible to `profile`. The next compile will confirm both.
- I can't see whether `scan::walk` emits its own `phase` markers during `rescan_subtree`. If it does, they will appear between `refresh_loaded` and `before_graft`. They are harmless, because phase events don't touch `previous_buffers`, but the parent's completeness check should tolerate extra phase names.
- The 25-minute build-job budget with `cargo test --workspace` plus two release builds is still unmeasured.
- The workflow's `push` path filter does not include `core/**`, so Rust-only changes will not trigger it automatically. With `workflow_dispatch` that is fine.

NO_FINDINGS
```
