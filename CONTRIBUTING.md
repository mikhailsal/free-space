# Contributing

Bug reports and pull requests are welcome. For a bug, please use the [issue template](.github/ISSUE_TEMPLATE/bug.md): it asks for `freespace --version` and `freespace doctor --json`, which answer most first questions.

## Layout

- `core/`: the scanner, the snapshot (SQLite), queries, the error contract, config and systemd units. No CLI or HTTP code.
- `cli/`: the `freespace` binary: commands, table output, `serve`/`gui` (axum), `install-timer`, `doctor`. `cli/build.rs` bundles `web/` with bun and embeds it.
- `web/`: the browser page, vanilla TypeScript, no framework.
- `skill/`: `SKILL.md` and `schema.json` (generated, see below).
- `docs/designs/free-space-disk-analyzer.md`: the design and its decisions. Read the part you are changing first.

## Build and test

You need Rust 1.88 or newer and [bun](https://bun.sh) 1.4 or newer.

```sh
cargo build
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

The web page has its own checks (run inside `web/`):

```sh
bun install
bunx tsc --noEmit
bunx biome format .
bun test tests/unit
bunx playwright install chromium
bunx playwright test
```

`bunx biome format --write .` fixes the formatting (Biome 2.6.2, line width 140, two-space indent).

`bun test tests/unit` also runs `scripts/rust-diff.sh` and `scripts/coverage-functions.sh` on made-up input, so `jq` and `c++filt` (from binutils) must be installed.

A few tests need root or a real systemd and run only in CI: `sudo scripts/fault-fs.sh target/debug/freespace` fills a tmpfs during a scan, and the `install` job runs `install.sh` end to end on the runner. `shellcheck install.sh scripts/*.sh` must be clean.

Scanning your whole disk while developing is slow; point commands at a small tree with a private snapshot instead:

```sh
export FREESPACE_CONFIG=/tmp/fs-dev/config.toml FREESPACE_SNAPSHOT_DIR=/tmp/fs-dev/snap
cargo run -- scan ~/some/folder
cargo run -- ls ~/some/folder
```

## Rules that tests enforce

- Every `freespace` command in `README.md` and `skill/SKILL.md` runs in `cli/tests/skill_contract.rs`; `--json` commands must validate against `skill/schema.json`. Mark a block that must not run (sudo, gui, curl) with `<!-- no-run -->` on the line before it; its commands are still parsed. Mark sample output with `<!-- output -->`.
- After changing a `--json` answer, regenerate the schema: `UPDATE_SCHEMA=1 cargo test -p freespace-core --test schema`. The same test fails when the file is stale.
- Every error code needs a row with an anchor in the README errors table.
- A new e2e spec in `web/tests/e2e/` goes into every e2e list under `dependencies` in `.gstack/pregate.json` (the lists are literal paths, no globs); `web/tests/unit/test-files.test.ts` fails until it does.
- freespace never deletes, moves or changes files it scans. Changes that would need write access to scanned trees are out of scope.

## Pull requests

One topic per PR, with tests. Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`). Add a line to `CHANGELOG.md` under Unreleased for anything a user would notice.

A PR that should change no behavior (a refactor, a file split) carries the `refactor` label. The label starts `.github/workflows/refactor-guard.yml`, which compares the PR against the commit it branched from:

- `rust-diff`: both CLI builds answer the same questions alike, once times, ids, live disk numbers and each binary's own version are masked; text byte for byte, `--json` as `jq` prints it (so JSON layout and escapes are not compared). The questions: `status`, `ls`, `top`, `find`, `diff`, `doctor`, text and `--json`, on one fixture snapshot and on a fresh scan; `refresh` on a copy of the fixture, and questions after it; and, once each, the bare run, `--version`, every `--help`, a usage error, `completions`, `schema` and `skill print`. After each, the snapshot folder and its files, with their modes, must match too. A call still running after 5 minutes is stopped and answers exit 124. Locally: `scripts/rust-diff.sh <base-binary> <head-binary> <out-dir>`; `doctor` reads this host's timer and installed binary and skill, so on a machine with freespace installed a local run can differ where CI's does not.
- `pixel-diff`: both page builds draw every view and state of the e2e mock at 1280, 700 and 360 px into byte-identical PNG files, and on the way to each make the same requests of the mock API (`<state>-<width>-requests.txt`, one per line, sorted), so a build that asks for other rows or asks twice differs though it draws the same. Each build is taken twice; a head whose two takes differ fails on its own, as drawn by chance. The spec is `web/tests/guard/views.spec.ts`; the artifact holds every shot and requests file of each take, to compare by hand, and a diff for each whose difference Playwright's own comparison sees (it skips pixels it takes for anti-aliasing, so a byte difference can come without one).
- `coverage`: line coverage per function of the Rust files the PR touches, in the job summary. It never fails the PR.

Each comparison first runs base against itself; if that differs, the job fails as unreliable before it looks at the PR. The jobs run head's harness (the scripts, the spec, the mock and the workflow), so a refactor PR that changes one of those files gets a warning on it: the guard then judges the PR with the harness the PR changed. A PR titled `refactor:` (or `refactor(scope):`) or on a `refactor/` branch fails `.github/workflows/refactor-label.yml` until it carries the label. A push or the `refactor` label starts the guard again, cancelling the run in progress; another label leaves that run alone and starts one more beside it.

## Releases

Maintainers tag `vX.Y.Z` on master after bumping `version` in `Cargo.toml` and moving the Unreleased notes under the new version. The `release` workflow (cargo-dist, `dist-workspace.toml`) builds the static musl archives and publishes the GitHub release that `install.sh` downloads. After changing `dist-workspace.toml`, run `dist generate` to refresh `.github/workflows/release.yml`.
