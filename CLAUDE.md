# freespace

Linux disk-usage tool: fast scan, cached SQLite snapshot, scheduled refresh, embedded web GUI, CLI `--json` and an agent skill. Design doc: `docs/designs/free-space-disk-analyzer.md`. Deferred work: `TODOS.md`.

## Checks

Until the gates PR of `docs/designs/code-quality-gates.md`:
- `web/biome.json`: the format is checked by the pre-gate's `web` guard, not yet by CI (`cd web && bunx biome format .`; `--write` fixes it). Its two lint rules (functions over 80 lines, cognitive complexity over 25) report 6 errors on purpose until the debt PRs land; the gates PR runs `biome ci --error-on-warnings` in CI and the pre-push hook.
- `/ship`'s pre-gate reads `.gstack/pregate.json`. Its runner `scripts/test-files.sh <test files>` runs each file with its own runner (bun, Playwright, the shots config). A new e2e spec goes into every e2e list under `dependencies`; `web/tests/unit/test-files.test.ts` fails until it does.

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
