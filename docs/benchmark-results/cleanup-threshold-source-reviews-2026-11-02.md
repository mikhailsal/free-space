# Cleanup threshold prototype: outside review disposition

Date: 2026-11-02. Reviewer: actual `claude-opus-5-5`, completed response,
sole model reported by provider. First review revision: `888b67fb9baa77bce3864301bad2bf63ed44fda4`.
The strict gstack structured parser accepted this as completed, gate **fail**.
No GitHub experiment was dispatched on this revision. The original provider
response is preserved unchanged semantically in the adjacent JSON evidence.

| Finding | Required correction before dispatch |
| --- | --- |
| P1 hardlink unit test folds its alias before asserting it | Inspect at threshold zero, also assert folding at nonzero threshold; exercise actual refresh fallback across boundary. |
| P1 sparse backing cumulative physical growth and incomplete guards | Trim after deletions outside timings; retain before/after backing receipts; guard copies and non-writer phases; forecast physical host headroom. |
| P2 setup/job deadlines and gate order | Project pre-host setup; enforce both job and step deadlines with reserves; evaluate artifact forecast before dispatch, repeat in job. |
| P2 threshold oracle shares candidate capacity | Compare independent fixture capacity/threshold and controls, plus full reference filesystem metadata. |
| P3 freshness coverage | Verify file modification extrema independently and scan-time structure internally; add aggregate extrema unit test. |
| P3 mutable runtime image | Pin base digest; archive actual smoke runtime and packages; screen loads and verifies the identical image. |
| P3 expected fixture counts absent | Record exact formula and receipt, independently check all root counts. |

Closure revision and subsequent reviewer disposition will be recorded before
dispatch. Successful unit tests or source review alone are not performance
results. The fixture allocation workaround itself remains unverified until
the smoke supplies actual inner allocation and outer physical-space evidence.

## Second full review and runtime delta

The second full review covered revision `21d29aa242e3f31a66d91992778398f733b45c90`
and accepted the original hardlink, trim, threshold, freshness, count and image
corrections. It still returned **fail**: validate the actual per-command refresh
work and expose a usable pre-dispatch forecast interface; use whole elapsed
job time for the in-job cap rather than just build time. Both require correction
before dispatch. No performance run was started.

Normal pre-push ShellCheck also rejected two intentional literal expressions.
They were rewritten without suppressing checks in revision
`f7decf817b5bf769c25956713dacc4c4a1870f50`; all normal hooks passed on that revision.
A separate bounded runtime delta review (03) returned exact `NO_FINDINGS`.
That narrow result covers the runtime save/version capture delta only; it is
not a clean verdict for the remaining full-protocol findings from review 02.

Parser caveat: review 02's bold severity markers were accepted by the installed
parser as a passing gate even though the response contains actionable findings.
The host treated the actual findings as a failing review and did not dispatch.
Provider output was not edited; closure requests use plain severity markers.
A parser pass alone is therefore not used as a no-findings assertion.

## Final closure and frozen source

Revision `d718f11fd3e64e7c8874c49d575d60067a0c9aee` fixes the remaining work-scope
and pre-dispatch/whole-job forecast findings. Review 04 then raised a narrow
coverage question because its packet omitted unchanged CLI/report/counting
implementations. The host supplied those exact unchanged sources in review 05;
the actual reviewer returned exact `NO_FINDINGS`. No source change was needed:
scan counts include the root in both product engines and both prototype passes;
the stated JSON field names and default full-scan boundary/flag exist. This is
source evidence; actual Rust runtime tests still run on GitHub before measurement.

Closure is composite: full review 02, runtime delta 03, updated harness review 04,
then schema proof 05. All 30 frozen source/dependency files have exact body-hash
coverage in the preserved packets. No product-engine diff from `origin/master`
was present in `core`, `cli/src`, `Cargo.toml` or `Cargo.lock` at source freeze.
Normal pre-push Rust, size, shell, TypeScript and Biome checks passed on the final
revision without bypass. The source-freeze JSON and capsule preserve scope and
original provider responses, including all earlier findings.

## Prospective roomy-v2 follow-up

The successful original pilot and its rejected screen admission are preserved
in the initial-smoke report/capsule. Before any million-entry observations,
revision `678e998e47020576224337b8fe8ab941f3df2bef` freezes fresh 8 GiB controls
and a 14,400-second maximum screen job inside the unchanged 216,000-second
aggregate campaign. Original 4 GiB data will not be pooled with the follow-up.

Actual Opus review 06 required host physical/available-memory evidence and
admission tests that pass between the old/new caps and fail above each new
cap. The final source records and checks 10 GiB total and available memory
and two logical CPUs before build/workload/writer/observer; fresh smoke and
current screen host are checked by their respective admission paths. Tests
exercise admitting and both boundary cases. Actual review 07 returned exact
`NO_FINDINGS`; all normal hooks passed without bypass. The roomy source-freeze
JSON/capsule preserves the exact 32-file coverage and the new reservation.
