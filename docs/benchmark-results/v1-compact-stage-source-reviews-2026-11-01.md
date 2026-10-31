# Compact staging: source review and measurement admission

The primary research source is `601ce381ac229923d2c791fd8e7c510f7b3f1dfc`, with matched harness control `5e1a563d871a80a3221999c360d1621a84adcf58` and product baseline `e8265778ef78ec4acd9da1375eedb0be37a123f5`. Normal bounded Git hooks passed and the published source hashes match the reviewed files. No performance or product qualification is implied.

The candidate retains two 8 MiB SQLite cache targets. Its temporary child and hardlink-owner indexes omit the explicit terminal integer primary key, relying on SQLite's implicit rowid for the same tie order. A tiny SQLite mechanism check found fewer index pages; this is neither an application benchmark nor evidence of lower total RAM. The optional 4 MiB source remains a separate cache experiment and requires a matched representative-scale 8-versus-4 comparison before acceptance.

The requested `claude-opus-5-5` reviews completed with findings. Fixes and source-based disagreements are recorded in the [raw review and disposition record](v1-compact-stage-source-reviews-2026-11-01.json). Focused final reviews returned `NO_FINDINGS` for two test-fixture corrections and the exact cleanup-sync target assertion. That does not constitute an unqualified final review of every change. The last canonical-baseline negative assertion was corrected exactly as the reviewer prescribed and inspected by the parent.

The changed-full harness now enforces the canonical baseline, bounds SQLite lookups, normalizes signed inode/device encoding, binds previous-snapshot identity, rejects stale case evidence and tests orchestration failure propagation. Static inspection confirmed deterministic hardlink ownership and that refresh preserves previous history. A real GitHub smoke must still verify these assumptions against product-generated snapshots; synthetic schemas and source inspection do not replace it. The candidate's actual bundled-SQLite planner and complete Rust tests must pass on GitHub before measurements.

Two review calls initially omitted the required JSON adapter and produced invalid responses. Their errors are preserved without attributing coverage. Corrected explicit-adapter calls passed transport and structured-result validation using the requested actual model. The earlier provider quota failure is preserved in the original cleanup repair record.

The [archive](v1-compact-stage-source-reviews-2026-11-01.tar.gz) includes exact review packets, results, source hashes, focused test logs, publication receipts and an indexed checksum for every payload. Local validation used mocked Python checks and capped normal hooks; no Rust tests or application runs occurred locally.

The local nightly timer is temporarily stopped to keep heavy research on GitHub. Its persistent setting can trigger a catch-up scan when restored, so restoration remains a deliberately scheduled operational action. No installed binary, backend or configuration change has been made.
