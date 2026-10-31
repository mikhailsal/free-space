# External plan review receipt

Date: 2026-10-31. Provider: Claude Code. Requested and reported model: `claude-opus-5-5`. Exit: 0. Scope: first 30 KB of the plan plus all D1–D13 decisions; no repository access. The report below is reviewer opinion, not automatically verified findings or owner approval.

# Review: V1-first performance plan (first 30 KB plus the owner decisions)

Scope: static review of the supplied text only. I have not inspected the repository. The engineering-review section was truncated, so some gaps below may be addressed there.

## Findings

### High

**H1. The workload ladder cannot fit the D13 campaign cap, and the plan does not reconcile the two.**
- Native confirmation alone is capped at 3 × 150 min = 7.5 runner-hours for one candidate on one metric. D13's initial campaign is 6 aggregate runner-hours.
- The plan also requires:
  - 1m screening (90 min per job);
  - limited confirmation (180 min);
  - a 4-budget × scan+refresh memory sweep at 1m, then 2 budgets × 2 operations at 7.2m;
  - 3 refresh sizes × unchanged/changed;
  - three W3 arms, a W4 batch arm and up to 9 cache combinations;
  - W5.
- The caps are envelopes, not estimates. Even at realistic durations (a v1 7.2m scan is about 6 min, but fixture creation, seeding and observers dominate), 6 hours covers roughly baseline noise calibration plus one candidate's confirmation.
- In the reviewed text, per-campaign manifests cap minutes, but nothing allocates the global 6 hours across W1 calibration, the baseline minimum-budget sweep and W3.
- Fix: add an explicit allocation table per wave, and a stop rule when a wave's share is spent.

**H2. "Calibrated measurement uncertainty" is the operative term in nearly every gate and is never defined.**
- The plan does not specify how uncertainty is estimated, which runs pay for it, or what counts as "confirmed".
- The gates are asymmetric:
  - Benefit gates (D8–D10) require exceeding uncertainty, so an underpowered run comes out inconclusive.
  - Regression gates (D2, D3, D6) require a *confirmed* increase, so an underpowered run passes.
- With three pairs on noisy cloud disks, a real 5–10% regression in refresh time or peak disk will usually go unconfirmed.
- Fix:
  - Predeclare A/A (baseline vs baseline) runs per metric and scale, and charge them to the D13 budget.
  - Define the band, for example the max |Δ| across A/A pairs or a stated multiple.
  - Define the decision rule, for example "confirmed regression = median paired Δ beyond the band and ≥2 of 3 pairs worse".
  - Do all of this before any candidate runs. "Fast baseline/base controls" points at this but is not specified or budgeted.

**H3. The W3 skip of the leaf final UPDATE may conflict with full-path hardlink ownership.**
- The writer chooses owners by full path (`staged/write.rs:249`, per the plan). DFS over children sorted by raw name visits paths in component order, which is not full-path byte order.
- Example: names containing bytes below `/` (0x2F), such as `-`, `.`, space or `!`.
  - DFS visits `a/x` before `a-b/x`, because component `a` sorts before `a-b`.
  - Byte-wise, `a-b/x` < `a/x`.
- If any later step reassigns a hardlink owner and relies on the final per-leaf UPDATE to correct the insert-time value, skipping that UPDATE silently misattributes sizes.
- "Prove inserted values equal final values" is the right requirement but is not concrete enough. Add fixtures that hardlink across sibling directories whose names differ only by bytes 0x20–0x2E, and assert owner and aggregate equality against baseline.

**H4. W4 cache increases and batching structurally conflict with D3 at the measured baseline.**
- v1 scan peak RSS is 23.23 MiB with two 8 MiB connection caches. Raising both to 16 or 32 MiB can add 16–48 MiB, which is a large *confirmed* RSS increase.
- D3 has no exchange rule. D5 exchanges only against a full-scan *slowdown*, not an RSS increase. Any retained-row batch budget raises RSS the same way.
- As written, most W4 arms are likely to fail D3 regardless of speed.
- Fix: limit W4 to batching within a budget that keeps RSS inside the noise band, or drop cache-increase arms and save the runner-hours. This needs a decision, not more measurement.

