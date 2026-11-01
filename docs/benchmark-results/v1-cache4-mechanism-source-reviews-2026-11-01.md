# Cache-4 mechanism source reviews — 2026-11-01

Published research source `1af549a8f1d834fb292e78a6cbd48ad9d12252cd` changes the per-connection SQLite staging cache target from 8 MiB to 4 MiB, a one-line delta from source `63d98cc8ff88a54ab9687c5fb93fffd930e33d26`. Normal pre-push checks passed and the reviewed source hash remained unchanged. No local Rust tests or app runs, and no GitHub runtime cache measurement, are represented here.

The initial `claude-opus-5-5` review completed, but its structured validation **failed**: it placed `NO_FINDINGS` at the opening rather than providing a valid concluding verdict, and identified missing context. Its raw response is retained; it is not a clean outside-review verdict. The follow-up review completed and passed structured validation with a **P3 finding**, not `NO_FINDINGS`: the shared cache constant also changes v2 disk staging. The owner accepted that scope finding for this v1 research; v2 performance remains unmeasured and no v2 guarantee is made.

The 4 MiB source and the earlier 8 MiB source remain unqualified. The separate private admission helper is not covered by these source reviews. The included whole-memory audit is independent context, not causal proof.

The archive contains 30 payloads and an internal SHA-256 index; every member was reread and verified.
