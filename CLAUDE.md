# freespace

Linux disk-usage tool: fast scan, cached SQLite snapshot, scheduled refresh, embedded web GUI, CLI `--json` and an agent skill. Design doc: `docs/designs/free-space-disk-analyzer.md`. Deferred work: `TODOS.md`.

## Checks

Keep files within these limits:

- Source files: 500 physical lines. Test files (`*/tests/**`, `*/tests.rs`, `web/tests/**`): 1000 lines.
- Rust functions: 80 lines; Rust nesting: 4 levels.
- TypeScript functions: 80 lines; cognitive complexity: 25.
- Biome uses the recommended rules and formats with 2-space indentation and a 140-character line width. Rust uses rustfmt.

The size check counts tracked `*.rs`, `*.ts`, `*.js`, `*.css`, `*.html`, `*.sh`, and files under `.githooks/`; it skips `docs/**`. Add a new language to the count only together with its linter. Split code before crossing a limit. Never raise a limit or add an exception without the user's agreement. Put long Rust tests in a `tests.rs` submodule.

Run the shared checks with `scripts/check.sh [rust|size|shell|tsc|biome]...` from the repository root, or use the path to that script from a subdirectory (for example, `../scripts/check.sh` from `web/`). The script then runs at the repository root. With no parts it runs all five. Install the pre-push hook with `git config core.hooksPath .githooks`. The hook checks the current work tree, while CI checks the pushed commit. Run the relevant local tests while changing code; the full e2e suite, gate canary, and `refactor-guard` comparisons run on GitHub.

`/ship`'s pre-gate reads `.gstack/pregate.json`. Its runner `scripts/test-files.sh <test files>` runs each selected file with its own runner (bun, Playwright, the shots config, the guard config), shooting into ignored `web/pregate-shots/`; Rust selections run once per crate with bounded Cargo concurrency and need `setsid`; e2e, shots and guard files need `flock` (both util-linux). Add new e2e specs to every e2e list under `dependencies`; `web/tests/unit/test-files.test.ts` checks these literal paths. Rust-diff unit files run one `bun test` each (`serial_patterns`), since bun prints nothing until the run ends.

A behavior-preserving refactor PR carries the `refactor` label. `.github/workflows/refactor-label.yml` requires it for a `refactor:` title or `refactor/` branch. `.github/workflows/refactor-guard.yml` compares base and head on GitHub, including CLI answers, installation results, GUI screenshots and API requests.

## Skill routing

When the user's request matches an available skill, invoke it via the Skill tool. Route only to skills in the session's available-skills list; answer directly for quick questions or small scoped edits.

Key routing rules:
- Product ideas/brainstorming → invoke /office-hours
- Strategy/scope → invoke /plan-ceo-review
- Architecture → invoke /plan-eng-review
- Design system/plan review → invoke /design-consultation or /plan-design-review
- Full review pipeline → invoke /autoplan
- Bugs/errors → invoke /investigate
- QA/testing site behavior → invoke /qa or /qa-only
- Code review/diff check → invoke /review
- Visual polish → invoke /design-review
- Ship/deploy/PR → invoke /ship or /land-and-deploy
- Save progress → invoke /context-save
- Resume context → invoke /context-restore
- Author a backlog-ready spec/issue → invoke /spec
