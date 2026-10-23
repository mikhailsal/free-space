# Code quality gates

Status: reviewed by /plan-eng-review 2026-10-23 (decisions D1-D27, ledger below); ready to implement. Date: 2026-10-23.

## Problem

Nothing stops a file or a function from growing without bound. CI today runs `cargo fmt --check`, `cargo clippy -D warnings` (default lint set), `cargo audit`, `tsc --noEmit` (strict), `shellcheck` and the test suites. None of them looks at size or structure, and `CLAUDE.md` has no rules for it. The result, with an AI agent writing most of the code:

- `web/src/app.ts` is 1555 lines; one class, `App`, is 1490 of them.
- 13 Rust functions are over 80 lines; the longest, `write_inner` in `core/src/snapshot/writer.rs`, is 203.

The new global rule (`~/.claude/CLAUDE.md`, "Code quality gates from day one") asks every project for strict linters, size limits with a checker, and enforcement in CI and a pre-push hook. This plan applies it to freespace.

## Goals

1. A file or function over its limit fails CI and the pre-push hook, whoever wrote it.
2. The debt is paid first, so the gates land with every limit on and zero exceptions (D14). An exception later needs the user's agreement; only then does an allowlist file appear.
3. The splits change no behavior, and that is checked, not assumed: characterization tests first, then byte-identical CLI answers and pixel-identical GUI shots, base against head, on GitHub (D15, D16, D20, D23).
4. Every gate is proven to fire, now and after later edits (D17, D21).
5. The limits and the commands are written in `CLAUDE.md` and `CONTRIBUTING.md`.

Not a goal: test coverage thresholds, mutation testing, a code-style rewrite. Heavy checks run on GitHub's free runners, not on the dev box (D23).

## Limits

| What | Limit | Checked by |
|---|---|---|
| Source file | 500 lines | `scripts/check-size.sh`, every physical line, inline `#[cfg(test)]` included (D11); `.css` too (D12) |
| Test file | 1000 lines | same; test files are `*/tests/**`, `*/tests.rs`, `web/tests/**` |
| Function (Rust) | 80 lines | clippy `too_many_lines` (pedantic, enabled in `[workspace.lints.clippy]`, `too-many-lines-threshold = 80` in `clippy.toml`), code lines without blanks and comments |
| Nesting (Rust) | 4 levels | clippy `excessive_nesting`, `excessive-nesting-threshold = 4` (D3); `cognitive_complexity` is not used, its own docs call it misleading |
| Function (TS) | 80 lines | Biome `noExcessiveLinesPerFunction` `maxLines: 80`, src and tests (D2, D6) |
| Cognitive complexity (TS) | 25 | Biome `noExcessiveCognitiveComplexity` (D4) |
| Lint set (TS, CSS) | Biome `"preset": "recommended"`, no suppressions | `biome ci --error-on-warnings` (D27) |
| Format | rustfmt; Biome `lineWidth: 140`, indent 2 spaces (D5) | `cargo fmt --check`; `biome ci --error-on-warnings` |

`check-size.sh` counts tracked `*.rs *.ts *.js *.css *.html *.sh` and `.githooks/*`, outside `docs/**` (D10). Everything else (lock files, `skill/schema.json`, images, fonts, docs, licences) is not counted. A new language joins the list together with its linter.

## Current violations (measured 2026-10-23)

Source files over 500 lines:

| File | Lines | Of which inline `#[cfg(test)]` |
|---|---|---|
| `web/src/app.ts` | 1555 (1657 after the width-140 format) | |
| `web/src/app.css` | 1144 | |
| `core/src/refresh.rs` | 572 | 13 |
| `cli/src/install.rs` | 569 | |
| `core/src/scan/walk.rs` | 563 | 89 |
| `core/src/scan/tree.rs` | 559 | 134 |
| `cli/src/query.rs` | 524 | |

No test file is over 1000 (`core/tests/queries.rs` is 931).

Rust functions over 80 lines (clippy count): `snapshot/writer.rs:119` 203, `refresh.rs:64` 175, `refresh.rs:394` 147, `queries/diff.rs:169` 141, `cli/src/install.rs:83` 132, `scan/tree.rs:281` 131, `scan/walk.rs:301` 124, `cli/src/status.rs:129` 115, `scan/walk.rs:137` 109, `refresh.rs:248` 107, `queries/status.rs:27` 103, `cli/src/doctor.rs:259` 103, `job.rs:151` 101. The next one down is 79, so the 80 limit cuts at a natural gap. Nesting over 4: none.

Web functions over a Biome limit (Biome's count, after the width-140 format): `App.notices` (`web/src/app.ts:925`) 88 lines; `App.renderSummary` (`app.ts:1310`) complexity 64; `inspector` (`web/src/inspector.ts:75`) complexity 46; `h` (`web/src/dom.ts:7`) complexity 29; the mock API's `handle` (`web/tests/e2e/mock.ts:216`) 126 lines and complexity 34.

## Tasks

Order (D14): a format PR, the guards, the debt PRs, then the gates PR. Each step lands on master before the next starts, except the two debt lanes, which run in parallel (see Worktree parallelization below).

### PR 1: Biome format and stable screenshots

- **F1.** Add `@biomejs/biome` 2.6.2, pinned exactly, and `web/biome.json` with the rules and format of the Limits table over `web/src`, `web/tests` and the playwright configs. Run the formatter once (12 files change). The lint is not enforced yet.
- **F2.** `web/tests/shots/readme.spec.ts` takes its shots with `animations: "disabled"` (the header disk bar's `transition: width 400ms`, `web/src/app.css:209`, made two runs differ by 16 px); the README PNGs are regenerated once (D16).

### PR 2: refactor guard on GitHub (D23)

- **R1.** `.github/workflows/refactor-guard.yml`, on pull requests labeled `refactor` (every debt PR carries it). Three jobs:
  - `rust-diff`: builds base (merge-base with master) and head, runs base against base and then base against head over `status`, `ls`, `top`, `find`, `diff`, `doctor`, text and `--json`, on one fixture snapshot; scan-path PRs also compare fresh scans. Byte-identical after the written normalization of D20: `--json` drops `snapshot_id`, every time field (`scanned_at`, `*_ts`, ages, `*_seconds`) and live `fs[].total/used/avail`; text output masks the same values; both binaries run back to back on one copy of the snapshot dir. The base-against-base run must pass first, or the job fails as unreliable.
  - `pixel-diff`: builds and serves base and head fresh, shoots every view and state the e2e mock reaches at 1280, 700 and 360 px, fails on any differing pixel, uploads before, after and diff PNGs.
  - `coverage`: `cargo llvm-cov`, uploads lcov and per-function line counts of the touched files. Non-blocking.

### PR 3…n: the debt, regression contract first

Every debt PR is labeled `refactor`, intends no behavior change, and passes: test names before = after plus the new ones; all CI jobs green; `refactor-guard` green.

Rust lane:

- **D0 (D15, D25).** Before a function or file is split, tests for its branches that coverage shows unreached (about 8 tests or table rows asserting labels, values and codes): the text `freespace status` (`cli/src/status.rs:129` `details`, 0/113 lines), the text tables of `cli/src/query.rs` (175/346), the failing checks of `doctor` (`cli/src/doctor.rs:259`, 35/83), refresh of a deleted folder (`core/src/refresh.rs:270`), a path that resolves elsewhere (`:445`), root gone (`:455`), diff error paths (`core/src/queries/diff.rs:180,254`), scan with no roots (`core/src/job.rs:155`). Plus a sudo script like `scripts/fault-fs.sh`, run in the rust job: mount a tmpfs inside a fixture root, scan (the mount shows as a mount with 0 bytes, `core/src/scan/walk.rs:392`), refresh the mount point (usage error with its text, `core/src/refresh.rs:402`). These land green on the unsplit code; then the split.
- **D3.** The 13 Rust functions over 80, split by phase (load, walk, insert, index, publish...), longest first.
- **D4.** The 5 Rust files over 500: `refresh.rs` by phase; `install.rs` and `query.rs` by subcommand; `walk.rs` and `tree.rs` move their inline tests to `tests.rs` submodules, which brings both under 500 (module paths and test names stay; `cargo test -- --list` before and after).
- **Scan path (D18).** A debt PR that touches `walk.rs`, `tree.rs`, `writer.rs` or `refresh.rs` lists in its body every new `clone()`, `collect()` or allocation inside a per-entry loop, or says there is none. No timing; the timed check is in `TODOS.md` (D24).

Web lane:

- **W0 (D16, D19).** Before `App` is split: one e2e table test over ArrowUp, ArrowRight, Home, PageUp, PageDown and Backspace (`browse.spec.ts`); Show more (`app.ts:773`) and cancelling a search (`app.ts:684`) in `find.spec.ts`. `web/tests/e2e/mock.ts` gains a hold-and-release for one request, and e2e gains: Browse request held, press `t`, Top renders, release Browse, the page still shows Top (rows, URL, header); a held Find cancelled, then released, changes nothing.
- **D1.** Split `App` (`web/src/app.ts`) by responsibility: keyboard and commands, one controller per view (Browse, Top files, Find, Changes), notices, layout, with `app.ts` a thin shell, every file ≤ 500. One request coordinator stays (today's `load()`: `loadSeq` and the `AbortController`, `app.ts:524-546`); view controllers fetch through it and never apply a response themselves (D19). `App.notices` and `App.renderSummary` come under the limits here.
- **D2.** Split `web/src/app.css` at its section comments into about four files under 500 (D12); `main.ts` imports them in today's order.
- **W1.** `inspector` (`web/src/inspector.ts:75`), `h` (`web/src/dom.ts:7`) and the mock's `handle` (`web/tests/e2e/mock.ts:216`) under the Biome limits.
- **W2 (D27).** After D1 and D2, fix every finding of Biome's recommended preset without a suppression: 87 at the format PR (noNonNullAssertion 57, noDescendingSpecificity 15, noControlCharactersInRegex 4, useTemplate 4, useOptionalChain 3, and 4 single ones), fewer once D1/D2 have rewritten app.ts and app.css.

### Last PR: the gates

- **G1 `scripts/check-size.sh`.** Files from `git ls-files -z` in the D10 scope; classify (source or test), count every physical line (a last line without `\n` counts), fail over 500 / 1000 with the path, its count and the limit. Outside a git work tree, or with zero files in scope, exit 1. A tracked file deleted in the work tree is skipped. No allowlist (D14).
- **G2 Rust lints.** `Cargo.toml` `[workspace.lints.clippy]` `too_many_lines = "warn"`; `clippy.toml` `too-many-lines-threshold = 80`, `excessive-nesting-threshold = 4`; `[lints] workspace = true` in `core/Cargo.toml` and `cli/Cargo.toml` (neither has a `[lints]` table today, and without it the lints never fire). CI's `-D warnings` makes them failures.
- **G3 Biome lint on.** `web/biome.json` switches from `"preset": "none"` to `"preset": "recommended"` (D27), keeping the two limits; `biome ci --error-on-warnings`, so a later stale suppression fails too.
- **G4 `scripts/check.sh [rust|size|shell|tsc|biome]...`** (D8). `rust`: `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`; `size`: `check-size.sh`; `shell`: shellcheck over `install.sh`, `scripts/*.sh`, `.githooks/pre-push`; `tsc`: `tsc --noEmit` in `web/`; `biome`: `biome ci --error-on-warnings` in `web/`. No argument runs all five; an unknown part exits 2 and lists the parts; a failing part makes the exit 1 though later parts pass; runs at the repo root from any subdirectory. Before a part runs, `command -v` for each tool it needs; a missing one exits 1 with the tool's name and its install command, locally and in CI (D9).
- **G5 `.githooks/pre-push`** execs `scripts/check.sh` (about 3-8 s warm, measured). Installed with `git config core.hooksPath .githooks`. It checks the work tree, not the pushed commits; CI is the gate.
- **G6 CI.** Every job calls `scripts/check.sh`, never the tools directly: `rust` job `rust`, new `quality` job `size biome` plus `scripts/test-check.sh`, `shellcheck` job `shell`, `web` job `tsc`. Tests stay inline in their jobs. The `.gstack/pregate.json` guards (added in PR 1 for /ship's pre-gate) call the same parts (`web`: `tsc biome`; `rust`: `rust`, then the tests; `shell`: `shell`, which covers `.githooks/pre-push`), so the pre-gate, the hook and CI share one entrypoint.
- **G7 Proof the gates work.** `scripts/test-check.sh` (T4): throwaway git repos from `mktemp -d`, the real scripts, stub `cargo`, `bunx` and `shellcheck` on `PATH`; rows for every rule of G1 and G4. `scripts/canary.sh` (D17, D21): in a temporary git worktree of HEAD, never the working tree, one violation per run at limit + 1 (an 81-line function in every workspace member from `cargo metadata`, a block nested 5 deep, an 81-line TS function in `web/src` and in `web/tests`, a complexity-26 function, an unformatted file, an unused `biome-ignore`, a 501-line `.rs` and `.css`, a 1001-line test file); the worktree is reset (`git checkout -- . && git clean -fdq`) before each injection; a pass needs exit 1, the rule's name and the injected file's path in one diagnostic. Runs in the rust job after clippy, reusing its target, with `bun install --frozen-lockfile` in `web/` for Biome.
- **G8 Docs.** `CLAUDE.md`: the limits, the counted extensions, `scripts/check.sh` and its parts, the hook install command, "split before you cross a limit; never raise a limit or add an exception without the user's agreement", long tests go to a `tests.rs` file, and what runs where (locally: the hook and the tests of the code being changed; full e2e, the canary and `refactor-guard` run on GitHub). `CONTRIBUTING.md`: the hook, the tools with their install commands, the hook checks the work tree and CI is the gate.

## Open questions (all resolved)

- **Q1 Inline Rust tests.** Count the whole file (D11); long tests move to `tests.rs`.
- **Q2 CSS.** 500, like code (D12).
- **Q3 Function ratchet.** Moot: the gates land with zero exceptions (D14).
- **Q4 Web linter.** Biome (D2): typescript-eslint 8.72 needs typescript < 6.1 and the repo is on 7; Biome has its own parser, both rules and a formatter.
- **Q5 Hook speed.** Measured 3-8 s warm for all five parts; the hook checks the whole workspace (PF1).

## Eng review (plan-eng-review, 2026-10-23)

Target: `docs/designs/code-quality-gates.md` (this plan). Reviewer: Claude (plan-eng-review). Commit: 41f5440. Line references to this file below (`code-quality-gates.md:NN`) point into the draft as reviewed; the sections above now hold the accepted design.

### Reality rows

| premise | claim | file:line receipt | verdict |
|---|---|---|---|
| CI checks | fmt, clippy -D warnings, audit, tsc, shellcheck, tests run today | .github/workflows/ci.yml:24,25,44,101,109 | verified |
| app.ts size | 1555 lines, App class 1490 | web/src/app.ts:65 (wc -l, awk) | verified |
| long Rust fns | 13 over 80, write_inner 203, next 79 | core/src/snapshot/writer.rs:119, cli/src/install.rs:468 (clippy threshold 60/80) | verified |
| files over 500 | 7 source files, none of tests over 1000 | wc -l over git ls-files | verified |
| inline tests | 12 / 88 / 133 lines | core/src/refresh.rs:560, core/src/scan/walk.rs:475, core/src/scan/tree.rs:426 | false (13 / 89 / 134) |
| expect fails when split | #[expect] fails clippy -D warnings once unfulfilled | probe crate, clippy 0.1.99 | verified |
| clippy cognitive_complexity | usable complexity gate | cargo clippy --explain cognitive_complexity | false (restriction, docs warn) |
| TS 7 | repo on TypeScript 7 native | web/package.json:16 | verified |
| typescript-eslint on TS 7 | open | web search: peer range < 6.1, support not planned before 7.1 API | false |
| Biome per-function rule | open | biomejs.dev noExcessiveLinesPerFunction (complexity group) | verified |
| hook clippy ~1 min warm | estimate | none | unverified |
| css imported per file | main.ts imports 3 css files | web/src/main.ts:1-3 | verified |
REALITY: premise-table finding unverified=1 (hook clippy time); 3 false claims corrected docs/designs/code-quality-gates.md:81
REALITY: already-done pass no commits on origin/master since 2026-10-23 17:00 in the touched paths docs/designs/code-quality-gates.md:43
REALITY: surface-check finding cognitive_complexity is a restriction lint its own docs advise against; typescript-eslint cannot run on TS 7; too-many-lines-threshold and #[expect] verified by probe docs/designs/code-quality-gates.md:48
REALITY: binding-decisions pass no branch decisions on lint or CI; no Binding decisions section in the plan docs/designs/code-quality-gates.md:1
REALITY: numbers finding inline test counts are 13/89/134, not 12/88/133; hook time is an estimate docs/designs/code-quality-gates.md:35
REALITY: deferred-asks pass no linked issue; the user's asks (linters, file and function limits, CI and pre-push) are G1-G5 docs/designs/code-quality-gates.md:45
REALITY: pr-count pass default one PR per wave; this plan is the gates PR plus D1-D4 debt PRs docs/designs/code-quality-gates.md:56

PLAN_REALITY: phase=eng rows=7/7 missing=none unverified=1 verdict=complete

### Scope record

Complexity: PR 1 touches about 27 files (5 new: `scripts/check-size.sh`, `quality/size-allowlist.txt`, `clippy.toml`, the web linter config, `.githooks/pre-push`; edited: `Cargo.toml`, `.github/workflows/ci.yml`, `web/package.json`, `bun.lock`, `CLAUDE.md`, `CONTRIBUTING.md`, 10 Rust files for 13 `#[expect]`, 2 web files for 3 inline disables), estimate from the task list. No new classes or services. Gate B tripped on files (8+).

feature answers: none (no cuts or deferrals proposed); structure: A Original arrangement (D1, answered 2026-10-23: "Original arrangement"); accepted scope: G1-G6 (a separate CI `quality` job running `scripts/check-size.sh` and the web lint, clippy lints picked up by the existing `rust` job, `.githooks/pre-push`, docs) and debt PRs D1-D4, ordered debt first and gates last (D14); `quality/size-allowlist.txt` is created only when the user agrees to a first exception (D14); pending remedies: none (S1-S5 resolved, see the ledger).
History: as answered, the accepted scope read "G1-G6 as written (allowlist at `quality/size-allowlist.txt`, ...) and debt PRs D1-D4; pending remedies: S1, S2, S3, S4." D14 (Debt first) changed the order and dropped the allowlist file.

Factual correction (no behavior change, no question): inline `#[cfg(test)]` sizes are 13 / 89 / 134 lines, not 12 / 88 / 133 (`core/src/refresh.rs:560`, `core/src/scan/walk.rs:475`, `core/src/scan/tree.rs:426`, counted to EOF). The table above and D4 are amended.

### Scope Challenge findings

1. S1 [P1] (confidence: 9/10) `docs/designs/code-quality-gates.md:63` (G3, Q4) and `web/package.json:16` `"typescript": "^7.0.2"`. The ESLint path needs typescript-eslint, whose latest release (8.72.0, npm registry 2026-10-23) declares peer `typescript: >=4.8.4 <6.1.0`, so it does not run on the repo's TypeScript 7. Biome 2.6.2 has its own parser and both rules: probe on `web/src` at maxLines 80 flags one function, `App.notices` (`web/src/app.ts:905`, 82); 21 files in 0.11 s. Reviewer: Claude.
2. S2 [P2] (confidence: 9/10) `docs/designs/code-quality-gates.md:62` (G2) `cognitive_complexity = "warn"`. `cargo clippy --explain cognitive_complexity`: the lint sits in `restriction` and its docs say it is kept there so as not to mislead users; the metric has known false positives. Biome's `noExcessiveCognitiveComplexity` at 25 flags `web/src/app.ts:1254` (64), `web/src/inspector.ts:71` (46), `web/src/dom.ts:7` (29). Reviewer: Claude.
3. S3 [P2] (confidence: 8/10) `~/.claude/CLAUDE.md` rule 1 asks for "linters and formatters"; the web side has no formatter (`web/package.json` has no format script, nothing in `.github/workflows/ci.yml:44` checks web formatting) and the plan does not add one. Reviewer: Claude.
4. S4 [P3] (confidence: 9/10) `~/.claude/CLAUDE.md` rule 1 names `clippy::cognitive_complexity` as an example, which S2's evidence argues against; the global rule would push the same lint into every new project. Reviewer: Claude.
5. S5 [P2] (confidence: 9/10) `web/tests/e2e/mock.ts:216` (the mock API's request handler). Lint scope for `web/tests` is unspecified in G3. Biome probe after the approved width-140 format: 126 lines (limit 80) and cognitive complexity 34 (limit 25); no other test function is over either limit. Rust tests are already under the same limits (CI runs clippy `--all-targets`; none of the 13 long functions is a test). Reviewer: Claude.
6. Fact: TS function counts in "Current violations" came from an awk count of physical lines. Biome excludes punctuation-only lines: `App.buildLayout` 91 physical = 79 by Biome, `App.notices` 102 = 82, `inspector` 95 = 80 (77 with `skipBlankLines`). Correction recorded, no question.

## Decision ledger

### S1: web linter for TypeScript 7
Finding: S1, P1, confidence 9/10, `docs/designs/code-quality-gates.md:63` and `web/package.json:16`, reviewer Claude.
Plan baseline: G3 "Add a linter with `max-lines-per-function` 80 and a complexity rule; the three long functions get an inline disable"; tool open (Q4). Not approved beyond the accepted scope (scope record, D1): a web linter exists, runs in the CI `quality` job and the pre-push hook, function limit 80.
Runtime evidence: typescript-eslint 8.72.0 peer `typescript >=4.8.4 <6.1.0` (registry.npmjs.org/typescript-eslint/latest); Biome 2.6.2 probe: `biome lint` with `noExcessiveLinesPerFunction` maxLines 80 on `web/src` flags `src/app.ts:905` (82) only, 0.11 s; `web/package.json:16` TypeScript ^7.0.2.
Comparison grid:
| Choice | Current | A Biome | B ESLint + TS 5.9 for lint | C no web linter now |
|---|---|---|---|---|
| S1 web linter tool | none | `@biomejs/biome` 2.6.2 pinned, `web/biome.json`, `noExcessiveLinesPerFunction` maxLines 80 | `eslint` 9 + `typescript-eslint` 8.72 + a second dev dependency `typescript@5.9` used only by the linter, `max-lines-per-function` 80 | none; G3 moves to TODOS.md |
| Function limit 80 (TS) | approved (scope record) | kept, Biome counting | kept, ESLint counting | not enforced for TS |
| Inline disables | approved with G3 | 1 (`App.notices`) | ~3 by physical count | none |
| CI `quality` job, pre-push | approved (scope record) | run `biome lint` | run `eslint` | size check only |
| S2 complexity rule | pending | pending | pending | pending |
| S3 web formatter | pending | pending | pending | pending |
Question D2:
D2 — Какой линтер ставим для веба?
Project/branch/task: master, план docs/designs/code-quality-gates.md, задача G3 (линтер для web/).
ELI10: Чтобы проверять длину функций в TypeScript, нужен линтер. Обычный выбор, ESLint, читает TypeScript через плагин typescript-eslint, а тот работает только с TypeScript до 6.1. У нас TypeScript 7 (web/package.json:16), поэтому ESLint придётся кормить отдельной старой копией TypeScript. Biome разбирает код сам и уже умеет ограничивать длину функций.
Stakes if we pick wrong: с ESLint в репо живут два компилятора TypeScript, которые могут разойтись в разборе нового синтаксиса; без линтера длинные TS-функции не ловятся вообще.
Recommendation: A) Biome because он работает на TS 7 без второго компилятора, нужное правило у него есть и проверено на нашем коде (0,1 с, ловит App.notices, 82 строки).
Completeness: A=10/10, B=8/10, C=4/10
Pros / cons:
A) Biome (recommended)
  ✅ Один бинарник без зависимостей от версии TypeScript, весь web/src за 0,11 с
  ✅ noExcessiveLinesPerFunction проверен на нашем коде: при 80 ловит ровно App.notices
  ❌ Считает строки иначе, чем clippy: строки из одних скобок не идут в счёт
B) ESLint + TS 5.9 для линта
  ✅ Самый распространённый линтер, огромный выбор правил и плагинов
  ✅ max-lines-per-function считает физические строки, как wc -l
  ❌ Вторая копия TypeScript (5.9) в devDependencies только ради линтера, плюс десятки пакетов (оценка)
C) Без веб-линтера пока
  ✅ Ноль новых зависимостей; app.ts всё равно ограничен проверкой размера файлов
  ✅ Можно вернуться, когда typescript-eslint поддержит TS 7
  ❌ Функции в TS не ограничены: 300-строчный метод пройдёт CI, пока файл ≤ 500
Net: Biome даёт нужное правило на нашем TS без костылей; ESLint даёт привычность ценой второго компилятора. Effort: A human ~2 h / CC ~10 min; B human ~4 h / CC ~20 min; C 0.
Header: Web linter
Options:
A) Biome
`@biomejs/biome` 2.6.2 pinned in web/package.json, web/biome.json with noExcessiveLinesPerFunction maxLines 80; run in the CI quality job and pre-push; one inline disable (App.notices). Complexity rule (S2) and formatter (S3) decided separately. human ~2 h / CC ~10 min.
B) ESLint + TS 5.9 for lint
eslint 9 + typescript-eslint 8.72 + typescript@5.9 as a second devDependency used only by the linter; max-lines-per-function 80; ~3 inline disables. S2 and S3 decided separately. human ~4 h / CC ~20 min.
C) No web linter now
Drop G3 from PR 1 and add a TODO to revisit when typescript-eslint supports TS 7; TS functions are not length-checked, web files still get the 500-line file limit. The web part of S2 has no tool to run in; S3 is decided separately.

