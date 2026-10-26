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

`bun test tests/unit` also runs `scripts/rust-diff.sh` and `scripts/coverage-functions.sh` on made-up input, so `jq`, `curl`, `perl`, `sqlite3`, `script`, `flock` and `setsid` (util-linux), `c++filt` (binutils), `getfacl`/`setfacl` (acl) and tzdata must be installed.

The pre-gate runner, `scripts/test-files.sh`, routes selected Rust files (including integration helpers and source modules with unit tests) to one Cargo run per crate. Cargo runs at low priority with two build jobs and one test thread. It needs `setsid` from util-linux to stop Cargo and its descendants together; e2e, shots and guard runs need `flock` too. Its unit tests also use these tools.

A few tests need root or a real systemd and run only in CI: `sudo scripts/fault-fs.sh target/debug/freespace` fills a tmpfs during a scan, `sudo scripts/mount-fs.sh target/debug/freespace` mounts one inside a scanned root and refreshes around it, and the `install` job runs `install.sh` end to end on the runner. `shellcheck install.sh scripts/*.sh scripts/rust-diff/*.sh` must be clean (`.shellcheckrc` lets it follow sourced files).

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

- `rust-install-diff`: a separate sudo comparator (`scripts/rust-diff-install.sh`) on a disposable GitHub-hosted runner. It builds base in a separate checkout and target directory, uses the cached head target, and self-checks base twice for JSON and text before comparing head. Every take uses `--no-start`, one fixed future daily schedule and a scratch snapshot directory. It records real unit/config contents, file types, numeric owners, modes, link targets, ACLs, group membership, timer/service state, JSON and text install/uninstall reports, installed-binary rerun reports and uninstall residues. The installed binary must match its own input bytes. `install-timer` does not create completions or the skill: the harness separately runs each binary’s `completions` and `skill install` commands into the same manifest destinations that `install.sh` uses, then tests their removal by `install-timer --uninstall`. Only the exact source-binary path, own-version skill stamp/diagnostic and text header, and recognized UTC scheduled date/time in `next_run` or its text line are normalized; null, weekday, extra fields and unexpected shapes remain visible or fail. Raw JSON/text reports and diagnostics remain artifacts. The harness refuses pre-existing installation paths, timer history, skill parents, groups or loaded units before writing, requires root-only manifest ancestors and ACL support, and checks that the service has never started. After recording residues it removes the deliberately retained group, persistent timer stamp and newly created empty parent directories to reset the baseline. Local tests exercise parsing and real scratch-file metadata/ACLs without installing anything; real systemd lifecycle, timer scheduling and root cleanup must be verified by this GitHub job. This covers the ACL path and `--no-start`; fallback group access, actual scans and `install.sh` downloading/escalation remain the existing install job’s responsibility.
- `rust-diff`: both CLI builds answer the same questions alike, once times (in their shape: `<date> <hh:mm>`, `<date>T<hh:mm:ss>`), ids, live disk numbers, the server's port and token, the pid in `gui`'s redirect file name and each binary's own version are masked (a masked JSON value keeps its key and becomes its type, `"<number>"`, so a key that comes or goes still shows; an age bound such as `1y` is echoed as the age in days); text byte for byte, `--json` as `jq` prints it (so JSON layout and escapes are not compared). The questions (`scripts/rust-diff/questions.sh`): `status`, `ls`, `top`, `find`, `diff`, `doctor`, text and `--json`, on one fixture snapshot and on a snapshot each build makes itself (two scans with 8 walker threads, the tree changed in between); the date questions again at UTC+5:30; a few on a terminal; globs (classes, escapes, `?` on a name that is not UTF-8, and the patterns it refuses); paths relative to a folder of the tree, through a symlink and `..`, not UTF-8, and as `--path-raw` hex; `serve`'s `/api` answers (status, headers, body), its refusals (no token, a token too long, too short or one character off, a foreign origin, a cross-site or same-site fetch, another host name, a POST; `localhost` and its own origin pass), a HEAD, the page's status and headers, the address it listens on, and its stop on TERM; `gui` on a terminal over ssh, with a browser that opens and with one that fails (what it prints, the redirect file in `XDG_RUNTIME_DIR`, what `xdg-open` got, and that the first request with the token removes the file); `refresh` on a copy of the fixture and on the build's own snapshot, questions after it, and its refusals (a folder that became a file, a folder under a symlink, a symlink, a root gone); questions, a scan and a refresh with no snapshot folder, during the first scan and during a scan over the fixture (a held lock); a config file with two roots, then with one of them gone; and, once each, the bare run, `--version`, every `--help`, usage errors, `install-timer` without root, `completions`, `schema` and `skill print`. The trees (`scripts/rust-diff/tree.sh`) hold hard links (across folders, and across the folder a refresh rewalks), a sparse file, a symlink, a file and a folder whose names are not UTF-8, a folder nobody may read, files just past midnight in one time zone, files dated by age from the start of the run (so `2w`, `6mo`, `400d`, `1y` and `2y` split them, in the wide part too), a wide, deep part and folders of distinct dates. Each run starts from a copy of the fixture snapshot and an empty `$HOME` and unit dir; after each part the snapshot folder, `$HOME` and the unit dir (names, types, modes, a symlink's target), `state.json`, every other file as it is (a lock a scan leaves, `gui`'s redirect file) and each database row by row (schema, `user_version`, every row of every table, generated columns and SQLite's statistics too; the columns a scan fills from the clock or the disk as their type, a hard link's inode as the first entry that has it, paths in hex with the work dir masked) must match too, and a command that changes the tree it reads stops the job. Base must answer each question with the exit status `questions.sh` lists for it (0 when it lists none), a refusal with the words it lists, `--json` with JSON (blank output is none), and `serve` and `gui` must stop on TERM with exit 0, or the job stops as stale (exit 4) before head runs: a question both builds refuse alike would test nothing, so a PR that renames a flag or a path updates `questions.sh` too; the `rust` job of `ci.yml` runs head against itself on every PR (two copies in two files, as the guard runs two builds), so such a PR fails there and not on the next refactor PR. A call still running after `RUST_DIFF_TIMEOUT` seconds (default 300, a whole number from 1) is stopped and answers exit 124. A command of the harness's own that fails stops the job with exit 5 and names it, apart from exit 1 (head differs); so does a `diff` that cannot compare two answers or two listings of a tree. Not compared: the page's bytes (`pixel-diff` draws them), a real browser and `gui`'s timers (no answer waits for them), a scan of the host's own filesystems (the default roots), and the scan's progress line on a terminal (its counts race the walk). `RUST_DIFF_FAN` (default 4, from 1 to 9) sets the wide part of the trees. Locally: `scripts/rust-diff.sh <base-binary> <head-binary> <out-dir>`, as a user (it refuses root: `install-timer` would install the build on the host), with `jq`, `curl`, `perl`, `sqlite3`, `script` and `flock` (util-linux) and tzdata installed (it stops with exit 2 without one: without the zone the date questions at UTC+5:30 would be asked at UTC); `doctor` reads this host's timer and installed binary and skill, so on a machine with freespace installed a local run can differ where CI's does not.
- `pixel-diff`: both page builds draw every view and state of the e2e mock at 1280, 700 and 360 px into byte-identical PNG files, and on the way to each make the same requests of the mock API (`<state>-<width>-requests.txt`, one per line with its query in name order, the lines sorted), so a build that asks for other rows or asks twice differs though it draws the same. Each build is taken twice; a head whose two takes differ fails on its own, as drawn by chance. The spec is `web/tests/guard/views.spec.ts`; the artifact holds every shot and requests file of each take, to compare by hand, and a diff for each whose difference Playwright's own comparison sees (it skips pixels it takes for anti-aliasing, so a byte difference can come without one).
- `coverage`: line coverage per function of the Rust files the PR touches, in the job summary. It never fails the PR.

Each comparison first runs base against itself; if that differs, the job fails as unreliable before it looks at the PR. The jobs run head's harness (the scripts, the spec, the mock and the workflow), so a refactor PR that changes one of those files gets a warning on it: the guard then judges the PR with the harness the PR changed. A PR titled `refactor:` (or `refactor(scope):`) or on a `refactor/` branch fails `.github/workflows/refactor-label.yml` until it carries the label. A push or the `refactor` label starts the guard again, cancelling the run in progress; another label leaves that run alone and starts one more beside it.

## Releases

Maintainers tag `vX.Y.Z` on master after bumping `version` in `Cargo.toml` and moving the Unreleased notes under the new version. The `release` workflow (cargo-dist, `dist-workspace.toml`) builds the static musl archives and publishes the GitHub release that `install.sh` downloads. After changing `dist-workspace.toml`, run `dist generate` to refresh `.github/workflows/release.yml`.