**H5. Under the current decisions, the W5 staged refresh is very likely blocked from promotion, and no cheap kill criterion exists.**
- The current refresh holds the tree in RAM and writes one replacement. A disk-backed staging pass adds O(total entries) of I/O plus a staging DB, roughly 480 MiB at 7.2m by the scan analogue.
- That makes a confirmed refresh slowdown (blocked by D6) and a confirmed peak-disk increase (blocked by D2) the expected outcomes.
- D5's exchange covers only full-scan slowdown, so even a 75% refresh RSS reduction cannot buy any refresh slowdown.
- D7 says the experiment may fail, which is consistent. However, the plan should add a 1m pre-screen with an explicit kill rule (for example, abandon if 1m refresh time or peak disk rises beyond the band) before any 7.2m spend.
- Whether the absolute D6 bar is intended for a refresh-RSS trade is an owner question (Q2).

**H6. Early cutover (W6) does not qualify v1 refresh on the actual host.**
- At 7.2m, v1 refresh measured about 1.1 GiB RSS and about 28 s for an unchanged 18-entry subtree. The installed v2 took 0.0176 s.
- Full-scan `low_memory` does not cover refresh. The D12 calibration covers a scan only.
- The host's RAM and entry count are unknown in the material. A GUI refresh after cutover could therefore be very slow or OOM, and nothing in W2 or W6 checks for it.
- The plan never states that cutover knowingly accepts a refresh regression of several orders of magnitude relative to current behaviour.
- Fix: add host-specific refresh RSS and latency to the cutover checklist, or record explicit acceptance (Q3).

### Medium

**M1. No in-scope mechanism can meet D10, so the "optimize peak disk" request is served only by W2 deletion.**
- The plan's own analysis puts the v1 peak at current + previous + replacement + staging. Staging is about 480 MiB of about 2.95 GiB (16%).
- Constraint 5 protects current/previous, and the new metric includes the seeded snapshots.
- A 25% reduction therefore needs a representation change, which is out of scope. State plainly that the disk outcome is delivered by cutover (removing about 8.8 GiB of v2 data and 2.1 GiB of legacy v1 files), not by code.

**M2. Percentage-only RSS gates create a loophole on the scan path.**
- 25% of 23 MiB is about 6 MiB. Under D9 that qualifies as a "memory candidate".
- Under D5, a 30% RSS reduction (about 7 MiB) can buy a 15% full-scan slowdown (about 51 s).
- This is probably not the owner's intent, but only the owner can add an absolute floor or tie the gate to the operation's dominant resource (Q4).

**M3. The memory-budget grid is coarse and floor-censored.**
- A scan using 23 MiB RSS will likely pass at 512 MiB for both arms. Neither an improvement nor a D3 regression is then observable below the floor.
- The 2× steps cannot detect a D3 regression within a step.
- D11 requires a *baseline* minimum tested successful budget at 7.2m, which is its own sweep cost and is not listed separately.
- Fix: add a 256 MiB (or lower) rung, or bisect between the last failure and first success, and budget the baseline sweep explicitly.

**M4. The D12 calibration scan is not integrated into the plan.**
The plan needs to state:
- **Purpose:** whether it feeds U3 (shape), W2 capacity, or timing. The plan says no local traversal is needed for fixtures.
- **What is recorded:** entry count, depth/fanout/hardlink distributions, peak disk, RSS and cgroup memory, wall time, and binary hash (U4).
- **First-scan limit:** with isolated output and no previous snapshot, it measures only the first-scan peak. Steady-state current + previous + replacement + staging must be extrapolated for W2 capacity planning.
- **Comparability:** without a cache flush it is not comparable to the runners' cold arms. Label it as a sensitivity observation.
- **Timer contention:** whether the v2 timer was paused. A concurrent v2 scheduled scan would invalidate the timing and contend for disk.
- **Deletion trigger:** what "needed" means as a capacity threshold, and that deleting the v2 DB frees nothing while the GUI or server holds it open. Deletion therefore needs a stop-serving procedure now, not in W2.

