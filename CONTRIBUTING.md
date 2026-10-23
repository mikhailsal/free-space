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

One topic per PR, with tests. Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `test:`, `docs:`, `chore:`). Add a line to `CHANGELOG.md` under Unreleased for anything a user would notice.

## Releases

Maintainers tag `vX.Y.Z` on master after bumping `version` in `Cargo.toml` and moving the Unreleased notes under the new version. The `release` workflow (cargo-dist, `dist-workspace.toml`) builds the static musl archives and publishes the GitHub release that `install.sh` downloads. After changing `dist-workspace.toml`, run `dist generate` to refresh `.github/workflows/release.yml`.
