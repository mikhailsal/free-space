#!/bin/sh
# Runs the given repo-relative test files, each with its own runner:
#   web/tests/unit/*   bun test
#   web/tests/e2e/*    Playwright (playwright.config.ts)
#   web/tests/shots/*  Playwright with playwright.shots.config.ts, into a temp
#                      dir ($SHOTS_DIR); `bun run shots` is what rewrites docs/img
#   web/tests/guard/*  Playwright with playwright.guard.config.ts, twice, each run
#                      shooting into its own temp dir ($GUARD_SHOTS); the two must
#                      match byte for byte (refactor-guard on GitHub compares base
#                      with head the same way; here the build with itself)
# The `runner` of .gstack/pregate.json, so /ship runs the tests a change selects:
#   scripts/test-files.sh web/tests/unit/format.test.ts web/tests/e2e/find.spec.ts
set -eu

[ "$#" -gt 0 ] || { echo "usage: $0 test-file..." >&2; exit 2; }
root=$(cd "$(dirname "$0")/.." && pwd)
unit=
e2e=
shots=
guard=
for f in "$@"; do
  case $f in
    # The lists below are split on spaces, so a path must not need quoting.
    *[!A-Za-z0-9._/-]* | *..*) echo "$0: unsafe test path $f" >&2; exit 2 ;;
    web/tests/unit/*) unit="$unit ./${f#web/}" ;;
    web/tests/e2e/*) e2e="$e2e ${f#web/}" ;;
    web/tests/shots/*) shots="$shots ${f#web/}" ;;
    web/tests/guard/*) guard="$guard ${f#web/}" ;;
    *) echo "$0: no runner for $f" >&2; exit 2 ;;
  esac
  # A runner given a missing file next to others runs the others and passes.
  [ -f "$root/$f" ] || { echo "$0: no such test file $f" >&2; exit 2; }
done

cd "$root/web"
# shellcheck disable=SC2086
if [ -n "$unit" ]; then bun test $unit; fi
if [ -n "$e2e$shots$guard" ]; then
  # Runs of this script in one checkout share web/dist and test-results (Playwright
  # empties it at start), so a second run waits for the first; the pre-gate's stall
  # timeout bounds the wait. The runners get fd 9 closed (9<&-): a server orphaned
  # by a killed run must not keep holding the lock.
  exec 9<.
  if ! flock -n 9; then
    echo "$0: waiting for another e2e run in $(pwd)" >&2
    flock 9
  fi
  # A free port for this run's static server (playwright.config.ts): a server left
  # behind by a killed run keeps its port but blocks neither the next run nor a
  # run in another worktree.
  E2E_PORT=$(bun -e 'const s = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response() }); console.log(s.port); s.stop(true);')
  export E2E_PORT
fi
# shellcheck disable=SC2086
if [ -n "$e2e" ]; then bunx playwright test $e2e 9<&-; fi
if [ -n "$shots$guard" ]; then
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
  # The pre-gate stops a stalled lane with TERM; exiting from a signal trap runs the EXIT trap.
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM
fi
if [ -n "$shots" ]; then
  SHOTS_DIR=$scratch/shots
  export SHOTS_DIR
  mkdir "$SHOTS_DIR"
  # shellcheck disable=SC2086
  bunx playwright test -c playwright.shots.config.ts $shots 9<&-
fi
if [ -n "$guard" ]; then
  for take in a b; do
    GUARD_SHOTS=$scratch/guard-$take
    export GUARD_SHOTS
    mkdir "$GUARD_SHOTS"
    # shellcheck disable=SC2086
    bunx playwright test -c playwright.guard.config.ts --update-snapshots=all $guard 9<&-
  done
  if ! diff -rq "$scratch/guard-a" "$scratch/guard-b" >&2; then
    echo "$0: two runs of one build took different shots, so the guard cannot compare builds here" >&2
    exit 1
  fi
fi