**M5. The plan's status text contradicts the owner decisions.**
- "Proposed performance thresholds and optional staged-refresh scope remain pending owner review" contradicts D2–D11 and D7, which are now decided.
- "At most two conditional experiments promoted to their own waves" can yield 8 waves, against D1's six. State that any extra wave needs a new owner decision.

### Low

**L1. The dependency table is inconsistent.**
- W2 is listed as "none" but "consumes W1 disk admission contract".
- W5 depends on "stabilized W4 interfaces", but W4 may be abandoned. It should read "W4 outcome (promoted or reverted)".

**L2. Quality gates are not called out as first-milestone tasks.**
- Global rules require quality gates as first-milestone tasks. The repo already has `scripts/check.sh`, CI and the pre-push hook.
- W1 should still explicitly confirm that new benchmark and harness files fall under lint and size checks. If the harness uses a language outside the size count, it must be added together with its linter, per the project CLAUDE.md.
- W1 should also confirm that new specs are added to `.gstack/pregate.json` e2e lists.

**L3. D11's "baseline OOM reproduced" and "candidate success repeated" lack counts.**
OOM near a threshold is stochastic. Specify N for each, for example 2 of 2.

**L4. The disk-peak sampling interval must be fixed and recorded per arm.**
A faster candidate shortens phase windows, which changes how likely a sampled lower bound is to catch the true peak.

## Questions that need the owner

1. **Q1 (D13):** Should the revised cap be aggregate runner-hours, wall-clock, or a per-wave allocation, and at what value? Compute is free, so this is purely a time limit.
2. **Q2 (D6/D2 for W5):** Should a bounded refresh-RSS reduction ever be able to buy a refresh slowdown or a disk increase (a D5-like exchange for refresh)? If not, should W5 be dropped now to save budget?
3. **Q3 (cutover):** Do you accept v1's refresh latency and RSS on the host as the price of cutover? Will you authorize one local refresh measurement beyond D12's single scan?
4. **Q4 (D5/D9):** Should RSS-based benefits require an absolute minimum (MiB), or apply only where RSS dominates (refresh)?

**Recommendation:** Revise the plan before starting W1 implementation. Get answers to Q1–Q4, define the uncertainty and "confirmed" rule with a budgeted A/A run, cut or restrict the W4 cache-increase arms, add a 1m kill criterion for W5, and integrate the D12 calibration and host refresh checks into W2/W6. As written, the gates cannot be applied consistently, the ladder exceeds the 6-hour cap, and several arms are predictably blocked by D2, D3 or D6, so runner time would be spent producing inconclusive or foregone results.

## Initial parent triage

- H1: the six-hour cap covers the initial harness/baseline campaign, not all waves. Job timeout caps are not predicted usage. Explicit first-campaign allocation is still needed; no automatic budget expansion.
- H2: substantive methodological gap. Uncertain regression results must be inconclusive, not automatically acceptable; define and budget baseline/base calibration before candidate trials.
- H3: potential failure mechanism not supported by current source. `attribute_link` queries the complete staged links table before insertion, so ownership does not depend on later DFS visits. Adversarial path-order fixtures remain useful proof.
- H4/H5: accepted no-regression rules remain binding. Restrict or reject experiments with cheap screening; do not silently weaken D2/D3/D6 or re-ask D7 just because failure is possible.
- H6: host refresh qualification is distinct from the authorized one full scan; a local refresh is not yet authorized.
- M1: no demonstrated further 25% disk-saving mechanism is yet established; a claim of impossibility is stronger than this review proves. Cutover versus further v1 improvements must be reported separately.
- M2: actionable owner choice: percentage-only exchange can trade substantial elapsed time for only a few MiB.
- M4: local scan is first-generation, no cache flush, installed binary hash recorded. Observer is in a separate cgroup. Owned scan conflicts with the scheduled service and ends within two hours, well before the next timer. Old DBs were not deleted. It measures workload I/O, not maximum device capability.
- M5: current-decision summary was reconciled while the reviewer ran. Historical question alternatives remain history.

Remaining findings require concrete implementation/protocol dispositions; none are claimed fixed by this receipt.