State: approved
Actual answer: A) Biome (D2, answered 2026-10-23: "Biome")
Accepted scope: `@biomejs/biome` 2.6.2 pinned in web/package.json, web/biome.json with noExcessiveLinesPerFunction maxLines 80; run in the CI quality job and pre-push. App.notices is split in a debt PR before the gates PR (D14), so no inline disable. Complexity rule (S2) and formatter (S3) decided separately.
History: D2 accepted scope as answered: "... run in the CI quality job and pre-push; one inline disable (App.notices). ... human ~2 h / CC ~10 min." The inline disable was superseded by D14 (Debt first).


### S2a: complexity lint for Rust
Finding: S2, P2, confidence 9/10, `docs/designs/code-quality-gates.md:62` (G2 `cognitive_complexity = "warn"`), reviewer Claude.
Plan baseline: G2 as written: `cognitive_complexity = "warn"` in `[workspace.lints.clippy]`, default threshold 25. Not approved (scope record accepted G2 as a task; this lint is the finding's subject and stays pending). Approved and fixed in every column: `too_many_lines` at 80 (scope record).
Runtime evidence: `cargo clippy --explain cognitive_complexity` (clippy 0.1.99): "We used to think it measured how hard a method is to understand", left in `restriction` "so as to not mislead users", suggests `excessive_nesting` and `too_many_lines` instead (rust-clippy#3793). Probe with CLIPPY_CONF_DIR: at threshold 25, 0 functions; at 15, 8 (max 22, `cli/tests/cli.rs:129`; `core/src/scan/walk.rs:301` 19). `excessive_nesting` (warn-by-default, active only with `excessive-nesting-threshold` set): threshold 4 flags 0, threshold 3 flags 80 blocks.
Comparison grid:
| Choice | Current | A Length only | B Nesting 4 | C Cognitive 25 | D Both |
|---|---|---|---|---|---|
| S2a `cognitive_complexity` | not enabled; plan proposes warn 25 | not enabled | not enabled | warn, threshold 25 | warn, threshold 25 |
| S2a `excessive_nesting` | not enabled | not enabled | `excessive-nesting-threshold = 4` in clippy.toml | not enabled | threshold 4 |
| Violations today | n/a | 0 | 0 | 0 | 0 |
| `too_many_lines` 80 | approved (scope record) | kept | kept | kept | kept |
| CLAUDE.md lists the limits (G6) | approved (scope record) | length only | adds nesting 4 | adds cognitive 25 | adds both |
| S2b web complexity rule | pending | pending | pending | pending | pending |
| S3 web formatter | pending | pending | pending | pending | pending |
Question D3:
D3 — Чем ограничивать сложность Rust-функций?
Project/branch/task: master, план docs/designs/code-quality-gates.md, задача G2 (clippy).
ELI10: План включает в clippy правило cognitive_complexity: оно ставит каждой функции «балл сложности» и падает выше 25. Но авторы clippy пишут в его описании: «мы раньше думали, что оно измеряет, насколько метод трудно понять», и держат его выключенным, чтобы не вводить людей в заблуждение. Взамен они советуют excessive_nesting: оно просто запрещает вкладывать блоки глубже заданного уровня. У нас сейчас максимум 4 уровня, и ни одна функция не набирает 25 баллов, так что сегодня оба правила ничего не ловят; вопрос в том, что будет держать новый код.
Stakes if we pick wrong: с cognitive_complexity CI будет падать по метрике, которой не доверяют её авторы; без правил глубоко вложенная функция до 80 строк пройдёт.
Recommendation: B) Nesting 4 because это замена, которую предлагает сам clippy, глубина вложенности считается однозначно, а нарушений сегодня ноль.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Length only
  ✅ Ноль новых правил: остаётся too_many_lines 80, которое уже в плане
  ✅ Нечего настраивать и объяснять: длину функции понимает каждый
  ❌ Функция на 70 строк с шестью уровнями if/match/for пройдёт CI
B) Nesting 4 (recommended)
  ✅ clippy сам советует его вместо cognitive_complexity; одна строка в clippy.toml
  ✅ Сегодня 0 нарушений (при пороге 3 было бы 80 блоков), долга не добавляет
  ❌ Порог 4 равен текущему максимуму: глубже вложить нельзя, даже если так читаемее
C) Cognitive 25
  ✅ Ловит не только вложенность, но и длинные цепочки условий и break/continue
  ✅ Сегодня 0 нарушений: максимум 22 (тест cli/tests/cli.rs:129)
  ❌ Описание clippy прямо говорит, что метрика вводит в заблуждение (rust-clippy#3793)
D) Both
  ✅ Две разные страховки: глубина вложенности и балл сложности
  ✅ Сегодня 0 нарушений по обоим правилам
  ❌ Тянет спорную метрику ради того, что уже ловят вложенность и длина
Net: что держит новый Rust-код помимо длины: однозначная глубина вложенности, спорный балл, оба или ничего. Effort: A 0; B, C, D human ~10 min / CC ~2 min.
Header: Rust complexity
Options:
A) Length only
Drop `cognitive_complexity` from G2; Rust functions are bounded only by too_many_lines 80 (approved). Web complexity (S2b) and formatter (S3) decided separately.
B) Nesting 4
Drop `cognitive_complexity`; add `excessive-nesting-threshold = 4` to clippy.toml (warn-by-default lint, CI's -D warnings enforces it, 0 violations today); CLAUDE.md lists it with the limits. S2b and S3 decided separately. human ~10 min / CC ~2 min.
C) Cognitive 25
Keep G2 as written: `cognitive_complexity = "warn"` in [workspace.lints.clippy], default threshold 25, 0 violations today (max 22); CLAUDE.md lists it. S2b and S3 decided separately. human ~10 min / CC ~2 min.
D) Both
`cognitive_complexity = "warn"` at 25 and `excessive-nesting-threshold = 4`, 0 violations today; CLAUDE.md lists both. S2b and S3 decided separately. human ~10 min / CC ~2 min.

State: approved
Actual answer: B) Nesting 4 (D3, answered 2026-10-23: "Nesting 4")
Accepted scope: Drop `cognitive_complexity`; add `excessive-nesting-threshold = 4` to clippy.toml (warn-by-default lint, CI's -D warnings enforces it, 0 violations today); CLAUDE.md lists it with the limits. S2b and S3 decided separately. human ~10 min / CC ~2 min.
History: none

### S2b: complexity rule for the web
Finding: S2, P2, confidence 9/10, `docs/designs/code-quality-gates.md:63` (G3 "and a complexity rule", threshold unspecified; Limits table: "the web linter's equivalent" of 25), reviewer Claude.
Plan baseline: G3 "a complexity rule"; not approved beyond the S1 answer (D2: Biome, "Complexity rule (S2) ... decided separately"). Fixed in every column: Biome 2.6.2, noExcessiveLinesPerFunction 80 with one suppression (D2); Rust nesting 4 (D3).
Runtime evidence: Biome 2.6.2 probe `noExcessiveCognitiveComplexity` on `web/src`: at 25, 3 functions: `src/app.ts:1254` 64, `src/inspector.ts:71` 46, `src/dom.ts:7` 29; at 15 (Biome's default), 13 functions. Biome has no general block-nesting rule (622 rules in configuration_schema.json; nesting rules cover only callbacks, test suites, switch, ternaries, template literals). An unused `biome-ignore` gives `suppressions/unused` "Suppression comment has no effect" (warning); `biome lint --error-on-warnings` exits 1 on it.
Comparison grid:
| Choice | Current | A Cognitive 25 | B Length only | C Cognitive 15 |
|---|---|---|---|---|
| S2b `noExcessiveCognitiveComplexity` | not enabled | error, maxAllowedComplexity 25 | not enabled | error, maxAllowedComplexity 15 |
| Suppressions added (debt for D1) | 0 | 3 | 0 | 13 |
| Stale suppression fails the lint | n/a | yes, `--error-on-warnings` | n/a | yes, `--error-on-warnings` |
| CLAUDE.md lists the limit (G6) | approved (scope record) | adds web complexity 25 | nothing added | adds web complexity 15 |
| S1 Biome + function length 80 | approved (D2) | kept | kept | kept |
| S2a Rust nesting 4 | approved (D3) | kept | kept | kept |
| S3 web formatter | pending | pending | pending | pending |
Question D4:
D4 — Ограничиваем ли сложность функций в вебе?
Project/branch/task: master, план docs/designs/code-quality-gates.md, задача G3 (Biome для web/).
ELI10: Для Rust мы выбрали запрет глубокой вложенности. У Biome такого общего правила нет, зато есть «когнитивная сложность» по методике SonarSource: она считает ветвления и сильнее штрафует вложенные. При пороге 25 она ловит три наши функции: обработчик клавиш в app.ts:1254 (64), inspector.ts:71 (46) и dom.ts:7 (29). Их пришлось бы пометить комментарием-исключением и убрать при распиле App (D1). Biome сам ругается на исключение, которое больше не нужно, так что пометки не переживут долг.
Stakes if we pick wrong: без правила веб-функция на 70 строк с глубоко вложенными условиями пройдёт CI; с порогом 15 сразу 13 исключений, и пометки превращаются в шум.
Recommendation: A) Cognitive 25 because это единственный способ ограничить вложенность в TS, а три исключения точно указывают, что распиливать в D1.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Cognitive 25 (recommended)
  ✅ Ловит то, что длина пропускает: app.ts:1254 занимает 64 строки, но набирает 64 балла
  ✅ Три исключения, и каждое Biome уберёт сам: ненужная пометка роняет lint
  ❌ Другая метрика, чем в Rust (баллы вместо глубины), её надо объяснить в CLAUDE.md
B) Length only
  ✅ Ноль исключений; в вебе остаётся только понятное правило 80 строк
  ✅ Правила для веба короче, нечего объяснять
  ❌ Вложенность в TS ничем не ограничена, в отличие от Rust
C) Cognitive 15
  ✅ Порог по умолчанию у Biome и SonarSource, строже держит новый код
  ✅ Ловит ещё 10 функций средней сложности, например app.ts:905 (25)
  ❌ 13 исключений с первого дня; список долга вырастает вчетверо
Net: держать ли вложенность в TS той же строгостью, что и в Rust, ценой трёх (или тринадцати) пометок долга. Effort: A human ~15 min / CC ~3 min; B 0; C human ~30 min / CC ~5 min.
Header: Web complexity
Options:
A) Cognitive 25
Biome noExcessiveCognitiveComplexity at maxAllowedComplexity 25; `biome-ignore` with reason "debt: split, see code-quality-gates.md" on app.ts:1254, inspector.ts:71, dom.ts:7; lint runs with --error-on-warnings so a stale ignore fails; CLAUDE.md lists it. S3 decided separately. human ~15 min / CC ~3 min.
B) Length only
No complexity rule for the web; TS functions are bounded by noExcessiveLinesPerFunction 80 (approved, D2) and the 500-line file limit. S3 decided separately.
C) Cognitive 15
Biome noExcessiveCognitiveComplexity at 15 (Biome's default); 13 `biome-ignore` with the same reason; --error-on-warnings; CLAUDE.md lists it. S3 decided separately. human ~30 min / CC ~5 min.

State: approved
Actual answer: A) Cognitive 25 (D4, answered 2026-10-23: "Cognitive 25")
Accepted scope: Biome noExcessiveCognitiveComplexity at maxAllowedComplexity 25; app.ts:1254, inspector.ts:71 and dom.ts:7 are split in debt PRs before the gates PR (D14); lint runs with --error-on-warnings so any later suppression that goes stale fails; CLAUDE.md lists it.
History: D4 accepted scope as answered: "... `biome-ignore` with reason "debt: split, see code-quality-gates.md" on app.ts:1254, inspector.ts:71, dom.ts:7; ...". The three biome-ignore were superseded by D14 (Debt first).

### S3: formatter for the web
Finding: S3, P2, confidence 8/10, `~/.claude/CLAUDE.md` rule 1 ("linters and formatters") vs `web/package.json` scripts (no format script) and `.github/workflows/ci.yml:44` (tsc only); the plan adds no web formatter. Reviewer: Claude.
Plan baseline: no web formatter in the plan. Fixed in every column: Biome 2.6.2 lint with length 80 (D2) and cognitive complexity 25 (D4), Rust nesting 4 (D3), allowlist ratchet G1 (scope record).
Runtime evidence: `biome format --write` probe on copies of `web/src` and `web/tests` (indent 2 spaces, the code's current style: double quotes, semicolons). Width 140: 8 of 21 src files and 4 test files change, 226 + 127 changed diff lines, `src/app.ts` 1555 -> 1657 lines; lint findings after it: `App.notices` 88 (was 82), `tests/e2e/mock.ts:216` 126 lines. Width 120: 7 src + 5 test files, 354 + 182 diff lines, `src/app.ts` 1555 -> 1753; new length findings `src/app.ts:1378` 94 and `src/inspector.ts:75` 99 on top of the two above. Today 51 lines in `web/src/*.ts` exceed 120 chars (max 289). Rust uses rustfmt defaults (no rustfmt.toml, max_width 100).
Comparison grid:
| Choice | Current | A Format, width 140 | B Format, width 120 | C Format after D1 | D No web formatter |
|---|---|---|---|---|---|
| S3 web formatter | none | Biome format in PR 1, lineWidth 140, indent 2 spaces | Biome format in PR 1, lineWidth 120, indent 2 spaces | none in PR 1; TODO: enable Biome format with D1 | none |
| One-time reformat | n/a | 12 files, 353 diff lines | 12 files, 536 diff lines | at D1 | none |
| `web/src/app.ts` allowlist entry | 1555 (G1) | 1657 | 1753 | 1555 | 1555 |
| Length suppressions in src | 1 (D2, `App.notices`) | 1 (`App.notices`, 88) | 3 (`App.notices`, `app.ts:1378`, `inspector`) | 1 | 1 |
| Check in CI `quality` job and pre-push | lint only (D2) | `biome ci --error-on-warnings` (format + lint) | same as A | lint only | lint only |
| Lint scope for `web/tests` (mock.ts:216) | pending (S5) | pending | pending | pending | pending |
Question D5:
D5 — Форматируем ли веб-код автоматически?
Project/branch/task: master, план docs/designs/code-quality-gates.md, глобальное правило 1 («линтеры и форматтеры»).
ELI10: Rust-код у нас форматирует rustfmt, и CI это проверяет. У веба форматтера нет, хотя глобальное правило его требует. Biome, который мы уже ставим, умеет форматировать. Первый прогон перепишет часть файлов: при ширине строки 140 изменится 12 файлов (353 строки диффа), а app.ts вырастет с 1555 до 1657 строк, потому что длинные однострочники разворачиваются. При ширине 120 изменений больше, и появятся две новые слишком длинные функции.
Stakes if we pick wrong: без форматтера стиль веба держится только на аккуратности агента; с узкой шириной долг в allowlist сразу растёт на сотни строк.
Recommendation: A) Format, width 140 because это ближе всего к нынешнему стилю: меньше всего правок и ни одной новой длинной функции в src.
Completeness: A=10/10, B=10/10, C=7/10, D=4/10
Pros / cons:
A) Format, width 140 (recommended)
  ✅ Правило 1 выполнено целиком: CI и pre-push проверяют формат веба, как rustfmt
  ✅ Меньше всего правок: 12 файлов, и в src нет новых длинных функций
  ❌ app.ts в allowlist записывается как 1657 строк, на 102 больше сегодняшнего
B) Format, width 120
  ✅ Более привычная ширина, строки короче и удобнее в diff на узком экране
  ✅ Тоже выполняет правило 1 целиком
  ❌ app.ts вырастает до 1753 строк, плюс две новые функции длиннее 80 (app.ts:1378, inspector)
C) Format after D1
  ✅ Распил App (D1) не смешивается с переформатированием, дифф проще ревьюить
  ✅ Allowlist стартует с нынешних 1555 строк
  ❌ До D1 правило 1 не выполнено, и стиль может разъехаться
D) No web formatter
  ✅ Ноль правок и ни одной новой проверки
  ✅ app.ts и allowlist не меняются
  ❌ Нарушает глобальное правило 1; стиль веба ничем не проверяется
Net: выполнить правило про форматтер сейчас ценой разового диффа и +102 строк долга, или отложить до распила App. Effort: A human ~30 min / CC ~5 min; B same; C 0 now, ~5 min CC at D1; D 0.
Header: Web formatter
Options:
A) Format, width 140
Biome formatter in PR 1: lineWidth 140, indent 2 spaces, over web/src, web/tests and the playwright configs; one-time reformat (12 files); CI quality job and pre-push run `biome ci --error-on-warnings`; allowlist records app.ts after formatting (1657). Lint scope for tests (S5) decided separately. human ~30 min / CC ~5 min.
B) Format, width 120
Same as A with lineWidth 120: one-time reformat (12 files, 536 diff lines), app.ts recorded at 1753, two more length suppressions (app.ts:1378, inspector). S5 decided separately. human ~30 min / CC ~5 min.
C) Format after D1
No formatter in PR 1; add a TODO to enable Biome format (width decided then) in the D1 PR, after App is split. S5 decided separately.
D) No web formatter
No web formatter; the plan records that global rule 1 is not applied to web formatting. S5 decided separately.

State: approved
Actual answer: A) Format, width 140 (D5, answered 2026-10-23: "Format, width 140")
Accepted scope: Biome formatter, lineWidth 140, indent 2 spaces, over web/src, web/tests and the playwright configs; the one-time reformat (12 files) is the first PR of the wave (D14); from the gates PR, the CI quality job and pre-push run `biome ci --error-on-warnings`.
History: D5 accepted scope as answered: "Biome formatter in PR 1: ... allowlist records app.ts after formatting (1657). ...". "In PR 1" became "the first PR of the wave" and the allowlist entry was dropped by D14 (Debt first).

### S5: Biome lint scope for web tests
Finding: S5, P2, confidence 9/10, `web/tests/e2e/mock.ts:216` (`async handle(route: Route)`), reviewer Claude.
Plan baseline: G3 does not say whether `web/tests` is linted. Fixed in every column: Biome lint for `web/src` with length 80 (D2) and complexity 25 (D4), `biome ci --error-on-warnings` and the width-140 format over src, tests and the playwright configs (D5).
Runtime evidence: Biome 2.6.2 probe on the width-140-formatted copy of `web/tests`: `tests/e2e/mock.ts:216` is 126 lines and cognitive complexity 34; nothing else in tests is over 80 lines or 25. Rust: CI runs `cargo clippy --workspace --all-targets` (`.github/workflows/ci.yml:25`), so Rust tests are under the same function limit, and none of the 13 long functions is a test.
Comparison grid:
| Choice | Current | A Same rules | B Looser for tests | C src only |
|---|---|---|---|---|
| S5 lint scope | n/a | `web/src`, `web/tests`, playwright configs | same paths | `web/src` only |
| Test function length | n/a | 80 | 160 (Biome `overrides` for `tests/**`) | not checked |
| Test complexity | n/a | 25 | off for `tests/**` | not checked |
| Suppressions in tests | 0 | 2 on `mock.ts:216` (length, complexity); split joins web debt D1 | 0 | 0 |
| CLAUDE.md lists the limits (G6) | approved | one rule for src and tests | adds the test override | says tests are not linted |
| D2, D4, D5 values | approved | kept | kept | kept |
Question D6:
D6 — Проверяет ли Biome тесты веба?
Project/branch/task: master, план docs/designs/code-quality-gates.md, задача G3 (Biome).
ELI10: В Rust clippy проверяет и тесты (CI запускает его с --all-targets), и все Rust-тесты укладываются в 80 строк. В вебе план не говорит, гоняем ли Biome по tests/. Если гоняем с теми же правилами, нарушитель один: обработчик запросов фейкового API в tests/e2e/mock.ts:216, 126 строк и сложность 34. Его надо пометить исключением и потом распилить.
Stakes if we pick wrong: без проверки тестов агент может вырастить в них функцию на 500 строк; со слишком строгими правилами тесты обрастают пометками.
Recommendation: A) Same rules because так же устроено в Rust, а нарушитель всего один и его легко распилить по маршрутам API.
Completeness: A=10/10, B=8/10, C=5/10
Pros / cons:
A) Same rules (recommended)
  ✅ Одно правило для всего кода, как у clippy с --all-targets в Rust
  ✅ Нарушитель один (mock.ts:216): две пометки, распил по маршрутам API
  ❌ Тесты иногда честно длинные; каждый такой случай будет спором про исключение
B) Looser for tests
  ✅ Ноль пометок: 160 строк вмещают mock.ts:216 (126), сложность в тестах не считается
  ✅ Перекликается с лимитами файлов: тест-файлу 1000 строк, исходнику 500
  ❌ Правила в Rust и вебе расходятся: в Rust тесты держат 80
C) src only
  ✅ Самая простая настройка, тесты только форматируются
  ✅ Ноль пометок и ноль долга в tests/
  ❌ Функции в тестах веба ничем не ограничены, кроме размера файла 1000
Net: одинаковая строгость для тестов и кода ценой двух пометок, или послабление для тестов без пометок. Effort: A human ~10 min / CC ~2 min; B human ~15 min / CC ~3 min; C 0.
Header: Test lint scope
Options:
A) Same rules
Biome lint covers web/src, web/tests and the playwright configs with length 80 and complexity 25; two `biome-ignore` with the debt reason on tests/e2e/mock.ts:216, and its split joins web debt D1. human ~10 min / CC ~2 min.
B) Looser for tests
Biome lint covers the same paths; an `overrides` entry for tests/** sets function length 160 and turns the complexity rule off; no suppressions; CLAUDE.md states the test override. human ~15 min / CC ~3 min.
C) src only
Biome lint covers web/src only; web/tests are formatted (D5) but not linted; CLAUDE.md says so.

State: approved
Actual answer: A) Same rules (D6, answered 2026-10-23: "Same rules")
Accepted scope: Biome lint covers web/src, web/tests and the playwright configs with length 80 and complexity 25; tests/e2e/mock.ts:216 is split in a debt PR before the gates PR (D14).
History: D6 accepted scope as answered: "... two `biome-ignore` with the debt reason on tests/e2e/mock.ts:216, and its split joins web debt D1. ...". The two biome-ignore were superseded by D14 (Debt first).

### S4: the global rule's clippy example
Finding: S4, P3, confidence 9/10, `~/.claude/CLAUDE.md:7` "Rust: rustfmt, clippy with `-D warnings` plus `clippy::too_many_lines` and `clippy::cognitive_complexity`", reviewer Claude.
Plan baseline: the global rule as written this session (outside the repo, user-owned). No approval to change it.
Runtime evidence: same as S2a: clippy's own docs for `cognitive_complexity` ("We used to think it measured how hard a method is to understand", kept in `restriction` "so as to not mislead users") and its suggested replacements `excessive_nesting`, `too_many_lines`; `excessive_nesting` only fires with `excessive-nesting-threshold` set in clippy.toml.
Comparison grid:
| Choice | Current | A Nesting instead | B Drop it | C Leave as is |
|---|---|---|---|---|
| S4 Rust example in `~/.claude/CLAUDE.md:7` | `too_many_lines` and `cognitive_complexity` | `too_many_lines` and `excessive_nesting` (set `excessive-nesting-threshold`; not `cognitive_complexity`, its own docs call it misleading) | `too_many_lines` only | unchanged |
| Rest of the global rule | as written | unchanged | unchanged | unchanged |
| This repo's lints (D3) | approved | unchanged | unchanged | unchanged |
Question D7:
D7 — Поправить пример clippy в глобальном CLAUDE.md?
Project/branch/task: master, план docs/designs/code-quality-gates.md; файл ~/.claude/CLAUDE.md (вне репо, общий для всех проектов).
ELI10: Глобальное правило, которое мы сегодня написали, в примере для Rust советует clippy::cognitive_complexity. Ревью показало, что сами авторы clippy считают эту метрику вводящей в заблуждение; для freespace мы её уже заменили на excessive_nesting. Если пример не поправить, каждый новый проект будет заводить ту же спорную проверку.
Stakes if we pick wrong: ИИ в следующих проектах включит cognitive_complexity по примеру и будет распиливать функции ради балла, которому не верят его же авторы.
Recommendation: A) Nesting instead because это то же решение, что вы приняли для freespace (D3), и оно не даст правилу разойтись с практикой.
Completeness: A=10/10, B=8/10, C=5/10
Pros / cons:
A) Nesting instead (recommended)
  ✅ Глобальное правило совпадает с тем, что мы сделали здесь (D3)
  ✅ В примере сказано, почему не cognitive_complexity: агент не вернёт её обратно
  ❌ Строка становится длиннее и упоминает порог, который надо задать в clippy.toml
B) Drop it
  ✅ Самая короткая строка: остаётся только too_many_lines
  ✅ Не навязывает проектам ещё одну настройку
  ❌ Без подсказки агент может снова выбрать cognitive_complexity как «очевидное» правило
C) Leave as is
  ✅ Ноль правок в глобальном файле
  ✅ Каждый проект сам решит на своём ревью, как здесь
  ❌ Пример продолжит советовать метрику, от которой отказались авторы clippy
Net: синхронизировать глобальный совет с решением D3 сейчас, или оставить разбор каждому проекту. Effort: A/B human ~2 min / CC ~1 min; C 0.
Header: Global rule
Options:
A) Nesting instead
Edit ~/.claude/CLAUDE.md:7 so the Rust example reads: clippy with `-D warnings` plus `clippy::too_many_lines` and `clippy::excessive_nesting` (set `excessive-nesting-threshold`; not `cognitive_complexity`, its own docs call it misleading). Nothing else in the file changes.
B) Drop it
Edit ~/.claude/CLAUDE.md:7 to remove `clippy::cognitive_complexity`, leaving `clippy::too_many_lines`. Nothing else changes.
C) Leave as is
No change to ~/.claude/CLAUDE.md.

State: approved
Actual answer: A) Nesting instead (D7, answered 2026-10-23: "Nesting instead")
Accepted scope: Edit ~/.claude/CLAUDE.md:7 so the Rust example reads: clippy with `-D warnings` plus `clippy::too_many_lines` and `clippy::excessive_nesting` (set `excessive-nesting-threshold`; not `cognitive_complexity`, its own docs call it misleading). Nothing else in the file changes. Applied 2026-10-23.
History: none

### Scope Challenge result

scope accepted as-is (no cuts; Original arrangement, D1). Dispositions: S1 accepted (Biome, D2); S2 accepted as S2a nesting 4 (D3) and S2b web cognitive 25 (D4); S3 accepted (Biome format width 140, D5); S5 accepted (same rules for tests, D6); S4 accepted and applied to `~/.claude/CLAUDE.md` (D7). Pending: none. Later change: D14 put the debt PRs before the gates PR and removed the exception machinery; no feature was cut, so the result stays scope accepted as-is.


### Section 1: Architecture

Check map after the approved answers (D1-D7), for orientation:

```
                 git push
                    |
          .githooks/pre-push  (opt-in: git config core.hooksPath .githooks)
                    |
   fmt --check, clippy -D warnings (too_many_lines 80, excessive_nesting 4),
   check-size.sh (quality/size-allowlist.txt), shellcheck,
   tsc --noEmit, biome ci --error-on-warnings (format 140, length 80, complexity 25)
                    |
                 GitHub CI
   rust job: fmt, clippy, tests        quality job: check-size.sh, biome ci
   web job: tsc, unit, e2e             shellcheck job: install.sh scripts/*.sh
```

Findings:

1. A1 [P2] (confidence: 8/10) `docs/designs/code-quality-gates.md:64-65` (G4 lists the hook's commands, G5 lists CI's) and `.github/workflows/ci.yml:24-25,44,109` (CI spells each command inline). Two hand-written lists of the same checks with flags (`-D warnings`, `--error-on-warnings`, `--all-targets`) drift: a flag added in CI and not in the hook makes the hook pass and CI fail, or the reverse. Reviewer: Claude.
2. A2 [P2] (confidence: 7/10) `docs/designs/code-quality-gates.md:64` "`doctor`-style message if a check's tool is missing": does the hook fail or skip? A hook that fails on a missing tool gets bypassed with `--no-verify`; one that skips passes pushes it did not check. Local tools today: shellcheck 0.8.0, bun, cargo, jq all present. Reviewer: Claude.
3. A3 [P3] (confidence: 9/10) `.github/workflows/ci.yml:109` `shellcheck install.sh scripts/*.sh` misses `.githooks/pre-push` (no `.sh` extension). Global rule 1 (shellcheck for shell) already requires it, so the hook joins the shellcheck list in PR 1 as part of the approved G4. No question.
4. A4 [P3] (confidence: 8/10) G4/G6: the hook is opt-in per clone. An agent in a fresh clone pushes without it; CI still gates. Global rule 3 says "installed by one documented command", so `CLAUDE.md` names `git config core.hooksPath .githooks` next to the check commands, as part of the approved G6. No question.
5. Security: no issues found. Biome is pinned exactly (D2) and locked in `web/bun.lock`; the hook runs repo scripts only on the owner's machine.
6. Distribution: no new user-facing artifact; the quality job installs bun and the locked Biome like the web job (`.github/workflows/ci.yml:40,43`).

### A1: one entrypoint for hook and CI checks
Finding: A1, P2, confidence 8/10, `docs/designs/code-quality-gates.md:64-65`, `.github/workflows/ci.yml:24-25,44,109`, reviewer Claude.
Plan baseline: G4 hook runs its own list (fmt, clippy, check-size, shellcheck, tsc, lint); G5 quality job runs check-size and the web lint; the rust job keeps its steps. Approved structure (D1): separate `quality` job, `quality/size-allowlist.txt`; unchanged in every column.
Runtime evidence: `.github/workflows/ci.yml:24` `cargo fmt --all --check`, `:25` `cargo clippy --workspace --all-targets -- -D warnings`, `:44` `bunx tsc --noEmit`, `:109` `shellcheck install.sh scripts/*.sh`; no shared script exists today (`ls scripts/`: bench and fault-injection scripts only).
Comparison grid:
| Choice | Current | A One entrypoint | B Two lists |
|---|---|---|---|
| A1 where the check commands live | ci.yml inline | `scripts/check.sh [rust\|size\|shell\|tsc\|biome]...`, no argument = all; rust job `rust`, quality job `size biome`, shellcheck job `shell`, web job `tsc`; the hook is `exec scripts/check.sh` | the hook spells its commands, ci.yml spells its own; CONTRIBUTING.md says to keep both in step |
| New files in PR 1 | 5 (D1) | 6 (+ `scripts/check.sh`) | 5 |
| CI jobs | approved (D1) | unchanged: each calls its part, tests stay inline | unchanged |
| Flags (`-D warnings`, `--error-on-warnings`, `--all-targets`) | approved (D2-D6) | written once | written twice |
| A2 missing-tool behavior | pending | pending | pending |
Question D8:
D8 — Один скрипт проверок для hook и CI?
Project/branch/task: master, план docs/designs/code-quality-gates.md, задачи G4 (pre-push) и G5 (CI).
ELI10: По плану pre-push hook и CI перечисляют одни и те же проверки дважды, каждый у себя и со всеми флагами. Стоит кому-то добавить флаг в одном месте и забыть в другом, и локально всё зелёное, а CI красный (или наоборот). Вместо этого можно завести scripts/check.sh: hook вызывает его целиком, а каждая CI job свою часть (rust, size, shell, tsc, biome). Тогда команды и флаги записаны один раз.
Stakes if we pick wrong: hook перестаёт предсказывать CI, и агент узнаёт о падении только после push.
Recommendation: A) One entrypoint because команды записаны один раз, а jobs из D1 остаются как были, просто вызывают свою часть.
Completeness: A=10/10, B=7/10
Pros / cons:
A) One entrypoint (recommended)
  ✅ Флаги вроде --error-on-warnings записаны один раз: hook и CI не могут разойтись
  ✅ Агент запускает ровно то же, что CI: scripts/check.sh или scripts/check.sh biome
  ❌ Ещё один файл в PR 1, и шаги CI читаются через скрипт, а не напрямую в YAML
B) Two lists
  ✅ Каждая команда видна прямо в ci.yml, без захода в скрипт
  ✅ На один файл меньше
  ❌ Синхронность держится только на памяти; расхождение всплывает после push
Net: один источник команд ценой одного скрипта, или наглядный YAML ценой ручной синхронизации. Effort: A human ~1 h / CC ~10 min; B human ~30 min / CC ~5 min.
Header: Check entrypoint
Options:
A) One entrypoint
Add scripts/check.sh with parts rust (fmt, clippy), size (check-size.sh), shell (shellcheck over install.sh, scripts/*.sh, .githooks/pre-push), tsc (tsc --noEmit), biome (biome ci --error-on-warnings); no argument runs all. .githooks/pre-push execs it; CI: rust job `rust`, quality job `size biome`, shellcheck job `shell`, web job `tsc`; tests stay inline. Jobs and allowlist location stay as D1. A2 decided separately. human ~1 h / CC ~10 min.
B) Two lists
The hook lists its commands and ci.yml lists its own, as G4/G5 are written; CONTRIBUTING.md asks to change both together. A2 decided separately. human ~30 min / CC ~5 min.

State: approved
Actual answer: A) One entrypoint (D8, answered 2026-10-23: "One entrypoint")
Accepted scope: Add scripts/check.sh with parts rust (fmt, clippy), size (check-size.sh), shell (shellcheck over install.sh, scripts/*.sh, .githooks/pre-push), tsc (tsc --noEmit), biome (biome ci --error-on-warnings); no argument runs all. .githooks/pre-push execs it; CI: rust job `rust`, quality job `size biome`, shellcheck job `shell`, web job `tsc`; tests stay inline. Jobs and allowlist location stay as D1. A2 decided separately. human ~1 h / CC ~10 min.
History: none

### A2: missing tool in scripts/check.sh
Finding: A2, P2, confidence 7/10, `docs/designs/code-quality-gates.md:64` "`doctor`-style message if a check's tool is missing", reviewer Claude.
Plan baseline: a message when a tool is missing; fail or skip unspecified. Fixed in every column: scripts/check.sh is the one entrypoint for hook and CI (D8).
Runtime evidence: on this machine `command -v` finds shellcheck (0.8.0), bun, cargo, jq; CI runners have all of them (`.github/workflows/ci.yml:16,21,40,109`). `git push --no-verify` bypasses any pre-push hook.
Comparison grid:
| Choice | Current | A Fail with hint | B Skip locally |
|---|---|---|---|
| A2 tool missing (cargo, bun, shellcheck) | unspecified | exit 1: "shellcheck not found; install: sudo apt-get install shellcheck" (one hint per tool) | locally: print "skipped shell: shellcheck not found" and go on; with `CI` set: exit 1 like A |
| What the push proves | n/a | every check ran | only the checks whose tools exist |
| CI behavior | approved (D8) | fails on a missing tool | fails on a missing tool |
Question D9:
D9 — Что делать hook, если инструмента нет?
Project/branch/task: master, план docs/designs/code-quality-gates.md, задача G4 (scripts/check.sh из D8).
ELI10: scripts/check.sh вызывает cargo, bun и shellcheck. Если какого-то нет, можно либо упасть с подсказкой, как поставить, либо пропустить эту проверку и продолжить. Падение честнее: push не уходит непроверенным. Пропуск удобнее на голой машине, но тогда зелёный hook не значит, что всё проверено. В CI все инструменты есть в обоих вариантах.
Stakes if we pick wrong: с пропуском агент на новой машине пушит код, который проверка не видела, и узнаёт об ошибке из CI; с падением без подсказки hook начнут обходить через --no-verify.
Recommendation: A) Fail with hint because зелёный hook тогда всегда значит «все проверки прошли», а подсказка снимает повод обходить его.
Completeness: A=10/10, B=7/10
Pros / cons:
A) Fail with hint (recommended)
  ✅ Зелёный hook значит, что прошли все проверки, без исключений
  ✅ Подсказка с командой установки: исправить можно сразу, не гугля
  ❌ На машине без shellcheck push не пройдёт, пока его не поставят или не обойдут hook
B) Skip locally
  ✅ Push с голой машины проходит, а CI всё равно поймает ошибку
  ✅ Не толкает к --no-verify, который выключает вообще все проверки
  ❌ Зелёный hook не гарантирует, что shellcheck или Biome вообще запускались
Net: строгий hook, который иногда мешает, или удобный, который иногда молча не проверяет. Effort: A human ~20 min / CC ~3 min; B human ~30 min / CC ~5 min.
Header: Missing tool
Options:
A) Fail with hint
scripts/check.sh checks `command -v` for each tool a selected part needs and exits 1 with the tool's name and its install command; same in CI. CONTRIBUTING.md lists the tools. human ~20 min / CC ~3 min.
B) Skip locally
Without `CI` set, a part whose tool is missing prints "skipped <part>: <tool> not found" and the script goes on; with `CI` set it exits 1 like A. CONTRIBUTING.md lists the tools. human ~30 min / CC ~5 min.

State: approved
Actual answer: A) Fail with hint (D9, answered 2026-10-23: "Fail with hint")
Accepted scope: scripts/check.sh checks `command -v` for each tool a selected part needs and exits 1 with the tool's name and its install command; same in CI. CONTRIBUTING.md lists the tools. human ~20 min / CC ~3 min.
History: none

Section 1 dispositions: A1 accepted (D8, one entrypoint `scripts/check.sh`); A2 accepted (D9, fail with hint); A3 accepted as required by global rule 1 (hook in the shellcheck list); A4 accepted as required by global rule 3 (install command in CLAUDE.md); security and distribution: no issues.

### Section 2: Code quality

Findings:

1. C1 [P2] (confidence: 9/10) `docs/designs/code-quality-gates.md:61` (G1 "every tracked file in scope") and `:39` ("Not counted: generated or vendored files ..., docs, fonts"). Read as an exclude list, it must also exclude binaries and text that is not code: `git ls-files` today has 4 `.png` (`docs/img/gui.png` counts as 653 "lines"), 9 `.woff2`, 1 `.svg`, `LICENSE`, `web/fonts/OFL.txt`, `skill/schema.json` (1588), `Cargo.lock` (1571), `docs/designs/free-space-disk-analyzer.md` (3387). Any new data file over 500 fails CI. Reviewer: Claude.
2. C2 [P2] (confidence: 9/10) Q1 `docs/designs/code-quality-gates.md:77`: inline `#[cfg(test)]` counting is undecided. `core/src/refresh.rs` 572 (13 test lines), `core/src/scan/walk.rs` 563 (89), `core/src/scan/tree.rs` 559 (134); `cli/src/serve/tests.rs` (433) already keeps tests in their own file. Reviewer: Claude.
3. C3 [P3] (confidence: 8/10) Q2 `docs/designs/code-quality-gates.md:78`: CSS limit undecided. `web/src/app.css` 1144 splits at its own section comments into parts of 78-410 lines (`app.css:67` actions, `:145` shell 410, `:555` work area 246, `:801` inspector 166, `:967` footer to states 177). Reviewer: Claude.
4. C4 [P2] (confidence: 8/10) Q3 `docs/designs/code-quality-gates.md:79`: `#[expect(clippy::too_many_lines)]` and `biome-ignore` stop the lint for a function at any length. `core/src/snapshot/writer.rs` is 335 lines, under 500, so it is not in the file allowlist either: `write_inner` (`writer.rs:119`, 203 lines) can grow to 350 and nothing fails. This breaks the global rule "an exception may shrink, never grow". Reviewer: Claude.
5. C5 [P3] (confidence: 9/10) G1 edge cases, all inside the approved ratchet contract, no question: a listed path that no longer exists counts as under the limit (fail: remove the entry), so a deleted or renamed file cannot leave a dead entry; files come from `git ls-files -z`; outside a git work tree, or with zero files matched, the script exits 1 instead of passing.

### C1: which files check-size.sh counts
Finding: C1, P2, confidence 9/10, `docs/designs/code-quality-gates.md:39,61`, reviewer Claude.
Plan baseline: G1 "every tracked file in scope"; not counted: generated or vendored files (`skill/schema.json`, `.github/workflows/release.yml`, lock files), docs, fonts. Fixed in every column: limits 500 / 1000, test paths `*/tests/**`, `*/tests.rs`, `web/tests/**`, ratchet rules (G1, scope record), entrypoint `scripts/check.sh size` (D8).
Runtime evidence: `git ls-files` by extension: 58 rs, 35 ts, 11 md, 9 woff2, 5 css, 4 toml, 4 sh, 4 png, 3 yml, 3 json, 2 lock, 2 html, 1 txt, 1 svg, 1 js (`docs/designs/mockups/browse-b/data.js`), `LICENSE`, `.gitignore`. Over 500 lines outside code: `docs/designs/free-space-disk-analyzer.md` 3387, `skill/schema.json` 1588, `Cargo.lock` 1571, `docs/img/gui.png` 653, `docs/img/gui-changes.png` 512.
Comparison grid:
| Choice | Current | A Code only | B All but excluded |
|---|---|---|---|
| C1 files counted | none | tracked `*.rs *.ts *.js *.css *.html *.sh` and `.githooks/*`, outside `docs/**` | every tracked file except an exclude list: `*.md`, `*.lock`, `*.png *.svg *.woff2`, `skill/schema.json`, `.github/workflows/release.yml`, `LICENSE`, `web/fonts/**`, `docs/**` |
| Code files over 500 today | 7 (plan table) | same 7 | same 7 |
| A new 600-line JSON fixture | n/a | not counted | fails until excluded |
| A new language (say `*.py`) | n/a | not counted until added to the list | counted at once |
| Limits, test paths, ratchet | approved | unchanged | unchanged |
| C2 inline tests, C3 CSS limit, C4 function ratchet | pending | pending | pending |
Question D10:
D10 — Какие файлы считает проверка размера?
Project/branch/task: master, план docs/designs/code-quality-gates.md, задача G1 (scripts/check-size.sh).
ELI10: План говорит «все файлы в git, кроме сгенерированных, документации и шрифтов». На деле в git ещё лежат картинки (gui.png git считает как 653 «строки»), LICENSE, JSON-схема на 1588 строк, Cargo.lock на 1571. Их всех пришлось бы перечислять в исключениях, а каждый новый большой файл данных ронял бы CI. Проще считать только код: файлы на наших языках (.rs .ts .js .css .html .sh и hook), кроме docs/.
Stakes if we pick wrong: со списком исключений CI падает на первом большом JSON-фикстуре; со списком языков новый язык не проверяется, пока его не добавят.
Recommendation: A) Code only because лимит про длину кода, а список языков короткий и меняется только когда в репо приходит новый язык, что по глобальному правилу и так требует новых линтеров.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Code only (recommended)
  ✅ Короткий список из шести расширений и .githooks, без списка исключений
  ✅ Картинки, шрифты, схемы и lock-файлы не мешают и не требуют записей
  ❌ Новый язык (например .py) не проверяется, пока его не добавят в список
B) All but excluded
  ✅ Новый тип файла проверяется сразу, ничего не забыть
  ✅ Ближе к тексту плана: «every tracked file»
  ❌ Список исключений уже из десяти пунктов и растёт с каждым новым видом данных
Net: явный список того, что считаем кодом, или длинный список того, что кодом не считаем. Effort: A human ~20 min / CC ~3 min; B human ~30 min / CC ~5 min.
Header: Size scope
Options:
A) Code only
check-size.sh counts tracked *.rs *.ts *.js *.css *.html *.sh and .githooks/*, outside docs/**; everything else is not counted. CLAUDE.md names the extensions and says a new language joins the list with its linter. human ~20 min / CC ~3 min.
B) All but excluded
check-size.sh counts every tracked file except *.md, *.lock, *.png, *.svg, *.woff2, skill/schema.json, .github/workflows/release.yml, LICENSE, web/fonts/**, docs/**; a new large non-code file needs an exclusion. human ~30 min / CC ~5 min.

State: approved
Actual answer: A) Code only (D10, answered 2026-10-23: "Code only")
Accepted scope: check-size.sh counts tracked *.rs *.ts *.js *.css *.html *.sh and .githooks/*, outside docs/**; everything else is not counted. CLAUDE.md names the extensions and says a new language joins the list with its linter. human ~20 min / CC ~3 min.
History: none

### C2: inline Rust tests and the file limit (Q1)
Finding: C2, P2, confidence 9/10, `docs/designs/code-quality-gates.md:77` (Q1), reviewer Claude.
Plan baseline: Q1 open; the plan leans to counting the whole file ("simpler and pushes tests into `tests.rs` files"). D4 already plans moving walk/tree tests out. Fixed in every column: code-only scope (D10), limits 500 / 1000, ratchet (G1).
Runtime evidence: files with an inline `#[cfg(test)]` block (total / before the block / block): `core/src/refresh.rs` 572 / 559 / 13, `core/src/scan/walk.rs` 563 / 474 / 89, `core/src/scan/tree.rs` 559 / 425 / 134, `core/src/platform.rs` 386 / 233 / 153, `core/src/queries/common.rs` 438 / 417 / 21; 20 such files in all. `cli/src/serve/tests.rs` is a separate test file.
Comparison grid:
| Choice | Current | A Whole file | B Code before tests |
|---|---|---|---|
| C2 what counts toward 500 | undecided | every line of the file | lines before the first `#[cfg(test)]`; the block itself is not counted |
| Allowlist entries for Rust today | n/a | refresh.rs 572, walk.rs 563, tree.rs 559 (+ install.rs, query.rs) | refresh.rs 559 (+ install.rs, query.rs); walk.rs, tree.rs pass |
| D4 work for walk.rs, tree.rs | approved | move tests to `tests.rs` submodules | none needed |
| A file with 450 lines of code and 900 lines of inline tests | n/a | fails (1350 > 500) | passes |
| check-size.sh parsing | approved (D10) | line count only | finds the first `#[cfg(test)]` line in `.rs` files |
| C3 CSS limit, C4 function ratchet | pending | pending | pending |
Question D11:
D11 — Считать ли inline-тесты в лимит 500 строк?
Project/branch/task: master, план docs/designs/code-quality-gates.md, открытый вопрос Q1.
ELI10: В Rust тесты часто живут прямо в файле, в блоке #[cfg(test)] в конце. Можно считать весь файл (просто, и тесты переезжают в отдельный tests.rs, как уже сделано в serve/), а можно считать только код до этого блока. Во втором случае walk.rs (474 строки кода) и tree.rs (425) сразу проходят без переноса тестов, но файл может расти тестами без предела.
Stakes if we pick wrong: без учёта тестов файл на 450 строк кода и 900 строк тестов проходит проверку; с учётом приходится переносить тесты в отдельные файлы.
Recommendation: A) Whole file because правило «файл не длиннее 500» тогда буквальное, скрипт просто считает строки, а перенос тестов в tests.rs уже запланирован в D4.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Whole file (recommended)
  ✅ Правило буквальное: 500 строк файла, как видит человек в редакторе
  ✅ Скрипт считает строки и не разбирает Rust; тесты уходят в tests.rs, как в serve/
  ❌ walk.rs и tree.rs попадают в allowlist, пока D4 не перенесёт их тесты
B) Code before tests
  ✅ walk.rs и tree.rs проходят сразу, allowlist короче на две записи
  ✅ Тесты рядом с кодом не наказываются, их не надо переносить
  ❌ Файл может расти тестами без предела; скрипт должен искать #[cfg(test)]
Net: простое буквальное правило ценой переноса тестов, или мягкое правило с дырой для тестов. Effort: A 0 now (D4 moves tests); B human ~20 min / CC ~3 min.
Header: Inline tests
Options:
A) Whole file
Every line of a .rs file counts toward 500; walk.rs and tree.rs enter the allowlist until D4 moves their tests to tests.rs submodules. CLAUDE.md says long tests go to a tests.rs file.
B) Code before tests
In .rs files, only lines before the first #[cfg(test)] count toward 500; walk.rs and tree.rs pass now, refresh.rs is recorded at 559; D4 drops the test moves. human ~20 min / CC ~3 min.

State: approved
Actual answer: A) Whole file (D11, answered 2026-10-23: "Whole file")
Accepted scope: Every line of a .rs file counts toward 500; D4 moves the walk.rs and tree.rs tests to tests.rs submodules before the gates PR (D14). CLAUDE.md says long tests go to a tests.rs file.
History: D11 accepted scope as answered: "... walk.rs and tree.rs enter the allowlist until D4 moves their tests ...". The allowlist entries were superseded by D14 (Debt first).

### C3: limit for CSS (Q2)
Finding: C3, P3, confidence 8/10, `docs/designs/code-quality-gates.md:78` (Q2), reviewer Claude.
Plan baseline: Q2 open; G1 applies 500 to every source file. D2 plans splitting `web/src/app.css` by component. Fixed in every column: code-only scope including `*.css` (D10), whole-file counting (D11), Biome format over CSS (D5).
Runtime evidence: `web/src/app.css` 1144 lines; its section comments: `:1` header, `:67` action buttons, `:145` shell (to `:555`, 410 lines), `:555` work area (246), `:801` inspector (166), `:967` footer keys, toasts, help, full-page states (177). `web/src/main.ts:1-3` imports the CSS files one by one, so more files cost nothing at runtime. Other counted CSS: `web/src/responsive.css` 149, `web/src/tokens.css` 43, `web/fonts/fonts.css` 11.
Comparison grid:
| Choice | Current | A 500 like code | B 800 for CSS |
|---|---|---|---|
| C3 CSS file limit | 500 (G1, not yet decided for CSS) | 500 | 800 |
| `app.css` in the allowlist | n/a | 1144 | 1144 |
| D2 split | approved | about 4 files by section (largest 410) | 2 files (e.g. shell + work area, the rest) |
| check-size.sh rules | approved (D10, D11) | one source limit | an extra per-extension limit for `*.css` |
| C4 function ratchet | pending | pending | pending |
Question D12:
D12 — Какой лимит у CSS-файлов?
Project/branch/task: master, план docs/designs/code-quality-gates.md, открытый вопрос Q2.
ELI10: app.css сейчас 1144 строки. Стили обычно длиннее кода на ту же функциональность: много коротких правил. Можно дать CSS тот же лимит 500, можно отдельный 800. В самом app.css уже есть разделы с комментариями (шапка, рабочая область, инспектор, подвал); самый большой из них 410 строк, так что 500 достижимо без натяжек.
Stakes if we pick wrong: при 800 CSS-файл может стать вдвое длиннее кода рядом; при 500 app.css дробится на четыре файла, а не на два.
Recommendation: A) 500 like code because разделы app.css и так укладываются в 500, а одно правило для всех исходников проще объяснять и проверять.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) 500 like code (recommended)
  ✅ Одно правило для всех исходников, скрипт не знает про расширения
  ✅ Разрез по готовым разделам: шапка 410, рабочая область 246, инспектор 166, подвал 177
  ❌ app.css распадается на четыре файла, а не на два
B) 800 for CSS
  ✅ Меньше файлов стилей: app.css делится на две части
  ✅ Учитывает, что CSS многословнее кода
  ❌ Отдельное правило для одного расширения в скрипте и в CLAUDE.md
Net: одно простое правило ценой лишней пары файлов, или особый лимит для стилей. Effort: A and B: D2 human ~1 h / CC ~10 min; B adds ~5 min to check-size.sh.
Header: CSS limit
Options:
A) 500 like code
CSS files share the 500-line source limit; D2 splits app.css by its section comments into about four files. No per-extension rule.
B) 800 for CSS
check-size.sh applies 800 to *.css and 500 to other source; D2 splits app.css into two files; CLAUDE.md states both limits. human ~5 min / CC ~1 min extra.

State: approved
Actual answer: A) 500 like code (D12, answered 2026-10-23: "500 like code")
Accepted scope: CSS files share the 500-line source limit; D2 splits app.css by its section comments into about four files. No per-extension rule.
History: none


### C4: ratchet on excepted functions (Q3)
Finding: C4, P2, confidence 8/10, `docs/designs/code-quality-gates.md:79` (Q3), `core/src/snapshot/writer.rs:119` (`fn write_inner(`, file 335 lines, not in the file allowlist), reviewer Claude.
Plan baseline: Q3 open: accept that `#[expect]` freezes which functions may be long but not how long, or record lengths too. Fixed in every column: `#[expect(clippy::too_many_lines)]` on the 13 Rust functions (G2), `biome-ignore` on `App.notices` (D2), on `app.ts:1254`, `inspector.ts:71`, `dom.ts:7` (D4) and two on `tests/e2e/mock.ts:216` (D6); file allowlist with whole-file counting (D10-D12).
Runtime evidence: probe from each function's first line to the first later line that is only `}` at the same indent (rustfmt and the Biome formatter both close a function there): all 13 Rust functions resolve, e.g. `writer.rs:119` 217 physical lines (clippy 203), `refresh.rs:64` 181, `job.rs:151` 109. 7 of the 13 sit in files over 500 (`refresh.rs` 3, `walk.rs` 2, `tree.rs`, `install.rs`), capped only through their file; the other 6 (`writer.rs`, `diff.rs`, `cli/src/status.rs`, `queries/status.rs`, `doctor.rs`, `job.rs`) have no cap at all today.
Comparison grid:
| Choice | Current | A Function entries | B Freeze host files | C Rule in CLAUDE.md |
|---|---|---|---|---|
| C4 cap on an excepted function | none | its own entry `fn <path> <name> <lines>` in `quality/size-allowlist.txt`; may shrink, never grow (same rules as files) | each file holding an excepted function gets a file entry at its current size, even under 500 | none in tooling; CLAUDE.md: never add lines to a function with an `#[expect]` or `biome-ignore` |
| Entries added | 0 | 18 (13 Rust, 5 TS) | 9 file entries (the 6 Rust files above, `inspector.ts`, `dom.ts`, `tests/e2e/mock.ts`) | 0 |
| A function grows while its file shrinks elsewhere | n/a | fails | passes | passes unless a reviewer notices |
| How length is measured | n/a | name lookup (`fn <name>` or method `<name>(`), then to the closing brace at the same indent; 0 or 2+ matches fails | file line count (D11) | n/a |
| Entry removed when | n/a | the suppression is gone and the function is under the limit | the file's excepted functions are all split | n/a |
| G2/D2/D4/D6 suppressions, file allowlist | approved | unchanged | unchanged | unchanged |
Question D13:
D13 — Как не дать «замороженной» функции расти?
Project/branch/task: master, план docs/designs/code-quality-gates.md, открытый вопрос Q3.
ELI10: Пометка #[expect] (в Rust) или biome-ignore (в TS) разрешает функции быть длиннее 80 строк, но не говорит, насколько. write_inner сейчас 203 строки и живёт в файле на 335 строк, так что и проверка размера файла её не держит: она может вырасти до 350, и CI промолчит. Глобальное правило требует, чтобы исключение могло только уменьшаться. Варианты: записать длину каждой такой функции в allowlist, заморозить целиком файлы, где они живут, или полагаться на правило в CLAUDE.md.
Stakes if we pick wrong: самые длинные функции, ради которых всё затевалось, продолжают расти, пока до них не дойдёт D3.
Recommendation: A) Function entries because это ровно правило «может только уменьшаться» для каждой функции, а форматтеры гарантируют, где функция кончается.
Completeness: A=10/10, B=8/10, C=5/10
Pros / cons:
A) Function entries (recommended)
  ✅ Каждая из 18 функций под своим потолком: рост на одну строку роняет CI
  ✅ Тот же механизм, что для файлов: shrink → обнови число, under limit → удали запись
  ❌ Скрипт ищет функцию по имени и закрывающей скобке: ещё ~40 строк в check-size.sh
B) Freeze host files
  ✅ Без нового механизма: те же записи файлов, только ещё 9 штук
  ✅ Ловит рост файла целиком, а не только одной функции
  ❌ Функция может расти, если рядом в файле что-то сократили
C) Rule in CLAUDE.md
  ✅ Ноль кода: правило для агента, проверяется на ревью
  ✅ Ничего не надо удалять, когда долг погашен
  ❌ Глобальное правило «только уменьшается» держится на внимательности, а не на CI
Net: точный потолок на каждую функцию ценой 40 строк скрипта, грубый потолок на файл, или честное слово. Effort: A human ~2 h / CC ~15 min; B human ~45 min / CC ~5 min; C human ~5 min / CC ~1 min.
Header: Function ratchet
Options:
A) Function entries
quality/size-allowlist.txt gets an `fn <path> <name> <lines>` entry for each of the 18 excepted functions; check-size.sh finds the function by name, measures to its closing brace at the same indent, and applies the file rules (over the entry fails, shrink asks to update, under the limit with no suppression asks to remove; 0 or 2+ name matches fails). human ~2 h / CC ~15 min.
B) Freeze host files
Each file holding an excepted function gets a file entry at its current size, even under 500, marked as a freeze so the "under the limit, remove it" rule waits until its excepted functions are split. human ~45 min / CC ~5 min.
C) Rule in CLAUDE.md
No tooling; CLAUDE.md says never to add lines to a function carrying #[expect(clippy::too_many_lines)] or biome-ignore. human ~5 min / CC ~1 min.

State: approved (moot)
Actual answer: no option chosen; D14 (Debt first) leaves no excepted function at gate time, so there is nothing to cap.
Accepted scope: none
History: D13 sent once 2026-10-23; the reply was a question, "а зачем вообще нужны исключения?", which opened E1; D14 answered E1 with Debt first.

### E1: debt first or gates first (reopened by the user's question on D13)
Finding: E1, P1, confidence 9/10, `docs/designs/code-quality-gates.md:58-74` (PR 1 freezes today's violations, D1-D4 pay them later), raised by the user on D13: "а зачем вообще нужны исключения?". Reviewer: Claude.
Plan baseline: scope record (D1): PR 1 = G1-G6 with the current state frozen, then debt PRs D1-D4. Exception machinery approved since: allowlist `quality/size-allowlist.txt` (D1) with app.ts at 1657 after formatting (D5) and walk.rs/tree.rs until D4 (D11); 13 `#[expect]` (G2); `biome-ignore` on `App.notices` (D2), `app.ts:1254`, `inspector.ts:71`, `dom.ts:7` (D4), two on `tests/e2e/mock.ts:216` (D6); D13 pending. Reason to reopen: changed assumption from the user; exceptions exist only because gates land before the debt is paid.
Runtime evidence: the debt today: 7 code files over 500 (`web/src/app.ts` 1555, `web/src/app.css` 1144, `core/src/refresh.rs` 572, `cli/src/install.rs` 569, `core/src/scan/walk.rs` 563, `core/src/scan/tree.rs` 559, `cli/src/query.rs` 524); 13 Rust functions over 80 (clippy); web after width-140 format: 2 over 80 lines (`App.notices` 88, `mock.ts:216` 126), 4 over complexity 25 (`app.ts:1254` 64, `inspector.ts:71` 46, `tests/e2e/mock.ts:216` 34, `dom.ts:7` 29). Guards for refactors: `cargo test --workspace`, the skill contract test, 6 unit test files and 5 e2e spec files (30 Playwright tests), README screenshot specs.
Comparison grid:
| Choice | Current | A Debt first | B Gates first | C One PR |
|---|---|---|---|---|
| E1 PR order | gates, then D1-D4 (scope record) | format PR, then debt PRs D1-D4 plus the 5 web functions, then the gates PR last | as planned | gates and all debt in one PR |
| Exceptions at gate time | n/a | none: 0 `#[expect]`, 0 `biome-ignore`, no allowlist entries | 13 `#[expect]`, 6 `biome-ignore`, 7 file entries (+ D13) | none |
| `quality/size-allowlist.txt`, ratchet code in check-size.sh | approved (D1, G1) | not built; check-size.sh only enforces 500 / 1000; the allowlist is added the day the user agrees to a first exception | built | not built |
| D13 function ratchet | pending | moot | asked again | moot |
| Approved values superseded | n/a | D2/D4/D6 suppressions, D5 "app.ts at 1657", D11 "walk/tree in the allowlist", D1 allowlist file (folder and job stay) | none | same as A |
| Gates protect new code from | n/a | the last PR (after the debt PRs) | PR 1 | that PR |
| Biggest PR | n/a | D1 (app.ts split, ~1650 lines moved) | D1 | everything (~5000 lines moved, estimate) |
| D8-D12 (entrypoint, missing tool, size scope, whole file, CSS 500), D3 nesting 4, D5 format 140 | approved | kept | kept | kept |
Question D14:
D14 — Сначала долг, потом проверки, и тогда без исключений?
Project/branch/task: master, план docs/designs/code-quality-gates.md, порядок PR (пересмотр по вашему вопросу к D13).
ELI10: Исключения в плане появились только из-за порядка работ. Проверки включаются первым PR, когда в коде ещё есть 7 длинных файлов, 13 длинных Rust-функций и 5 длинных или запутанных веб-функций. Чтобы CI не покраснел сразу, их пришлось заморозить пометками и списком. Если сначала распилить этот долг (несколько PR, каждый под защитой уже существующих тестов), а проверки включить последним PR, исключений не будет вовсе: ни allowlist, ни #[expect], ни biome-ignore, а скрипт размеров сводится к проверке «500 / 1000». Цена: пока идут PR с долгом, новые нарушения ловить нечем.
Stakes if we pick wrong: с исключениями тянем механизм заморозки, который нужен ровно до конца распила; без них несколько дней работаем без проверок.
Recommendation: A) Debt first because распил делает агент, тесты уже защищают поведение, а в итоге та же чистая кодовая база без механизма исключений и без D13.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Debt first (recommended)
  ✅ Ни одного исключения в коде: нет 13 #[expect], 6 biome-ignore и allowlist
  ✅ check-size.sh простой (только лимиты), D13 и правила ratchet не нужны
  ❌ Пока идут PR с долгом, CI не ловит новые длинные функции и файлы
B) Gates first
  ✅ Новый код под защитой с первого же PR
  ✅ Долг можно гасить в любом темпе, хоть через месяцы
  ❌ Механизм исключений и вопрос D13 нужны только до конца распила
C) One PR
  ✅ Нет ни окна без проверок, ни исключений
  ✅ Одна итерация ревью вместо пяти-шести
  ❌ PR примерно на 5000 перемещённых строк (оценка): регрессию в нём трудно найти
Net: короткое окно без проверок ради отсутствия механизма исключений, или защита с первого дня ценой временной заморозки. Effort: debt the same in all (human ~1 week / CC ~3-5 h); A and C drop the ratchet (CC ~15 min less); B adds D13.
Header: PR order
Options:
A) Debt first
Order: a Biome format PR, then debt PRs (D1 app.ts, D2 app.css, D3 13 Rust functions, D4 5 Rust files, plus App.notices, app.ts:1254, inspector.ts:71, dom.ts:7, mock.ts:216), then the gates PR with every limit on and zero exceptions; check-size.sh enforces 500 / 1000 only and an allowlist is added only after the user agrees to an exception. Supersedes the D2/D4/D6 suppressions, D5's 1657 entry, D11's walk/tree entries and D1's allowlist file; D13 closes as moot.
B) Gates first
Keep the order as planned: PR 1 with the gates and every current violation frozen (allowlist, #[expect], biome-ignore), then debt PRs D1-D4; D13 is asked again.
C) One PR
One PR with the gates and all debt paid, zero exceptions; same supersessions as A; D13 moot.

State: approved
Actual answer: A) Debt first (D14, answered 2026-10-23: "Debt first")
Accepted scope: Order: a Biome format PR, then debt PRs (D1 app.ts, D2 app.css, D3 13 Rust functions, D4 5 Rust files, plus App.notices, app.ts:1254, inspector.ts:71, dom.ts:7, mock.ts:216), then the gates PR with every limit on and zero exceptions; check-size.sh enforces 500 / 1000 only and an allowlist is added only after the user agrees to an exception. Supersedes the D2/D4/D6 suppressions, D5's 1657 entry, D11's walk/tree entries and D1's allowlist file; D13 closes as moot.
History: none

Section 2 dispositions: C1 accepted (D10, code only); C2 accepted (D11, whole file); C3 accepted (D12, 500 like code); C4 moot after E1 (D13 reply led to D14); E1 accepted (D14, debt first); C5 accepted as part of the G1 contract (no question).

### Section 3: Tests

Test framework detection: `CLAUDE.md` has no `## Testing` section; detected from the repo. Rust: `cargo test --workspace`, 111 tests in 8 binaries (inline unit tests, `core/tests/{publish,queries,schema}.rs`, `cli/tests/{cli,faults,skill_contract}.rs`). Web: `bun test tests/unit`, 6 files, 34 tests; Playwright, 5 spec files, 30 tests (`web/playwright.config.ts`); README screenshots `web/tests/shots/readme.spec.ts`, not in CI. Shell: bash test scripts run by CI, precedent `scripts/fault-fs.sh` (sudo, rust job). CI proves failure paths as jobs, precedent `no-bun` (`.github/workflows/ci.yml:84-95`). No coverage tool in CI; measured for this review with cargo-llvm-cov 0.9.1 (installed locally): workspace 5079/6581 lines (77%), as user `tass` without root, so root-only branches read as unreached.

```
CODE PATHS (gates PR, new code)                                   USER FLOWS (agent or developer)
[+] scripts/check-size.sh (part `size`)                            [+] Push with a violation
  ├── [GAP] outside a git work tree -> exit 1                         ├── [GAP] [→E2E] 81-line fn: hook blocks the push, names the rule
  ├── [GAP] zero files in scope -> exit 1                             ├── [GAP] fresh clone without the hook: CI fails the PR
  ├── [GAP] scope D10: code extensions + .githooks/*, not docs/**     └── [GAP] dirty work tree: the hook checks the tree, not the commit
  ├── [GAP] test class: */tests/**, */tests.rs, web/tests/**         [+] Missing tool
  ├── [GAP] whole file D11; a last line without \n counts             ├── [GAP] no shellcheck: exit 1 with the install hint (D9)
  ├── [GAP] 500 pass / 501 fail (CSS too, D12); 1000 / 1001 tests     └── [GAP] the same in CI
  ├── [GAP] tracked file deleted in the work tree -> skipped         [+] CI wiring
  └── [GAP] names with spaces (git ls-files -z)                       └── [GAP] typo in a part name: exit 2, never a silent pass
[+] scripts/check.sh [rust|size|shell|tsc|biome]...
  ├── [GAP] no argument -> all five parts
  ├── [GAP] unknown part -> exit 2 with the list of parts
  ├── [GAP] a failing part -> exit 1 even when later parts pass
  ├── [GAP] tool missing -> exit 1 with its install hint (D9)
  └── [GAP] started from a subdirectory -> runs at the repo root
[+] gate wiring (config)
  ├── [GAP] too_many_lines 80 fires in core AND cli
  ├── [GAP] excessive_nesting 4 fires
  └── [GAP] Biome length 81, complexity 26, format 140, unused suppression -> exit 1, in src and tests

EXISTING BEHAVIOR AT RISK (debt PRs: refactors, no intended change)
[+] Rust, the 13 functions: 1530 lines, cargo test reaches 1097 (72%)
  ├── [★★★ TESTED] finalize_with 124/125, queries status build 93/94, graft 96/104, walk_with_mounts 98/108
  ├── [★★  TESTED] load_tree 126/144, refresh_locked 115/137, diff_pair 117/137, scan_locked 83/99,
  │                write_inner 121/158, read_dir 84/108 (error and rare-kind branches unreached)
  ├── [GAP] cli/src/status.rs:129 details 0/113: the text `freespace status` (tests use --json)
  ├── [GAP] cli/src/doctor.rs:259 snapshot_checks 35/83: the failing checks
  └── [CI only] cli/src/install.rs:83 run 5/120 (install job, real VM); writer disk full (fault-fs.sh)
[+] Rust files in D4: cli/src/query.rs 175/346 (51%, text tables), refresh.rs 87%, walk.rs 87%, tree.rs 93%,
    install.rs 8% (install job)
[+] Web: App, 70 methods; 30 e2e + 34 unit
  ├── [★★★ TESTED] browse, find, changes, top, reload and notices, states, axe at 1280/700/360
  ├── [GAP] [→E2E] keys handled but never pressed: ArrowUp, ArrowRight, Home, PageUp, PageDown, Backspace
  ├── [GAP] [→E2E] Show more (app.ts:753 showMore), cancel a search (app.ts:664 cancel)
  └── [GAP] [→E2E] app.css split (D2): rule order; nothing compares rendering

COVERAGE (new code): 0/16 paths, 0/6 flows (the code does not exist yet)
EXISTING AT RISK: Rust 72% of the long functions' lines; web 6 keys + 2 actions unreached; rendering unguarded
```

Findings:

1. T1 [P1] (confidence: 9/10) Regression rule, Rust debt (D3, D4). `docs/designs/code-quality-gates.md:72` guards D3 with "the existing tests". llvm-cov on the 13 functions: 1097/1530 lines (72%). Unreached and reachable without root: `cli/src/status.rs:129` `details` 0/113 (the whole text `status`); `cli/src/query.rs` 175/346 (text tables of ls/top/find/diff); `cli/src/doctor.rs:259` 35/83 (failing checks at `:270`, `:290`, `:316`, `:332`, `:358`); `core/src/refresh.rs:270` (refresh of a deleted folder: REMOVED), `:445` (path resolves elsewhere), `:455` (root gone); `core/src/queries/diff.rs:180,254` (error paths); `core/src/job.rs:155` (no roots). Root-only or fault paths: `install::run` (install job), writer disk full (`scripts/fault-fs.sh`); the mount-point refusal (`refresh.rs:402`) needs a mount, so it stays unreached. Reviewer: Claude.
2. T2 [P1] (confidence: 9/10) Regression rule, web debt (D1, D2, the 5 functions). `docs/designs/code-quality-gates.md:70` "README screenshots regenerate identically" does not hold today: two runs of `bun run shots` differ by 16 px, and a run differs from the committed `docs/img/gui.png` by 336 px and `gui-changes.png` by 464 px, all in the header's disk bar (2364,64 at 2x), which `web/src/app.css:209` animates with `transition: width 400ms`; the shot lands mid-transition. Unreached by e2e: 6 handled keys, `showMore` (`web/src/app.ts:753`), `cancel` (`:664`). Nothing compares rendering, so a CSS split that reorders rules passes every test. Reviewer: Claude.
3. T3 [P2] (confidence: 9/10) G2 puts the lints in `[workspace.lints.clippy]`; a member gets them only with `[lints] workspace = true`, and neither `core/Cargo.toml` nor `cli/Cargo.toml` has a `[lints]` table today. Without it `too_many_lines` never fires for that crate and CI stays green. Adding `[lints] workspace = true` to both is part of the approved G2 (no question). How to prove every gate fires, now and after later edits (a raised threshold, a new crate): D17. Reviewer: Claude.
4. T4 [P2] (confidence: 9/10) Required proof of approved behavior (D8-D12, D14, C5), no question: `scripts/test-check.sh`, a bash test like `scripts/fault-fs.sh`, builds throwaway git repos with `mktemp -d` and runs the real scripts. Table rows for check-size.sh: outside a work tree; zero files; each scope rule of D10 (an over-limit `.md`, `.json`, `.png`, `docs/x.js` pass; `.githooks/pre-push` counts); test classification of the three patterns; 500/501 source, 500/501 `.css`, 1000/1001 test; a 501st line without a final newline fails; a tracked file deleted in the tree is skipped; a path with a space. Rows for check.sh, with stub `cargo`, `bunx` and `shellcheck` first on `PATH` that log their argv and exit with a chosen code: no argument runs all five parts; unknown part exits 2 and lists the parts; a failing first part makes the exit 1 though the rest pass; a tool missing from `PATH` exits 1 with its install hint; started from `web/` it behaves the same. Unknown part and subdirectory are not new policy: a part list must reject names it does not know, or a typo in `ci.yml` passes. Runs in the `quality` job; shellcheck covers it through `scripts/*.sh`. Reviewer: Claude.
5. T5 [P3] (confidence: 8/10) The pre-push hook checks the work tree, not the commits being pushed: uncommitted edits can make it pass or fail differently from CI. Stashing in a hook risks the user's changes, and checking out the pushed commit means a cold build. CI stays the gate; CONTRIBUTING.md says so in one line. No question. Reviewer: Claude.
6. T6 [P3] (confidence: 9/10) Tests made obsolete by this plan: none. Moving the walk/tree inline tests to `tests.rs` keeps module paths and names (`cargo test -- --list` before and after). LLM/eval scope: none; `CLAUDE.md` lists no prompt patterns, `skill/` is untouched, `cli/tests/skill_contract.rs` keeps running. Reviewer: Claude.

### T1: regression contract for the Rust debt PRs
Finding: T1, P1, confidence 9/10, `docs/designs/code-quality-gates.md:72`, llvm-cov receipts above, reviewer Claude.
Plan baseline: D3 "Guard: the existing tests, plus `split_passes_answer_like_one_pass`-style equivalence where a split touches query logic"; D4 none stated. Fixed in every column: debt first (D14), whole-file counting (D11), zero exceptions at gate time (D14); behavior to preserve: every CLI output (text and `--json`), exit codes, error messages and fixes, snapshot contents; intended changes: none.
Runtime evidence: 111 tests pass (`cargo llvm-cov --workspace`); long functions 1097/1530 lines; `details` 0/113; `query.rs` 51%; `doctor` `snapshot_checks` 35/83; refresh branches `refresh.rs:270,402,445,455` unreached; CI-only: install job, fault-fs.sh.
Comparison grid:
| Choice | Current | A Characterize first | B Existing suite | C Differential only |
|---|---|---|---|---|
| New permanent tests before the split | none | about 8 tests or table rows at the CLI or core API for the unreached branches above that run without root; they assert labels, values and codes, not spacing | none | none |
| One-time old vs new binary check | none | yes: master and the PR binary answer `status`, `ls`, `top`, `find`, `diff`, `doctor`, text and `--json`, byte-identical on one fixture snapshot; scan-path PRs compare the `--json` answers of fresh scans after dropping time fields | no | yes, same as A |
| Test names before = after (`cargo test --workspace -- --list`) | n/a | yes | yes | yes |
| CI jobs (install, fault-fs.sh, musl, msrv) green | yes | yes | yes | yes |
| Effort on top of the split | n/a | human ~2 days / CC ~1.5 h | none | human ~4 h / CC ~20 min |
Question D15:
D15 — Как защитить поведение при распиле Rust-кода?
Project/branch/task: master, план docs/designs/code-quality-gates.md, debt PRs D3 (13 функций) и D4 (5 файлов).
ELI10: Распил длинных функций не должен ничего менять снаружи: вывод команд, коды ошибок, содержимое снапшота. Покрытие показало, что тесты проходят через 72% строк этих 13 функций. Совсем не проверены текстовый `freespace status` (0 из 113 строк), текстовые таблицы `ls/top/find/diff` (половина query.rs), падающие проверки doctor и refresh удалённой папки. Можно сначала дописать тесты на эти места и только потом пилить, можно один раз сравнить вывод старого и нового бинарника, а можно пилить под защитой того, что есть.
Stakes if we pick wrong: распил тихо меняет текстовый вывод или сообщение об ошибке, тесты зелёные, а пользователь и skill видят другое.
Recommendation: A) Characterize first because новые тесты останутся и после распила, а разовое сравнение бинарников ловит всё, что дошло до вывода.
Completeness: A=10/10, B=6/10, C=8/10
Pros / cons:
A) Characterize first (recommended)
  ✅ Каждая непроверенная ветка, достижимая без root, получает тест до распила и остаётся под защитой потом
  ✅ Старый и новый бинарник дают байт в байт одинаковый вывод всех команд на одном снапшоте
  ❌ Около 8 новых тестов и сравнение до распила: human ~2 days / CC ~1.5 h сверху
B) Existing suite
  ✅ Ничего нового: 111 тестов, install job и fault-fs.sh уже в CI на каждом PR
  ✅ Распил начинается сразу, без подготовительной работы
  ❌ Текстовый status, таблицы и падающие проверки doctor могут сломаться незаметно
C) Differential only
  ✅ Любая разница в выводе на фикстуре видна, без новых постоянных тестов
  ✅ Дёшево: human ~4 h / CC ~20 min на все PR
  ❌ Ветки ошибок фикстура не достигает, и следующий рефакторинг снова без защиты
Net: тесты, которые остаются, ценой подготовки, или быстрый распил под 72% покрытия.
Header: Rust guard
Options:
A) Characterize first
Before each Rust debt PR splits a function or file, add tests at the CLI or core API for its unreached branches that run without root (status text, query text tables, doctor failing checks, refresh of a deleted folder, path resolving elsewhere, root gone, diff error paths, scan with no roots; about 8 tests or table rows asserting labels, values and codes). Then split. Acceptance: test names before = after plus the new ones; all CI jobs green; master and PR binaries give byte-identical text and `--json` answers for status, ls, top, find, diff, doctor on one fixture snapshot (scan-path PRs: fresh scans compared after dropping time fields). No intended behavior change. human ~2 days / CC ~1.5 h.
B) Existing suite
Split under the 111 existing tests and the CI jobs; acceptance: test names before = after, all green. No intended behavior change. No extra effort.
C) Differential only
No new tests; acceptance: test names before = after, all CI jobs green, and the one-time master vs PR binary comparison from A. No intended behavior change. human ~4 h / CC ~20 min.

State: approved
Actual answer: A) Characterize first (D15, answered 2026-10-23: "Characterize first (recommended)")
Accepted scope: Before each Rust debt PR splits a function or file, add tests at the CLI or core API for its unreached branches that run without root (status text, query text tables, doctor failing checks, refresh of a deleted folder, path resolving elsewhere, root gone, diff error paths, scan with no roots; about 8 tests or table rows asserting labels, values and codes). Then split. Acceptance: test names before = after plus the new ones; all CI jobs green; master and PR binaries give byte-identical text and `--json` answers for status, ls, top, find, diff, doctor on one fixture snapshot (scan-path PRs: fresh scans compared after dropping time fields). No intended behavior change. human ~2 days / CC ~1.5 h.
History: none

### T2: regression contract for the web debt PRs
Finding: T2, P1, confidence 9/10, `docs/designs/code-quality-gates.md:70` ("README screenshots regenerate identically"), `web/src/app.css:209`, `web/src/app.ts:664,753`, reviewer Claude.
Plan baseline: D1 "Guard: the 30 Playwright e2e tests and the unit tests pass unchanged; README screenshots regenerate identically"; D2 none stated; the 5 web functions (D14) none stated. Fixed in every column: debt first (D14), Biome format width 140 lands first (D5), same rules for tests (D6); behavior to preserve: everything the page shows and does at 1280, 700 and 360 px, keys, focus, URLs, API calls; intended changes: none.
Runtime evidence: two runs of `bun run shots` differ by 16 px; a run against the committed PNGs: `gui.png` 336 px, `gui-changes.png` 464 px, all inside a 42x8 / 58x8 box at (2364,64) / (2348,64), the header disk bar, `transition: width 400ms` (`app.css:209`); e2e presses Escape, Enter, h, ArrowDown, t, r, d, /, Tab, End, ArrowLeft, ?; the key handler also takes ArrowUp, ArrowRight, Home, PageUp, PageDown, Backspace; no e2e reaches `showMore` or `cancel`.
Comparison grid:
| Choice | Current | A Close gaps + pixel diff | B Existing suites | C Committed baselines |
|---|---|---|---|---|
| README shots | animated bar, differ per run | `animations: "disabled"` in readme.spec.ts, PNGs regenerated once in the format PR | same fix as A | same fix as A |
| New e2e cases | none | one table test over the 6 keys (browse.spec.ts), Show more, cancel a search (find.spec.ts) | none | as A |
| Rendering check for D1, D2 and the 5 functions | none | one-time: a scratch Playwright run (not committed) shoots every view and state the e2e mock reaches at 1280, 700, 360 px before and after; zero differing pixels | README shots only (2 views at 1280), zero differing pixels | `toHaveScreenshot` baselines in CI for the same set, generated on the CI image, updated on every Chromium bump and intended UI change |
| Test names before = after (bun test, Playwright `--list`) | n/a | yes | yes | yes |
| Effort on top of the split | n/a | human ~1 day / CC ~1 h | human ~1 h / CC ~10 min | human ~2 days / CC ~2 h, plus upkeep |
Question D16:
D16 — Как защитить поведение при распиле App и app.css?
Project/branch/task: master, план docs/designs/code-quality-gates.md, debt PRs D1 (app.ts), D2 (app.css) и 5 веб-функций.
ELI10: app.ts (1555 строк) и app.css (1144) распиливаются на модули, и снаружи ничего не должно поменяться. 30 e2e-тестов проходят по основным сценариям, но шесть клавиш (ArrowUp, ArrowRight, Home, PageUp, PageDown, Backspace), кнопку Show more и отмену поиска не проверяет ничто. Распил CSS опасен порядком правил: тесты смотрят на DOM, а не на то, как страница выглядит. Скриншоты README для сравнения сейчас не годятся: полоса диска в шапке анимируется 400 мс, и каждый прогон даёт другие пиксели (16 px между двумя прогонами, 336 px против закоммиченного).
Stakes if we pick wrong: распил тихо ломает клавишу или вёрстку на узком экране, тесты зелёные, а видно это только пользователю.
Recommendation: A) Close gaps + pixel diff because дыры в e2e закрываются навсегда, а разовое попиксельное сравнение ловит сдвиг CSS без постоянных эталонов, которые надо обновлять.
Completeness: A=9/10, B=6/10, C=10/10
Pros / cons:
A) Close gaps + pixel diff (recommended)
  ✅ Шесть клавиш, Show more и отмена поиска получают e2e и остаются под защитой после распила
  ✅ Каждый экран и состояние на 1280, 700 и 360 px сравниваются попиксельно до и после распила
  ❌ Попиксельное сравнение разовое: следующий рефакторинг CSS снова без него (human ~1 day / CC ~1 h)
B) Existing suites
  ✅ Только починка скриншотов README: одна строка, human ~1 h / CC ~10 min
  ✅ 30 e2e и 34 unit уже проходят по основным сценариям
  ❌ Клавиши, Show more и узкие экраны проверяются глазами или никак
C) Committed baselines
  ✅ Каждый будущий PR сравнивает рендер с эталоном, не только этот распил
  ✅ Закрывает те же дыры в e2e, что и A
  ❌ Эталоны надо генерировать на образе CI и обновлять при каждом обновлении Chromium и правке UI
Net: разовая проверка вёрстки на время распила, или постоянная ценой обслуживания эталонов.
Header: Web guard
Options:
A) Close gaps + pixel diff
readme.spec.ts takes its shots with `animations: "disabled"` and the PNGs are regenerated once in the format PR. Before D1, e2e gains one table test over ArrowUp, ArrowRight, Home, PageUp, PageDown, Backspace (browse.spec.ts), Show more, and cancelling a search (find.spec.ts). For D1, D2 and the 5 web functions: a scratch Playwright run, not committed, shoots every view and state the e2e mock reaches at 1280, 700 and 360 px before and after; zero differing pixels. Acceptance: test names before = after plus the new ones; all green. No intended behavior change. human ~1 day / CC ~1 h.
B) Existing suites
readme.spec.ts fix as in A; D1, D2 and the 5 functions split under the 30 e2e and 34 unit tests; acceptance: test names before = after, all green, README shots pixel-identical before and after. No intended behavior change. human ~1 h / CC ~10 min.
C) Committed baselines
Everything in A, and the screenshot set becomes `toHaveScreenshot` baselines checked in CI, generated on the CI image and updated with each Chromium bump and intended UI change. No intended behavior change. human ~2 days / CC ~2 h, plus upkeep.

State: approved
Actual answer: A) Close gaps + pixel diff (D16, answered 2026-10-23: "Close gaps + pixel diff (recommended)")
Accepted scope: readme.spec.ts takes its shots with `animations: "disabled"` and the PNGs are regenerated once in the format PR. Before D1, e2e gains one table test over ArrowUp, ArrowRight, Home, PageUp, PageDown, Backspace (browse.spec.ts), Show more, and cancelling a search (find.spec.ts). For D1, D2 and the 5 web functions: a scratch Playwright run, not committed, shoots every view and state the e2e mock reaches at 1280, 700 and 360 px before and after; zero differing pixels. Acceptance: test names before = after plus the new ones; all green. No intended behavior change. human ~1 day / CC ~1 h.
History: none

### T3: proof that every gate fires
Finding: T3, P2, confidence 9/10, `docs/designs/code-quality-gates.md:62` (G2 `[workspace.lints.clippy]`), `core/Cargo.toml` and `cli/Cargo.toml` (no `[lints]` table), reviewer Claude.
Plan baseline: none; G1-G6 add the gates and nothing shows they fail on a violation. Fixed in every column: `[lints] workspace = true` added to core and cli as part of G2 (no question); `scripts/check.sh` parts (D8); zero exceptions at gate time (D14); limits 500 / 1000 / 80, nesting 4, complexity 25, format 140 (D3-D5, D11, D12); `scripts/test-check.sh` for check-size.sh and check.sh (T4, required).
Runtime evidence: `grep -n '^\[lints' core/Cargo.toml cli/Cargo.toml Cargo.toml`: no match; no `clippy.toml` yet. A member without `[lints] workspace = true` ignores `[workspace.lints]`, so `too_many_lines` (pedantic, off by default) never fires there; `excessive_nesting` comes from `clippy.toml` and fires anyway. Precedent for a CI job that proves a failure: `no-bun` (`.github/workflows/ci.yml:84-95`). Warm clippy on this workspace: 1-5 s; Biome: 0.11 s for 21 files.
Comparison grid:
| Choice | Current | A Canary in CI | B Once in the gates PR |
|---|---|---|---|
| Where the proof runs | n/a | every CI run: `scripts/canary.sh` in the rust job after clippy | once, by hand, outputs pasted in the gates PR body |
| How | n/a | a temporary `git worktree` of HEAD (never the working tree), one violation per gate at limit + 1: an 81-line function in every workspace member (from `cargo metadata`), a block nested 5 deep, an 81-line TS function in web/src and in web/tests, a complexity-26 function, an unformatted file, an unused `biome-ignore`, a 501-line `.rs` and `.css`, a 1001-line test file; each `scripts/check.sh <part>` must exit 1 and name the rule | the same injections once |
| Catches later drift (a raised threshold, a new crate without `[lints] workspace = true`, a dropped `-D warnings`) | n/a | yes: the limit + 1 case passes and the canary fails | only a member without `[lints] workspace = true`: `check.sh rust` fails when a member lacks it |
| CI time | n/a | about +1 min in the rust job (estimate: clippy re-checks both crates from the new path) | none |
| Effort | n/a | human ~4 h / CC ~30 min | human ~2 h / CC ~15 min |
Question D17:
D17 — Как доказать, что новые проверки действительно срабатывают?
Project/branch/task: master, план docs/designs/code-quality-gates.md, gates PR (G2, G3, scripts/check.sh).
ELI10: Проверка, которая никогда не падает, выглядит так же, как работающая. Пример прямо из нашего репо: правило длины функций ставится в Cargo.toml на весь workspace, но крейт его получает, только если в его собственном Cargo.toml есть `[lints] workspace = true`. Сейчас её нет ни в core, ни в cli. Забыть эту строку, и CI зелёный при функции в 300 строк. Можно один раз вручную подложить нарушения и убедиться, что всё падает, а можно сделать это постоянной проверкой в CI, как уже сделан job no-bun.
Stakes if we pick wrong: проверку тихо выключает забытая строка, поднятый порог или новый крейт, агент пишет функцию-монстра, а CI зелёный.
Recommendation: A) Canary in CI because это ровно тот случай, ради которого всё затевалось: ИИ поднимает порог или забывает строку, и CI должен покраснеть.
Completeness: A=10/10, B=7/10
Pros / cons:
A) Canary in CI (recommended)
  ✅ Каждый прогон CI доказывает, что каждое правило падает на нарушении ровно на 1 больше лимита
  ✅ Поднятый порог, новый крейт без строки или пропавший -D warnings краснят CI сразу
  ❌ Ещё один скрипт и около минуты в rust job (оценка), human ~4 h / CC ~30 min
B) Once in the gates PR
  ✅ Без нового скрипта и без лишнего времени в CI
  ✅ check.sh rust сам падает, если у крейта нет `[lints] workspace = true`
  ❌ Поднятый порог или пропавший флаг позже никто не заметит
Net: постоянное доказательство ценой минуты CI, или разовое ценой слепоты к будущим правкам конфигов.
Header: Gate proof
Options:
A) Canary in CI
Add `scripts/canary.sh`: in a temporary git worktree of HEAD (never the working tree) inject one violation per gate at limit + 1 (an 81-line function in every workspace member from `cargo metadata`, a block nested 5 deep, an 81-line TS function in web/src and web/tests, a complexity-26 function, an unformatted file, an unused `biome-ignore`, a 501-line `.rs` and `.css`, a 1001-line test file) and require each `scripts/check.sh <part>` to exit 1 naming the rule. Runs in the rust job after clippy, reusing its target, with `bun install --frozen-lockfile` in web/ for Biome; shellcheck covers it. human ~4 h / CC ~30 min; about +1 min CI (estimate).
B) Once in the gates PR
Do the same injections once by hand before merging the gates PR and paste the outputs in its body; `scripts/check.sh rust` fails when a workspace member lacks `[lints] workspace = true`. human ~2 h / CC ~15 min.

State: approved
Actual answer: A) Canary in CI (D17, answered 2026-10-23: "Canary in CI (recommended)")
Accepted scope: Add `scripts/canary.sh`: in a temporary git worktree of HEAD (never the working tree) inject one violation per gate at limit + 1 (an 81-line function in every workspace member from `cargo metadata`, a block nested 5 deep, an 81-line TS function in web/src and web/tests, a complexity-26 function, an unformatted file, an unused `biome-ignore`, a 501-line `.rs` and `.css`, a 1001-line test file) and require each `scripts/check.sh <part>` to exit 1 naming the rule. Runs in the rust job after clippy, reusing its target, with `bun install --frozen-lockfile` in web/ for Biome; shellcheck covers it. human ~4 h / CC ~30 min; about +1 min CI (estimate).
History: none

Section 3 dispositions: T1 accepted (D15, characterize first); T2 accepted (D16, close gaps + pixel diff); T3 accepted (D17, canary in CI; `[lints] workspace = true` in core and cli as part of G2); T4 accepted as required proof of D8-D12, D14, C5 (`scripts/test-check.sh`, no question); T5 accepted as a documented limit (hook checks the work tree; CI is the gate); T6: no tests retired, no eval scope.

### Section 4: Performance

Measured on this machine (8 CPUs, VMware guest, load average 7.97 during the runs), warm caches, as user `tass`:

| Check | Time |
|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings`, warm | 1-5 s (no-op 5.3 s, after touching core 2.3 s, after touching cli 1.0 s) |
| same, from a fresh `git worktree` sharing `target/` (the canary's case), `-j 2` | 9 s; second run 0 s; the main tree afterwards 2 s |
| `cargo fmt --all --check` | 0.37 s |
| `shellcheck install.sh scripts/*.sh` | 0.43 s |
| `bunx tsc --noEmit` (web) | 1.0-1.2 s |
| Biome lint (21 files) | 0.11 s |
| `git ls-files -z` | 0.009 s |

Findings:

1. PF1 [P2] (confidence: 9/10) `docs/designs/code-quality-gates.md:81` (Q5) says workspace clippy "is about a minute when the build is warm". Measured: 1-5 s, and the whole hook (`scripts/check.sh`, all five parts) about 3-8 s warm. A fresh clone pays the dependency build once (not measured). Q5 is answered by the plan's own baseline: the hook checks the whole workspace; no changed-crate logic. No question. Reviewer: Claude.
2. PF2 [P3] (confidence: 8/10) D17's canary: clippy re-checks both crates from the worktree path in 9 s locally, so "+1 min (estimate)" is an upper bound. Under `-D warnings` a failing `core` stops clippy before `cli` (cli depends on core), so the canary runs clippy once per member (`-p`), which the approved "every workspace member" already implies. No question. Reviewer: Claude.
3. PF3 [P2] (confidence: 7/10) D3 splits the per-entry hot path: `core/src/scan/walk.rs:301` `read_dir`, `:137` `walk_with_mounts`, `core/src/scan/tree.rs:281` `finalize_with`, `core/src/snapshot/writer.rs:119` `write_inner`, `core/src/refresh.rs:64` `load_tree`, `:248` `graft`. A split that passes data between the new functions through a `clone()`, a `collect()` into a temporary `Vec` or a per-entry allocation costs time per entry: a warm scan of `/usr` is 546,555 entries, the README machine 7.8 million. Release is `lto = "thin"`, `codegen-units = 4` (`Cargo.toml:33-36`), so plain splits inline. Measuring on this machine is coarse: 11 warm `/usr` scans fell from 8.0 s to 3.8 s wall (walk 2.6 to 0.9 s, index 3.5 to 1.8 s); CPU time (`perf stat` task-clock, run under sudo) 7.7-9.8 s, about ±12%; hardware instruction counters are not supported in this VM; no valgrind or heaptrack. The README's cold benchmark (288 s and 399 s for one binary) cannot see a refactor at all. Reviewer: Claude.
4. No N+1, cache or memory issue in the new code: check-size.sh reads about 130 tracked files once; Biome and tsc run once per push; the `quality` job adds `bun install --frozen-lockfile` like the web job.

### PF3: speed guard for the hot-path splits
Finding: PF3, P2, confidence 7/10, `core/src/scan/walk.rs:137,301`, `core/src/scan/tree.rs:281`, `core/src/snapshot/writer.rs:119`, `core/src/refresh.rs:64,248`, reviewer Claude.
Plan baseline: no speed check for D3. Fixed in every column: D15 characterize first (tests, test names, byte-identical outputs); debt first (D14); release profile unchanged.
Runtime evidence: 11 warm scans of `/usr` (546,555 entries): 21.6 s (first, cold-ish), then 8.0, 7.3, 6.9, 5.6, 5.5, 5.0, 4.3, 3.9, 3.8, 4.4 s; CPU time of 5 more: 9848, 8800, 8403, 8554, 7714 ms; `instructions:u` `<not supported>`; load average 7.97 on 8 CPUs.
Comparison grid:
| Choice | Current | A Interleaved CPU time | B Review only |
|---|---|---|---|
| Speed check for PRs that touch walk.rs, tree.rs, writer.rs, refresh.rs | none | master and PR release binaries alternate 7 warm `/usr` scans each (`perf stat` task-clock); median CPU time of the PR above master by more than 15% blocks the PR until explained; per-phase seconds from `--json` (walk, finalize, insert, index) in the PR body | none |
| Per-entry code review | none | the PR body lists every new `clone()`, `collect()` or allocation inside per-entry loops, or says there are none | same as A |
| What it catches | n/a | a slowdown above about 15% on this machine, and the allocation pattern behind it | the allocation pattern, if the reviewer reads it right |
| Effort per hot-path PR | n/a | human ~1 h / CC ~5 min (about 2 min of scans) | human ~15 min / CC ~2 min |
Question D18:
D18 — Стеречь ли скорость скана при распиле горячих функций?
Project/branch/task: master, план docs/designs/code-quality-gates.md, debt PR D3 (функции скана walk, tree, writer, refresh).
ELI10: Шесть из 13 длинных функций вызываются на каждую запись диска: полмиллиона раз на /usr и почти 8 миллионов на машине из README. Сам распил обычно бесплатен, компилятор всё встраивает обратно. Но если при распиле данные между новыми функциями начнут копироваться или собираться во временные списки, скан замедлится, а тесты этого не заметят. Мерить на этой машине грубо: время прыгает от 8 до 4 секунд, CPU-время на ±12%, счётчиков инструкций в этой VM нет. Поэтому замер ловит только замедление больше ~15%.
Stakes if we pick wrong: скан тихо становится медленнее, хотя скорость скана и есть то, чем мы хвастаемся в README.
Recommendation: A) Interleaved CPU time because две минуты замеров на PR ловят грубое замедление, а список копирований в описании PR ловит его причину.
Completeness: A=9/10, B=6/10
Pros / cons:
A) Interleaved CPU time (recommended)
  ✅ Замедление больше ~15% блокирует PR с цифрами, а не на глаз
  ✅ Секунды по фазам (walk, insert, index) в описании PR показывают, где именно стало медленнее
  ❌ Шум ±12% на этой VM: замедление на 5-10% пройдёт незамеченным (CC ~5 min на PR)
B) Review only
  ✅ Без замеров: PR перечисляет новые clone, collect и аллокации в циклах по записям
  ✅ Дешевле: human ~15 min / CC ~2 min на PR
  ❌ Замедление без явного clone (например, лишний проход по дереву) никто не увидит
Net: две минуты замеров на PR ради цифр, или только чтение кода.
Header: Speed guard
Options:
A) Interleaved CPU time
For each debt PR touching walk.rs, tree.rs, writer.rs or refresh.rs: master and PR release binaries alternate 7 warm scans of /usr each under `perf stat -e task-clock`; a PR median above master's by more than 15% blocks the PR until explained; the PR body carries both medians and the per-phase seconds from `--json`, and lists every new clone(), collect() or allocation inside per-entry loops (or says none). human ~1 h / CC ~5 min per PR.
B) Review only
For the same PRs, the PR body lists every new clone(), collect() or allocation inside per-entry loops (or says none); no timing. human ~15 min / CC ~2 min per PR.

State: approved
Actual answer: B) Review only (D18, answered 2026-10-23: "Review only")
Accepted scope: For each debt PR touching walk.rs, tree.rs, writer.rs or refresh.rs, the PR body lists every new clone(), collect() or allocation inside per-entry loops (or says none); no timing. human ~15 min / CC ~2 min per PR.
History: none

Section 4 dispositions: PF1 accepted (Q5 answered by measurement: whole workspace in the hook, 3-8 s warm); PF2 accepted (canary runs clippy per member; +1 min is an upper bound, 9 s measured); PF3 accepted as B (D18, review only: per-entry clone/collect/allocation list in the PR body, no timing); item 4: no issues.

### Outside voice (Codex, gpt-6-astra, plan-review role)

Codex completed (`OUTSIDE_STATUS: completed provider=codex host=claude`) with 4 findings (1 High, 3 Medium), recommendation: "revise the refactor and verification contracts before implementation because the current checks can both reject unchanged behavior and falsely certify regressions". Verified by Claude against the code before the questions: all four receipts hold (`web/src/app.ts:514-536` one `load()` with `loadSeq` and `AbortController` for every view, `:664-668` cancel bumps the same counter; `web/tests/e2e/mock.ts` has no way to hold or delay a response; `core/src/queries/status.rs:106` live `statvfs`; `core/src/snapshot/writer.rs:93-100` random `snapshot_id` from `/dev/urandom`; `cli/Cargo.toml:18` cli depends on core; `web/playwright.config.ts:21` `reuseExistingServer: !process.env.CI`).

### O1: request coordination through the App split
Finding: O1, High, confidence 9/10, `web/src/app.ts:514-536,664-668`, reviewer Codex (verified by Claude).
Plan baseline: D1 "one controller per view"; D16 guard: keys, Show more, cancel, pixel diff. Fixed in every column: D16 accepted scope; debt first (D14).
Runtime evidence: every view loads through one `load()`: `seq = ++this.loadSeq`, the previous `AbortController` aborted, a response applied only if `seq === this.loadSeq`; `cancel()` bumps the counter. No e2e holds a response, so nothing shows that a late Browse answer cannot overwrite Top after a key press.
Comparison grid:
| Choice | Current | A Apply | B Keep | C Investigate | D Defer |
|---|---|---|---|---|---|
| Who owns loadSeq and the AbortController after D1 | unspecified ("one controller per view") | one request coordinator (today's `load()`); view controllers fetch through it and never apply a response themselves | unspecified | decided after reading the split's first draft | unspecified, revisited later |
| Out-of-order e2e before D1 | none | mock.ts gains a hold-and-release for one request; test: Browse request held, press `t`, Top renders, release Browse: the page still shows Top (rows, URL, header); same for cancel during a held Find | none | none | none |
| Effort | n/a | human ~3 h / CC ~20 min | none | CC ~10 min reading | none |
Question D19:
D19 — Оставить один координатор запросов при распиле App и проверить опоздавший ответ?
Project/branch/task: master, план docs/designs/code-quality-gates.md, debt PR D1 (распил App).
ELI10: Сейчас все виды (Browse, Top, Find, Changes) грузятся через одну функцию load(). Она нумерует запросы и отменяет старый, поэтому ответ, пришедший поздно, не перезапишет экран. План говорит «один контроллер на вид», но не говорит, кто владеет этой нумерацией. Если каждый контроллер заведёт свою, поздний ответ Browse может перерисовать уже открытый Top. Ни один тест сейчас этого не проверяет: mock не умеет придерживать ответ.
Stakes if we pick wrong: после распила при быстром переключении видов страница иногда показывает чужие строки, и ни один тест этого не ловит.
Recommendation: A) Apply because это единственный инвариант App, который ломается без видимых симптомов, а тест на него занимает один новый хук в mock.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Apply (recommended)
  ✅ Один координатор: нумерация и отмена запросов остаются в одном месте после распила
  ✅ e2e с придержанным ответом доказывает, что поздний Browse не перерисует Top
  ❌ Новый хук hold/release в tests/e2e/mock.ts и два теста: human ~3 h / CC ~20 min
B) Keep
  ✅ План не меняется, распил свободен в выборе структуры
  ✅ Нет новых тестов и хуков в mock
  ❌ Гонка при переключении видов остаётся непроверенной именно там, где её легче всего сломать
C) Investigate
  ✅ Решение принимается по первому черновику распила, с реальным кодом на руках
  ✅ Может оказаться, что структура и так сохраняет один load()
  ❌ Вопрос возвращается в середине D1, когда код уже частично перенесён
D) Defer
  ✅ Не держит распил: решение позже, отдельной строкой
  ✅ Остальные решения D1-D18 не трогаются
  ❌ Если распил случится раньше решения, инвариант уйдёт без защиты
Net: один хук в mock и правило «один координатор» против непроверенной гонки.
Header: Request owner
Options:
A) Apply
D1 keeps one request coordinator (today's load(): loadSeq and the AbortController); view controllers fetch through it and never apply a response themselves. Before D1, tests/e2e/mock.ts gains a hold-and-release for one request, and e2e gains: Browse request held, press t, Top renders, release Browse, the page still shows Top (rows, URL, header); a held Find cancelled, then released, changes nothing. human ~3 h / CC ~20 min.
B) Keep
D1 and D16 as approved; ownership of request sequencing left to the split.
C) Investigate
Read the first draft of the D1 split and decide then whether the coordinator rule and the out-of-order tests are needed.
D) Defer
Leave this proposal unresolved; D1 and D16 stay as approved.

State: approved
Actual answer: A) Apply (D19, answered 2026-10-23: "Apply (recommended)")
Accepted scope: D1 keeps one request coordinator (today's load(): loadSeq and the AbortController); view controllers fetch through it and never apply a response themselves. Before D1, tests/e2e/mock.ts gains a hold-and-release for one request, and e2e gains: Browse request held, press t, Top renders, release Browse, the page still shows Top (rows, URL, header); a held Find cancelled, then released, changes nothing. human ~3 h / CC ~20 min.
History: none

### O2: normalization for the D15 binary comparison
Finding: O2, Medium, confidence 9/10, `core/src/queries/status.rs:106`, `core/src/snapshot/writer.rs:93-100`, reviewer Codex (verified by Claude).
Plan baseline: D15 accepted scope: "byte-identical text and `--json` answers ... on one fixture snapshot (scan-path PRs: fresh scans compared after dropping time fields)". Fixed in every column: the rest of D15 (characterization tests, test names, CI jobs).
Runtime evidence: `status` fills `fs[].total/used/avail` from a live `statvfs` of each root at query time, and ages from the clock; each scan writes a random 8-byte `snapshot_id`; so two runs a second apart can differ with identical code.
Comparison grid:
| Choice | Current | A Apply | B Keep | C Investigate | D Defer |
|---|---|---|---|---|---|
| What "identical" means | byte-identical | byte-identical after a written normalization: `--json` drops `snapshot_id`, every time field (`scanned_at`, `*_ts`, ages, `*_seconds`) and live `fs[].total/used/avail`; text masks the same values; both binaries run back to back on one copy of the snapshot dir | byte-identical (fails on unchanged code) | normalization found by running master against itself first | unresolved |
| Self-check | none | master vs master must pass the same normalized comparison before master vs PR counts | none | the master-vs-master run is the investigation | none |
| Effort | n/a | CC ~10 min once | none | CC ~10 min | none |
Question D20:
D20 — Записать, что именно нормализуется при сравнении старого и нового бинарника?
Project/branch/task: master, план docs/designs/code-quality-gates.md, debt PRs D3/D4, приёмка из D15.
ELI10: В D15 мы решили сравнивать вывод старого и нового бинарника байт в байт. Но status берёт свободное место с живого диска и возраст снапшота по часам, а каждый скан получает случайный id. Значит, даже один и тот же код может дать разный вывод с разницей в секунду. Нужно заранее записать, какие поля выкидываются или маскируются, и проверить сравнение на master против master.
Stakes if we pick wrong: сравнение падает на неизменном коде, и его начинают игнорировать, или его ослабляют на ходу так, что оно пропускает настоящую разницу.
Recommendation: A) Apply because без списка нормализации и проверки master против master приёмка из D15 не работает.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Apply (recommended)
  ✅ Список полей записан заранее: сравнение нельзя тихо ослабить посреди PR
  ✅ Прогон master против master доказывает, что сравнение не шумит на неизменном коде
  ❌ Маскированные поля (живое место на диске, возраст) этим сравнением не проверяются
B) Keep
  ✅ Строже на бумаге: байт в байт, без исключений
  ✅ Ничего не дописывать в план
  ❌ На деле падает на неизменном коде из-за statvfs, часов и случайного id
C) Investigate
  ✅ Список берётся из реального прогона master против master, а не из чтения кода
  ✅ Ничего лишнего не маскируется
  ❌ Тот же результат, что A, только позже и без записанного заранее списка
D) Defer
  ✅ Решение остаётся до первого Rust-PR с долгом
  ✅ Остальное в D15 не трогается
  ❌ Первый же PR упрётся в шумное сравнение
Net: записанная нормализация с самопроверкой против сравнения, которое шумит на неизменном коде.
Header: Diff normalize
Options:
A) Apply
The D15 comparison is byte-identical after a written normalization: --json drops snapshot_id, every time field (scanned_at, *_ts, ages, *_seconds) and live fs[].total/used/avail; text output masks the same values; both binaries run back to back on one copy of the snapshot dir. Master against master must pass the same comparison before master against the PR counts. CC ~10 min once.
B) Keep
D15 as approved: byte-identical without normalization.
C) Investigate
Run master against itself first and derive the normalization list from what differs.
D) Defer
Leave this proposal unresolved; D15 stays as approved.

State: approved
Actual answer: A) Apply (D20, answered 2026-10-23: "Apply (recommended)")
Accepted scope: The D15 comparison is byte-identical after a written normalization: --json drops snapshot_id, every time field (scanned_at, *_ts, ages, *_seconds) and live fs[].total/used/avail; text output masks the same values; both binaries run back to back on one copy of the snapshot dir. Master against master must pass the same comparison before master against the PR counts. CC ~10 min once.
History: D15 accepted scope refined by D20 (normalization and self-check).

### O3: one injection at a time in the canary
Finding: O3, Medium, confidence 9/10, `cli/Cargo.toml:18`, reviewer Codex (verified by Claude); refines PF2.
Plan baseline: D17 accepted scope: one violation per gate, each `scripts/check.sh <part>` must exit 1 naming the rule; PF2 note: clippy per member (`-p`). Fixed in every column: canary in CI (D17), temporary worktree, limit + 1 injections, check.sh parts (D8).
Runtime evidence: cli depends on core, so `cargo clippy -p freespace` checks core first; with core's injected function still present, core's `too_many_lines` diagnostic satisfies a "names the rule" check for the cli case. Clippy from a fresh worktree path: 9 s for both crates at `-j 2`.
Comparison grid:
| Choice | Current | A Apply | B Keep | C Investigate | D Defer |
|---|---|---|---|---|---|
| Injections per check run | all at once, `-p` per member | one: reset the worktree (`git checkout -- . && git clean -fdq`) before each injection | all at once | decide after a first canary run | unresolved |
| What a pass requires | exit 1 and the rule name | exit 1, the rule name and the injected file's path in the same diagnostic | exit 1 and the rule name | | |
| `-p` in check.sh | needed | not needed: each run carries one violation | needed | | |
| CI time | ~20 s (estimate) | about 9 runs; Rust ones re-check one crate each (estimate ~30-60 s total) | ~20 s | | |
Question D21:
D21 — В canary подкладывать по одному нарушению и сверять файл в сообщении?
Project/branch/task: master, план docs/designs/code-quality-gates.md, scripts/canary.sh из D17.
ELI10: Canary подкладывает нарушения и проверяет, что проверка падает и называет правило. Но cli зависит от core: если в core уже лежит подложенная длинная функция, проверка cli упадёт из-за core, и мы засчитаем успех, хотя для cli правило может быть выключено. Лечится просто: перед каждым нарушением возвращать worktree в чистое состояние и требовать, чтобы в сообщении был путь именно подложенного файла.
Stakes if we pick wrong: canary зелёный, а правило в cli выключено, то есть ровно тот случай, ради которого canary заводили.
Recommendation: A) Apply because иначе canary может засчитать падение core как доказательство для cli.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Apply (recommended)
  ✅ Каждая проверка доказывает ровно одно правило в ровно одном файле
  ✅ check.sh не нужен выбор крейта через -p: меньше логики в скрипте
  ❌ Больше прогонов: около 9 вместо 2-3, оценка 30-60 с в rust job
B) Keep
  ✅ Быстрее: все нарушения за один-два прогона
  ✅ Без сброса worktree между шагами
  ❌ Падение core может засчитаться за cli, и выключенное правило останется незамеченным
C) Investigate
  ✅ Решение по первому реальному прогону canary
  ✅ Можно увидеть, как clippy сообщает о зависимом крейте
  ❌ Дыра уже видна по Cargo.toml, прогон её только подтвердит
D) Defer
  ✅ D17 остаётся как одобрен
  ✅ Решение отдельно, позже
  ❌ Gates PR может уйти с canary, который не изолирует крейты
Net: полминуты CI против canary, который может засчитать чужое падение.
Header: Canary reset
Options:
A) Apply
scripts/canary.sh resets its worktree (git checkout -- . && git clean -fdq) before each injection, so each check run carries exactly one violation; a pass needs exit 1, the rule name and the injected file's path in the same diagnostic. check.sh needs no -p. About 9 runs; estimate 30-60 s in the rust job.
B) Keep
D17 as approved with PF2's per-member -p runs.
C) Investigate
Run the first canary draft and decide from its output.
D) Defer
Leave this proposal unresolved; D17 stays as approved.

State: approved
Actual answer: A) Apply (D21, answered 2026-10-23: "Apply (recommended)")
Accepted scope: scripts/canary.sh resets its worktree (git checkout -- . && git clean -fdq) before each injection, so each check run carries exactly one violation; a pass needs exit 1, the rule name and the injected file's path in the same diagnostic. check.sh needs no -p. About 9 runs; estimate 30-60 s in the rust job.
History: D17 accepted scope refined by D21; PF2's per-member -p note superseded.

### O4: fresh server per revision in the D16 pixel diff
Finding: O4, Medium, confidence 9/10, `web/playwright.config.ts:21`, reviewer Codex (verified by Claude).
Plan baseline: D16 accepted scope: a scratch Playwright run shoots every view and state before and after; zero differing pixels. Fixed in every column: the rest of D16.
Runtime evidence: `reuseExistingServer: !process.env.CI`: outside CI, a server already on port 4173 is reused, so the "after" run can serve the "before" bundle and match perfectly.
Comparison grid:
| Choice | Current | A Apply | B Keep | C Investigate | D Defer |
|---|---|---|---|---|---|
| Server per capture set | possibly reused | each revision's run sets `CI=1` (Playwright builds `dist/` and starts its own server, fails if 4173 is taken) | possibly reused | | unresolved |
| Revision record | none | each capture set saved with `git rev-parse HEAD` and the sha256 of `dist/main.js` | none | | |
| Effort | n/a | CC ~2 min | none | | |
Question D22:
D22 — Для попиксельного сравнения поднимать свежий сервер на каждую ревизию?
Project/branch/task: master, план docs/designs/code-quality-gates.md, разовое сравнение скриншотов из D16.
ELI10: Локально Playwright переиспользует уже запущенный сервер страницы. Если он остался от прогона «до», то прогон «после» снимет старую сборку, и сравнение покажет ноль отличий при сломанной вёрстке. Решение: запускать оба прогона с CI=1, чтобы Playwright каждый раз собирал и поднимал свой сервер, и записывать ревизию и хэш сборки рядом со снимками.
Stakes if we pick wrong: сравнение подтверждает распил CSS, которого на сервере не было.
Recommendation: A) Apply because это две минуты работы против ложного «ноль отличий».
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Apply (recommended)
  ✅ Каждый набор снимков сделан со своей свежей сборки, это видно по хэшу
  ✅ Занятый порт 4173 роняет прогон, а не подсовывает старый сервер
  ❌ Каждый прогон пересобирает страницу: плюс несколько секунд
B) Keep
  ✅ Ничего не меняется в D16
  ✅ Быстрее при повторных прогонах
  ❌ Старый сервер может дать идеальное совпадение со сломанной вёрсткой
C) Investigate
  ✅ Можно проверить, держится ли сервер между прогонами на этой машине
  ✅ Решение на фактах
  ❌ Риск виден по конфигу, проверка его только подтвердит
D) Defer
  ✅ D16 остаётся как одобрен
  ✅ Решение отдельно, позже
  ❌ Первое сравнение может пройти на старой сборке
Net: несколько секунд пересборки против ложного зелёного.
Header: Fresh server
Options:
A) Apply
Each revision's scratch run sets CI=1, so Playwright builds dist/ and starts its own server (and fails if port 4173 is taken); each capture set is saved with git rev-parse HEAD and the sha256 of dist/main.js. CC ~2 min.
B) Keep
D16 as approved.
C) Investigate
Check whether a server survives between runs on this machine and decide then.
D) Defer
Leave this proposal unresolved; D16 stays as approved.

State: moot after E2 (D23)
Actual answer: none; D22 reply (2026-10-23): "а нельзя всё такое тяжёлое и сложное оставить для ci на gh чтобы не перегружать машину? там бесплатный compute для oss, надо утилизировать"
Accepted scope: none; the pixel diff runs on a fresh GitHub runner (D23), where `CI` is set and Playwright never reuses a server, so the risk does not arise.
History: D22 reply reopened where the heavy one-time checks run (E2); closed as moot by D23.

### E2: where the heavy checks run (reopened by the user's reply to D22)
Finding: E2, P2, confidence 8/10, raised by the user on D22: "а нельзя всё такое тяжёлое и сложное оставить для ci на gh чтобы не перегружать машину? там бесплатный compute для oss, надо утилизировать". Reviewer: Claude.
Plan baseline: D15 and D16 comparisons are one-time scratch runs on this machine (two builds per revision, every view and state at 3 widths before and after, normalized binary diff); coverage was measured locally with cargo-llvm-cov for this review; D17/D21 canary and T4 tests already run in CI. Fixed in every column: D19 (one coordinator, out-of-order e2e), D20 (normalization and self-check), D21 (one injection at a time), the pre-push hook (global rule 3, 3-8 s warm), debt first (D14).
Runtime evidence: `gh repo view`: visibility PUBLIC, so GitHub-hosted runners are free without a minutes cap; this machine: 8 CPUs, load average 7.97 during this review's scans; the local pixel and coverage probes took about 5 min of builds (llvm-cov target 748 MB). On a runner, `CI` is set, so Playwright never reuses a server (`web/playwright.config.ts:21`) and D22's risk does not arise.
Comparison grid:
| Choice | Current | A GitHub runs them | B Local, as approved |
|---|---|---|---|
| Where the D15 binary diff runs | this machine, once per Rust debt PR | job `rust-diff` in a new `.github/workflows/refactor-guard.yml`: builds base (merge-base with master) and head, base vs base first, then base vs head, normalized per D20 | this machine, once per PR |
| Where the D16 pixel diff runs | this machine, once per web debt PR | job `pixel-diff`: base and head each built and served fresh on the runner, every view and state at 1280/700/360, fails on any differing pixel, uploads before/after/diff PNGs | this machine, with D22's fresh server (`CI=1`, revision and sha256 recorded) |
| Coverage for D15's characterization | measured once here | job `coverage`: cargo llvm-cov, lcov and per-function lines of touched files as an artifact, non-blocking | local llvm-cov when needed |
| When | n/a | on pull requests labeled `refactor` (every debt PR; later refactors reuse it) | one-time, debt PRs only |
| Local work | n/a | pre-push hook and the tests of the code being changed; full e2e, canary and these jobs on GitHub; CLAUDE.md says so | full comparisons locally |
| D22 | open | closed: satisfied by construction on the runner | A) Apply |
| Effort | n/a | human ~1 day / CC ~1.5 h (workflow and scripts) | CC ~1 h per comparison set, on this machine |
Question D23:
D23 — Отдать тяжёлые проверки GitHub Actions?
Project/branch/task: master, план docs/designs/code-quality-gates.md, сравнения из D15/D16 и покрытие (пересмотр по вашему ответу на D22).
ELI10: Сейчас план гоняет разовые тяжёлые сравнения на этой машине: два билда каждой ревизии, около 90 скриншотов, сравнение вывода старого и нового бинарника, покрытие через llvm-cov. Репо публичное, поэтому раннеры GitHub для него бесплатны и без лимита минут. Можно вынести всё это в отдельный workflow, который запускается на PR с меткой refactor: он соберёт master и PR, сравнит вывод и скриншоты и выложит картинки различий как артефакты. Локально тогда остаются только pre-push hook (3-8 с) и тесты того кода, который правится. Заодно пропадает проблема D22: на раннере сервер всегда свежий.
Stakes if we pick wrong: либо машина под нагрузкой 8 из 8 ядер ещё и собирает по две ревизии на каждый PR, либо мы пишем и поддерживаем workflow, который пригодится только на время распила.
Recommendation: A) GitHub runs them because компьют бесплатный, машина разгружается, а сравнения становятся постоянными для каждого будущего refactor-PR, а не разовыми.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) GitHub runs them (recommended)
  ✅ На этой машине остаются только hook и тесты правимого кода; сборки, скриншоты и покрытие уходят на раннеры
  ✅ Любой будущий PR с меткой refactor получает то же сравнение, а не только распил долга
  ❌ Ещё один workflow и скрипты сравнения: human ~1 day / CC ~1.5 h, ответ приходит через минуты после push
B) Local, as approved
  ✅ Без нового workflow: сравнения разовые и исчезают после распила
  ✅ Ответ сразу, без ожидания CI
  ❌ Две сборки на ревизию, 90 скриншотов и llvm-cov на машине, которая и так загружена
Net: постоянный бесплатный CI-инструмент ценой одного workflow, или разовые локальные прогоны на загруженной машине.
Header: Heavy checks
Options:
A) GitHub runs them
Add .github/workflows/refactor-guard.yml on pull requests labeled refactor (every debt PR carries it): rust-diff builds base (merge-base with master) and head, runs base vs base and then base vs head over status, ls, top, find, diff, doctor, text and --json, on one fixture snapshot, normalized per D20; pixel-diff builds and serves base and head fresh, shoots every view and state the e2e mock reaches at 1280, 700, 360 px, fails on any differing pixel and uploads before/after/diff PNGs; coverage runs cargo llvm-cov and uploads lcov plus per-function lines of touched files, non-blocking. Locally: the pre-push hook and the tests of the code being changed; full e2e, canary and these jobs run on GitHub; CLAUDE.md says so. Replaces D15's and D16's one-time local comparisons; D22 closes as satisfied by construction. human ~1 day / CC ~1.5 h.
B) Local, as approved
D15 and D16 comparisons stay one-time runs on this machine; D22 takes its A) Apply (CI=1 fresh server, revision and sha256 recorded).

State: approved
Actual answer: A) GitHub runs them (D23, answered 2026-10-23: "GitHub runs them (recommended)")
Accepted scope: Add .github/workflows/refactor-guard.yml on pull requests labeled refactor (every debt PR carries it): rust-diff builds base (merge-base with master) and head, runs base vs base and then base vs head over status, ls, top, find, diff, doctor, text and --json, on one fixture snapshot, normalized per D20; pixel-diff builds and serves base and head fresh, shoots every view and state the e2e mock reaches at 1280, 700, 360 px, fails on any differing pixel and uploads before/after/diff PNGs; coverage runs cargo llvm-cov and uploads lcov plus per-function lines of touched files, non-blocking. Locally: the pre-push hook and the tests of the code being changed; full e2e, canary and these jobs run on GitHub; CLAUDE.md says so. Replaces D15's and D16's one-time local comparisons; D22 closes as satisfied by construction. human ~1 day / CC ~1.5 h.
History: supersedes the "one-time" and "scratch, not committed" parts of D15/D20 and D16; their behavior lists, normalization and acceptance stay.

Outside voice dispositions: O1 accepted (D19, one request coordinator and out-of-order e2e); O2 accepted (D20, written normalization, master vs master first); O3 accepted (D21, one injection per run, file path in the diagnostic); O4 moot (D22 reply led to E2); E2 accepted (D23, heavy checks in a `refactor-guard` workflow on GitHub). Remaining disagreement with Codex: none.

### TODO candidates

### TD1: speed check for scan-path PRs (upgrade path of D18)
Finding: TD1, P3, confidence 7/10, from PF3/D18, reviewer Claude.
Plan baseline: D18 review only (per-entry clone/collect/allocation list in the PR body; no timing). Fixed in every column: D18 for the debt PRs; refactor-guard workflow (D23).
Runtime evidence: warm `/usr` scans on this VM 8.0 to 3.8 s over 10 runs, CPU time ±12%, no hardware counters; GitHub runner noise not measured.
Comparison grid:
| Choice | A Add to TODOS.md | B Skip | C Build it now |
|---|---|---|---|
| Result | TODOS.md item: interleaved CPU-time comparison of base and head release binaries (7 warm scans each), 15% bar, per-phase seconds; trigger: a scan-path PR whose per-phase seconds move visibly, or a report of slower scans | nothing recorded beyond the decision log entry | reopens D18 for the debt PRs |
Question D24:
D24 — Записать в TODOS.md проверку скорости скана на будущее?
Project/branch/task: master, план docs/designs/code-quality-gates.md, TODO из D18.
ELI10: В D18 вы выбрали только ревью кода без замеров скорости. Это нормально для распила, но если позже скан станет медленнее без явного копирования, никто не заметит. Можно записать в TODOS.md готовый рецепт замера (по 7 тёплых сканов старого и нового бинарника, порог 15%) и условие, когда его включать.
Stakes if we pick wrong: либо в TODOS.md лишняя строка, либо через полгода рецепт и повод придумывают заново.
Recommendation: A) Add to TODOS.md because решение D18 остаётся как есть, а условие для пересмотра записано там, где его найдут.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Add to TODOS.md (recommended)
  ✅ Рецепт и условие включения лежат рядом с остальными отложенными задачами
  ✅ D18 для распила не меняется
  ❌ Ещё одна строка в TODOS.md, которую надо держать в актуальном виде
B) Skip
  ✅ TODOS.md не растёт; решение уже есть в decision log
  ✅ Ничего не нужно поддерживать
  ❌ Через полгода рецепт и повод придумывают заново
C) Build it now
  ✅ Замер с первого же PR с долгом
  ✅ Цифры по фазам в каждом PR
  ❌ Отменяет ваш ответ на D18 без нового довода
Net: одна строка в TODOS.md против забытого рецепта.
Header: TODO speed
Options:
A) Add to TODOS.md
Add under a new "Deferred from /plan-eng-review 2026-10-23 (code quality gates)" section: interleaved CPU-time comparison for scan-path PRs, trigger and recipe as above, Effort S, Priority P3, depends on refactor-guard (D23).
B) Skip
No TODO; the decision log keeps D18's upgrade trigger.
C) Build it now
Add the interleaved CPU-time comparison to the debt PRs now (reopens D18).

State: approved
Actual answer: A) Add to TODOS.md (D24, answered 2026-10-23: "Add to TODOS.md (recommended)")
Accepted scope: Add under a new "Deferred from /plan-eng-review 2026-10-23 (code quality gates)" section: interleaved CPU-time comparison for scan-path PRs, trigger and recipe as above, Effort S, Priority P3, depends on refactor-guard (D23).
History: none

### TD2: root-only branches under sudo in CI
Finding: TD2, P2, confidence 8/10, `core/src/refresh.rs:402` (mount point refused), `core/src/scan/walk.rs:392` (child on another device becomes Kind::Mount), reviewer Claude.
Plan baseline: D15 characterizes the unreached branches that run without root; root-only ones stay unreached. D3 splits `refresh_locked` and `read_dir`, which hold both. Fixed in every column: D15, D20, D23.
Runtime evidence: llvm-cov as non-root: `refresh.rs:402-406` and `walk.rs:392-395` unreached; CI already runs `sudo scripts/fault-fs.sh`, which mounts a tmpfs (`scripts/fault-fs.sh:32`).
Comparison grid:
| Choice | A Add to TODOS.md | B Skip | C Build it now |
|---|---|---|---|
| Result | TODOS.md item: a sudo script mounts a tmpfs inside a fixture root; scan shows it as a mount with 0 bytes; refresh of the mount point exits with the usage error and its text | the two branches stay untested through the split | the same script lands with D15's characterization tests, before D3, run in the rust job next to fault-fs.sh |
| Effort | later | none | human ~2 h / CC ~15 min |
Question D25:
D25 — Покрыть root-only ветки (точка монтирования) тестом под sudo в CI?
Project/branch/task: master, план docs/designs/code-quality-gates.md, D15 и распил refresh_locked / read_dir в D3.
ELI10: Две ветки нельзя проверить без root: скан, встречающий папку на другом диске (она становится точкой монтирования с нулём байт), и отказ refresh для такой точки. В D15 тесты пишутся только на то, что работает без root, поэтому эти ветки остаются непроверенными, хотя D3 как раз распиливает функции, где они живут. В CI есть sudo, и fault-fs.sh уже монтирует tmpfs: такой же маленький скрипт закроет обе ветки.
Stakes if we pick wrong: распил тихо ломает обработку точек монтирования, а это то, что видит каждый пользователь с отдельным /home или /boot.
Recommendation: C) Build it now because ветки распиливаются в D3, а инструмент (sudo и tmpfs в CI) уже есть.
Note: options differ in kind, not coverage — no completeness score.
Pros / cons:
A) Add to TODOS.md
  ✅ Распил не задерживается, задача записана
  ✅ Можно сделать позже вместе с другими root-тестами
  ❌ Распил D3 пройдёт мимо этих веток без защиты
B) Skip
  ✅ Ничего не добавлять
  ✅ Ветки короткие: сообщение об ошибке и обнуление размера
  ❌ Ошибка в обработке монтирований всплывёт только у пользователя
C) Build it now (recommended)
  ✅ Обе ветки под тестом до того, как D3 их распилит
  ✅ Тот же приём, что у fault-fs.sh: sudo, tmpfs, rust job
  ❌ Ещё один sudo-скрипт в CI: human ~2 h / CC ~15 min
Net: пятнадцать минут сейчас против непроверенных точек монтирования во время распила.
Header: TODO mounts
Options:
A) Add to TODOS.md
Add under "Deferred from /plan-eng-review 2026-10-23 (code quality gates)": a sudo CI script for the mount-point branches, Effort S, Priority P2.
B) Skip
Leave both root-only branches untested.
C) Build it now
With D15's characterization tests and before D3, add a sudo script (like scripts/fault-fs.sh) run in the rust job: mount a tmpfs inside a fixture root, scan (the mount shows as a mount with 0 bytes), refresh the mount point (usage error with its text). human ~2 h / CC ~15 min.

State: approved
Actual answer: C) Build it now (D25, answered 2026-10-23: "Build it now (recommended)")
Accepted scope: With D15's characterization tests and before D3, add a sudo script (like scripts/fault-fs.sh) run in the rust job: mount a tmpfs inside a fixture root, scan (the mount shows as a mount with 0 bytes), refresh the mount point (usage error with its text). human ~2 h / CC ~15 min.
History: none

### L1: Biome rule set (late finding, during PR 1)
Finding: L1, P2, confidence 8/10, `~/.claude/CLAUDE.md` rule 1 ("Linters and formatters at strict settings") and `web/biome.json` (PR 1), reviewer Claude.
Plan baseline: D2/D4/D6 approved two Biome rules (length 80, cognitive complexity 25) and said nothing about the rest; PR 1 writes `"preset": "none"` so only those two run. Rust runs clippy's default set plus two extra lints, so the web side is weaker than the Rust side. Fixed in every column: D2, D4, D5, D6, D14 (zero exceptions at gate time).
Runtime evidence: Biome 2.6.2 after the width-140 format: preset none gives the 6 planned diagnostics (5 functions); preset recommended adds 87: noNonNullAssertion 57 (app.ts 23, tests/unit/contrast.test.ts 16, e2e 16, help.ts 2), noDescendingSpecificity 15 (app.css 14, responsive.css 1), noControlCharactersInRegex 4 (states.ts, format.ts), useTemplate 4, useOptionalChain 3, useIterableCallbackReturn 1, useMathMinMax 1, noUselessEscapeInRegex 1, noImportantStyles 1. The `recommended: false` field is deprecated in 2.6 (use `preset`).
Comparison grid:
| Choice | A Recommended preset | B Recommended without noNonNullAssertion | C Two rules only |
|---|---|---|---|
| biome.json | `"preset": "recommended"` plus the two limits | same, `noNonNullAssertion: "off"` | `"preset": "none"` plus the two limits |
| Debt before the gates | 87 more findings in the web lane (W2), most in files D1/D2 already rewrite | 30 more findings | none |
| Effort | human ~1 day / CC ~1 h | human ~4 h / CC ~30 min | 0 |
Question D27:
D27 — Включаем стандартный набор правил Biome (recommended) или только два лимита?
Project/branch/task: chore/biome-format, PR 1 плана code-quality-gates; задача W-лейна.
ELI10: Глобальное правило требует линтеры «at strict settings». В Rust так и есть: clippy работает со своим стандартным набором плюс двумя нашими лимитами. Для веба в плане только два правила Biome, длина и сложность функций. Стандартный набор Biome на нашем коде даёт ещё 87 замечаний. Две трети из них — это `x!` («здесь точно не null»), такие места обходят строгую проверку null в TypeScript. Остальное — порядок селекторов в CSS и мелочи вроде шаблонных строк.
Stakes if we pick wrong: либо веб проверяется слабее, чем требует глобальное правило, либо в W-лейн добавляется день ручной работы.
Recommendation: A) Recommended preset because так веб получает тот же уровень, что и Rust, а 23 из 57 `!` и 14 из 15 CSS-замечаний сидят в app.ts и app.css, которые D1 и D2 и так переписывают.
Completeness: A=10/10, B=8/10, C=5/10
Pros / cons:
A) Recommended preset (recommended)
  ✅ Веб на том же уровне, что Rust: стандартный набор линтера плюс два лимита
  ✅ Большая часть замечаний в app.ts и app.css, которые распил D1/D2 всё равно переписывает
  ❌ Ещё 87 исправлений в W-лейне до PR с гейтами: human ~1 day / CC ~1 h
B) Recommended без noNonNullAssertion
  ✅ Остаётся 30 замечаний, в основном CSS и мелочи
  ✅ `x!` в тестах (contrast.test.ts, 16 штук) остаются как есть
  ❌ `x!` в коде продолжает обходить строгую проверку null без всякого сигнала
C) Только два лимита
  ✅ Как утверждено в ревью, ничего добавлять не нужно
  ✅ Ноль дополнительной работы до PR с гейтами
  ❌ Веб проверяется заметно слабее Rust, вразрез с глобальным правилом 1
Net: день работы в W-лейне против линтера для веба, который слабее, чем того требует глобальное правило.
Header: Biome rules
Options:
A) Recommended preset
`"preset": "recommended"` in web/biome.json (set in the gates PR); new task W2 in the web lane fixes the 87 findings without suppressions, after D1/D2 so their rewrites go first; refactor-guard applies. human ~1 day / CC ~1 h.
B) Recommended without noNonNullAssertion
Same with `noNonNullAssertion: "off"`; W2 fixes the other 30. human ~4 h / CC ~30 min.
C) Two rules only
Keep `"preset": "none"` with the two limits, as reviewed.

State: approved
Actual answer: A) Recommended preset (D27, answered 2026-10-23: "Recommended preset (recommended)")
Accepted scope: `"preset": "recommended"` in web/biome.json (set in the gates PR); new task W2 in the web lane fixes the 87 findings without suppressions, after D1/D2 so their rewrites go first; refactor-guard applies. human ~1 day / CC ~1 h.
History: none

TODO dispositions: TD1 accepted as A (D24, added to `TODOS.md` under "Deferred from /plan-eng-review 2026-10-23 (code quality gates)"); TD2 accepted as C (D25, the mount-point sudo script joins the characterization step of D15, before D3).

Approval readiness: PASS. Checked: S1 (D2 "Biome"), S2a (D3 "Nesting 4"), S2b (D4 "Cognitive 25"), S3 (D5 "Format, width 140"), S5 (D6 "Same rules"), S4 (D7 "Nesting instead"), A1 (D8 "One entrypoint"), A2 (D9 "Fail with hint"), C1 (D10 "Code only"), C2 (D11 "Whole file"), C3 (D12 "500 like code"), C4 (moot: D13 reply opened E1), E1 (D14 "Debt first"), T1 (D15 "Characterize first (recommended)", refined by D20 and D23), T2 (D16 "Close gaps + pixel diff (recommended)", refined by D23), T3 (D17 "Canary in CI (recommended)", refined by D21), PF3 (D18 "Review only"), O1 (D19 "Apply (recommended)"), O2 (D20 "Apply (recommended)"), O3 (D21 "Apply (recommended)"), O4 (moot: D22 reply opened E2), E2 (D23 "GitHub runs them (recommended)"), TD1 (D24 "Add to TODOS.md (recommended)"), TD2 (D25 "Build it now (recommended)"), L1 (D27 "Recommended preset (recommended)", late finding during PR 1; re-checked after it). Scope structure: D1 "Original arrangement". Accepted without a question as required by approved decisions or global rules: A3, A4 (global rules 1 and 3), C5 (G1 contract), T3's `[lints] workspace = true` (G2), T4 (proof of D8-D12, D14, C5), T5 (documented limit), PF1, PF2. Pending: none.

## NOT in scope

- **Coverage thresholds.** A non-goal; the `coverage` job of `refactor-guard` uploads numbers and never fails a PR (D23).
- **Mutation testing.** A non-goal; the canary proves the gates fire, the characterization tests guard behavior.
- **Timed speed check for the scan path.** D18 chose review only for the debt PRs; the timed recipe and its trigger are in `TODOS.md` (D24).
- **Allowlist and exception machinery.** The gates land with zero exceptions (D14); an allowlist file appears only after the user agrees to a first exception.
- **Tests, e2e and the canary in the pre-push hook.** They run on GitHub's free runners, not on the dev box (D23); the hook runs only `scripts/check.sh`.
- **A hook that checks the pushed commits.** Stashing risks the user's edits and a checkout of the pushed commit is a cold build; CI is the gate (T5).
- **Changed-crate logic in the hook.** The whole workspace takes 3-8 s warm (PF1).
- **typescript-eslint.** Its peer range stops below TypeScript 6.1 and the repo is on 7 (D2).
- **`clippy::cognitive_complexity`.** A restriction lint its own docs call misleading; nesting 4 instead (D3), and the global rule now says so (D7).
- **Splitting test files under 1000.** `core/tests/queries.rs` (931) stays as it is.
- **A style rewrite beyond the Biome format.** A non-goal.

## What already exists

| Existing | Where | Reused or rebuilt |
|---|---|---|
| sudo + tmpfs test script with `trap` cleanup | `scripts/fault-fs.sh:15-18,32`, run in the rust job | reused as the pattern for the mount-point script (D25) |
| failure path proven as a CI job | `no-bun` job, `.github/workflows/ci.yml:84-95` | the canary follows the same idea inside the rust job (D17) |
| mock API for e2e | `web/tests/e2e/mock.ts` | extended with hold-and-release (D19); `pixel-diff` drives it to reach every state |
| Playwright shots | `web/tests/shots/readme.spec.ts` | kept, with `animations: "disabled"` (D16) |
| one request coordinator | `load()`, `web/src/app.ts:524-546` (`loadSeq`, `AbortController`) | kept as the only place a response is applied after the App split (D19) |
| tests in their own `tests.rs` | `cli/src/serve/tests.rs` | the model for moving the walk.rs and tree.rs tests (D11, D4) |
| equivalence test style | `split_passes_answer_like_one_pass` | reused where a split touches query logic |
| rustfmt, clippy, tsc, shellcheck in CI | `.github/workflows/ci.yml:24,25,44,109` | reused; every job calls them through `scripts/check.sh` (D8) |

Nothing is rebuilt. New code: `scripts/check-size.sh`, `scripts/check.sh`, `scripts/test-check.sh`, `scripts/canary.sh`, the mount-point script, `.githooks/pre-push`, `web/biome.json`, `clippy.toml`, `.github/workflows/refactor-guard.yml`. No shared-code choice was put to a question; the shared entrypoint is D8.

## Diagrams

Order of the work (D14, D23):

```
 PR 1 format (web/)  ──►  PR 2 refactor-guard (.github/, web/tests/)
                                     │
                   ┌─────────────────┴──────────────────┐
         Rust lane (core/, cli/)                 Web lane (web/)
         D0 tests + mount script                 W0 e2e gaps + mock hold
           │   (green on unsplit code)             │   (green on unsplit code)
         D3 13 functions ≤ 80                    D1 App split, one coordinator
         D4 5 files ≤ 500                        D2 app.css split
                   │                             W1 inspector, h, mock handle
                   │                             W2 Biome recommended findings
                   └─────────────────┬──────────────────┘
                                     ▼
              last PR: gates on, zero exceptions (G1-G8)
```

Checks after the gates PR (D8, D9, D17, D23):

```
 git push ─► .githooks/pre-push ─► scripts/check.sh (all parts, 3-8 s warm)
                                     rust  : cargo fmt --check; clippy -D warnings
                                             (too_many_lines 80, excessive_nesting 4)
                                     size  : check-size.sh (500 / 1000)
                                     shell : shellcheck install.sh scripts/*.sh .githooks/pre-push
                                     tsc   : tsc --noEmit
                                     biome : biome ci --error-on-warnings (80, 25, width 140)
                                   tool missing ─► exit 1, name + install command
                                   unknown part ─► exit 2, list of parts

 GitHub ci.yml   rust job    : check.sh rust ─► tests ─► sudo fault-fs.sh, mount script ─► canary.sh
                 quality job : check.sh size biome ─► test-check.sh
                 shellcheck  : check.sh shell
                 web job     : check.sh tsc ─► unit, e2e

 GitHub refactor-guard.yml (PR label `refactor`)
                 rust-diff  : build base + head ─► base vs base (must match) ─► base vs head, normalized (D20)
                 pixel-diff : serve base, serve head fresh ─► shoot all states x 1280/700/360 ─► 0 px or fail + PNGs
                 coverage   : llvm-cov ─► lcov + per-function lines (never fails)
```

Canary loop (D17, D21):

```
 worktree of HEAD (temp) ─► for each gate:
     git checkout -- . && git clean -fdq
     inject one violation at limit + 1
     scripts/check.sh <part> ─► exit 1 AND rule name AND injected path in one diagnostic? ─ no ─► canary fails
 remove worktree
```

Inline diagrams in code: the request coordinator that D1 leaves (sequence of `loadSeq`, abort and apply, as in the e2e of D19) and the header of `scripts/canary.sh` (the loop above).

## Failure modes

| New path | Realistic failure | Test or handling | What the user sees |
|---|---|---|---|
| check-size.sh | scope filter breaks and matches no file | exits 1 on zero files; test-check.sh row | CI red with the reason |
| check-size.sh | run outside a git work tree (tarball, CI checkout without `.git`) | exits 1; test-check.sh row | red with the reason |
| clippy lints | a new crate without `[lints] workspace = true` | canary injects into every member from `cargo metadata` | canary red, names the crate's file |
| clippy / Biome thresholds | someone raises 80 to 120 | canary injects at limit + 1 of the documented value | canary red |
| check.sh | a typo in a part name in `ci.yml` | exits 2 and lists parts; test-check.sh row | CI red |
| check.sh | a tool missing on the runner or dev box | exits 1 with its install command (D9) | red with the fix |
| pre-push | fresh clone without the hook installed | CI runs the same parts | PR red, not the push |
| pre-push | uncommitted edits hide a violation | documented (T5); CI runs on the commits | PR red |
| refactor-guard | a debt PR opened without the `refactor` label | partial: the characterization tests and e2e still run in `ci.yml`; the byte and pixel diffs do not | green PR with fewer checks; CLAUDE.md requires the label |
| rust-diff | normalization masks a field the split broke (a time value) | partial: `core/tests/queries.rs:279,554,840` assert `newest_mtime` against `scanned_at` | wrong age in output, not caught by the diff |
| rust-diff | the fixture or the runner makes output unstable | base vs base must match first (D20) | job red as unreliable, not a false pass |
| pixel-diff | fonts or antialiasing differ between two builds | fails loudly and uploads diff PNGs | red, with pictures |
| mount script | a failed run leaves the tmpfs mounted | `trap` cleanup like `fault-fs.sh:18`; ephemeral runner | nothing |
| App split | a late Browse response overwrites Top | out-of-order e2e with hold-and-release (D19) | e2e red |

Critical gaps (no test, no handling, silent): none. The two partial rows keep a test layer underneath; the remaining unreached lines after D0 (`install::run`, writer disk full) are covered by the install job and `fault-fs.sh`.

## Worktree parallelization strategy

| Step | Modules touched | Depends on |
|------|----------------|------------|
| Format and shots (F1, F2) | web/ | — |
| refactor-guard (R1) | .github/workflows/, web/tests/ | F1, F2 |
| Rust characterization and mount script (D0) | core/, cli/, scripts/, .github/workflows/ | R1 |
| Rust splits (D3, D4) | core/, cli/ | D0 |
| Web e2e gaps and mock hold (W0) | web/tests/ | R1 |
| Web splits and lint debt (D1, D2, W1, W2) | web/src/, web/tests/ | W0 |
| Gates (G1-G8) | scripts/, .githooks/, core/, cli/ (Cargo.toml), web/ (config), .github/workflows/, CLAUDE.md, CONTRIBUTING.md | all of the above |

**Parallel lanes:**
- Lane A: D0 → D3 → D4 (shared core/, cli/; D3 and D4 touch the same files, so sequential inside the lane)
- Lane B: W0 → D1 → D2 → W1 → W2 (shared web/)

**Execution order:** F1+F2, merge. R1, merge. Launch A + B in parallel worktrees. Merge both. Then the gates PR.

**Conflict flags:** D0 adds the mount script to the rust job in `.github/workflows/ci.yml`; Lane B does not touch `ci.yml`, so no cross-lane conflict. The gates PR edits `ci.yml` after both lanes have merged.

## Implementation Tasks
Synthesized from this review's findings. Each task derives from a specific
finding above. Run with Claude Code or Codex; checkbox as you ship.
Effort assumption: scaffolding and config ~20x, tests ~20x, refactors behind a differential guard ~8x (human ÷ CC).

- [x] **T1 (P1, human: ~2h / CC: ~10min)** — web — Add Biome 2.6.2, format once at width 140, shots with animations disabled
  - Surfaced by: Scope Challenge — S1 (D2), S3 (D5); Section 3 — T2 (D16, header bar `transition: width 400ms`)
  - Files: web/package.json, web/bun.lock, web/biome.json, web/src/, web/tests/, web/tests/shots/readme.spec.ts, docs/img/
  - Verify: `cd web && bunx biome format .` reports nothing; `bun run shots` twice gives identical PNGs
- [ ] **T2 (P1, human: ~1 day / CC: ~1.5h)** — CI — Add refactor-guard workflow (rust-diff, pixel-diff, coverage)
  - Surfaced by: Outside voice — E2 (D23), O2 (D20); Section 3 — T1 (D15), T2 (D16)
  - Files: .github/workflows/refactor-guard.yml, web/tests/ (state-shooting spec), scripts/ (diff helper)
  - Verify: a no-op PR labeled `refactor` is green in all three jobs; a PR that changes one output byte fails rust-diff
- [ ] **T3 (P1, human: ~2 days / CC: ~1.5h)** — core, cli — Characterization tests for unreached branches
  - Surfaced by: Section 3 — T1 (D15): status.rs:129, query.rs, doctor.rs:259, refresh.rs:270,445,455, diff.rs:180,254, job.rs:155
  - Files: cli/tests/cli.rs, core/tests/queries.rs, core/tests/publish.rs
  - Verify: `cargo test --workspace` green on master before any split; `cargo llvm-cov` shows those lines reached
- [ ] **T4 (P1, human: ~2h / CC: ~15min)** — scripts, CI — Mount-point test under sudo
  - Surfaced by: TODO candidates — TD2 (D25): refresh.rs:402, walk.rs:392
  - Files: scripts/ (new script), .github/workflows/ci.yml
  - Verify: `sudo scripts/<script>.sh` passes locally; rust job green
- [ ] **T5 (P1, human: ~6h / CC: ~40min)** — web tests — e2e for the six keys, Show more, cancel; mock hold-and-release; out-of-order e2e
  - Surfaced by: Section 3 — T2 (D16); Outside voice — O1 (D19)
  - Files: web/tests/e2e/mock.ts, web/tests/e2e/browse.spec.ts, web/tests/e2e/find.spec.ts
  - Verify: `cd web && bunx playwright test` green on the unsplit App
- [ ] **T6 (P1, human: ~3 days / CC: ~3h)** — core, cli — Split the 13 Rust functions over 80
  - Surfaced by: Problem / Current violations; Section 4 — PF3 (D18, allocation list in the PR body)
  - Files: core/src/snapshot/writer.rs, core/src/refresh.rs, core/src/queries/diff.rs, core/src/queries/status.rs, core/src/scan/tree.rs, core/src/scan/walk.rs, core/src/job.rs, cli/src/install.rs, cli/src/status.rs, cli/src/doctor.rs
  - Verify: `cargo clippy --workspace --all-targets -- -W clippy::too_many_lines` with a scratch `clippy.toml` at threshold 80 reports none; refactor-guard green
- [ ] **T7 (P1, human: ~1 day / CC: ~1h)** — core, cli — Bring the 5 Rust files under 500
  - Surfaced by: Section 2 — C2 (D11)
  - Files: core/src/refresh.rs, cli/src/install.rs, cli/src/query.rs, core/src/scan/walk.rs, core/src/scan/tree.rs (+ tests.rs submodules)
  - Verify: `wc -l` under 500; `cargo test -- --list` same names before and after
- [ ] **T8 (P1, human: ~3 days / CC: ~4h)** — web — Split App by responsibility, one request coordinator
  - Surfaced by: Problem; Outside voice — O1 (D19); Scope Challenge — S1 (App.notices), S2b (renderSummary)
  - Files: web/src/app.ts, web/src/ (new modules)
  - Verify: e2e green incl. out-of-order tests; refactor-guard pixel-diff 0 px
- [ ] **T9 (P1, human: ~2h / CC: ~15min)** — web — Split app.css at its section comments
  - Surfaced by: Section 2 — C3 (D12)
  - Files: web/src/app.css, web/src/main.ts
  - Verify: refactor-guard pixel-diff 0 px
- [ ] **T10 (P1, human: ~4h / CC: ~30min)** — web — inspector, h and the mock's handle under the Biome limits
  - Surfaced by: Scope Challenge — S2b (D4), S5 (D6)
  - Files: web/src/inspector.ts, web/src/dom.ts, web/tests/e2e/mock.ts
  - Verify: `bunx biome lint --error-on-warnings` clean; e2e green
- [ ] **T19 (P1, human: ~1 day / CC: ~1h)** — web — Fix the Biome recommended-preset findings without suppressions
  - Surfaced by: Late finding L1 (D27), global rule 1 "strict settings"
  - Files: web/src/, web/tests/ (after T8, T9)
  - Verify: `bunx biome lint .` with `"preset": "recommended"` reports none; e2e green; refactor-guard pixel-diff 0 px
- [ ] **T11 (P1, human: ~3h / CC: ~15min)** — scripts — check-size.sh
  - Surfaced by: Section 2 — C1 (D10), C2 (D11), C3 (D12), C5
  - Files: scripts/check-size.sh
  - Verify: `scripts/test-check.sh` rows for check-size.sh
- [ ] **T12 (P1, human: ~1.5h / CC: ~15min)** — scripts — check.sh with five parts and tool hints
  - Surfaced by: Section 1 — A1 (D8), A2 (D9)
  - Files: scripts/check.sh
  - Verify: `scripts/test-check.sh` rows for check.sh
- [ ] **T13 (P1, human: ~30min / CC: ~5min)** — config — Rust lints and Biome lint on
  - Surfaced by: Scope Challenge — S2a (D3); Section 3 — T3 (`[lints] workspace = true`)
  - Files: Cargo.toml, clippy.toml, core/Cargo.toml, cli/Cargo.toml, web/biome.json (`"preset": "recommended"`, D27)
  - Verify: `scripts/check.sh` clean on master; canary red for each injection
- [ ] **T14 (P1, human: ~1h / CC: ~10min)** — hook, CI — pre-push hook and every CI job through check.sh
  - Surfaced by: Section 1 — A1 (D8), A3, A4
  - Files: .githooks/pre-push, .github/workflows/ci.yml
  - Verify: `git config core.hooksPath .githooks` then a push with an 81-line function is blocked
- [ ] **T15 (P1, human: ~4h / CC: ~20min)** — scripts — test-check.sh
  - Surfaced by: Section 3 — T4
  - Files: scripts/test-check.sh, .github/workflows/ci.yml (quality job)
  - Verify: quality job green; each row fails when its rule is removed from the script
- [ ] **T16 (P1, human: ~4h / CC: ~30min)** — scripts — canary.sh, one injection per run
  - Surfaced by: Section 3 — T3 (D17); Outside voice — O3 (D21); Section 4 — PF2
  - Files: scripts/canary.sh, .github/workflows/ci.yml (rust job)
  - Verify: rust job green; raising a threshold makes it red
- [ ] **T17 (P2, human: ~1h / CC: ~10min)** — docs — CLAUDE.md and CONTRIBUTING.md
  - Surfaced by: Section 1 — A4; Section 3 — T5; Outside voice — E2 (D23)
  - Files: CLAUDE.md, CONTRIBUTING.md
  - Verify: both name the limits, `scripts/check.sh`, the hook command, the `refactor` label and what runs where
- [ ] **T18 (P3, human: ~4h / CC: ~30min)** — CI — Timed speed check for scan-path PRs (in TODOS.md)
  - Surfaced by: Section 4 — PF3 (D18); TODO candidates — TD1 (D24)
  - Files: .github/workflows/refactor-guard.yml
  - Verify: base vs base under the 15% bar; trigger written in TODOS.md

## Unresolved decisions

None. Every question D1-D27 has an actual answer or is recorded as moot (C4, O4) with the reply that closed it.

## Completion summary

- Step 0: Scope Challenge — scope accepted as-is
- Reality rows: 7/7 applicable rows emitted (`PLAN_REALITY` line), 1 unverified premise (hook clippy time; Section 4 then measured 1-5 s)
- Architecture Review: 4 issues found (A1-A4)
- Code Quality Review: 6 issues found (C1-C5; E1 reopened by the user's question on D13; L1, the Biome rule set, found late while building PR 1 and answered by D27)
- Test Review: diagram produced, 27 gaps identified (22 in new code, 5 in existing behavior; findings T1-T6)
- Performance Review: 3 issues found (PF1-PF3)
- NOT in scope: written
- What already exists: written
- TODOS.md updates: 2 items proposed to user (TD1 added to TODOS.md; TD2 built now)
- Failure modes: 0 critical gaps flagged
- Unresolved decisions: 0 in this review
- Outside voice: Codex (gpt-6-astra), completed, 4 findings (O1-O3 accepted, O4 moot after E2)
- Parallelization: 2 lanes, 2 parallel / 3 sequential (format, refactor-guard, gates)
- Lake Score: 9/11 = answers picking a 10/10 option (D2, D5, D6, D7, D8, D9, D15, D17, D27) / answers scored for Completeness (those plus D16 at 9/10 and D18 at 6/10)

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `/plan-ceo-review` | Scope & strategy | 0 | — | not run for this plan |
| Outside Review | Codex (gpt-6-astra), plan-review phase of `/plan-eng-review` | Independent 2nd opinion | 1 | completed (issues_found) | 4 findings: 3 accepted (D19-D21), 1 moot (D23) |
| Eng Review | `/plan-eng-review` | Architecture & tests (required) | 1 | ISSUES OPEN (PLAN), this run; logged after this report is saved | 40 issues, 0 critical gaps; all mapped to tasks T1-T19 |
| Design Review | `/plan-design-review` | UI/UX gaps | 0 | — | not run for this plan |
| DX Review | `/plan-devex-review` | Developer experience gaps | 1 | CLEAR | score: 4/10 → 8/10, TTHW: 4-14 min → 2-5 min (for `free-space-disk-analyzer.md`, commit af5b9ef, 11 commits ago; not this plan) |

- **OUTSIDE COVERAGE:** Codex (gpt-6-astra), plan-review phase, completed at commit 41f5440, 4 findings; no native fallback used.
- **CROSS-MODEL:** Claude (this review) and Codex (outside voice). Codex found the request-ordering risk in the App split (O1), the missing normalization for the binary diff (O2), stacked injections in the canary (O3) and server reuse in the pixel diff (O4); none overlapped Claude's findings, and Claude verified all four in code before they were put to the user.
- **VERDICT:** none CLEARED for this plan. This eng run is issues_open because its 40 findings are mapped work (T1-T19), not open questions; eng review required.
NO UNRESOLVED DECISIONS
