#!/bin/sh
# Runs the given repo-relative test files, each with its own runner:
#   web/tests/unit/*.test.ts   bun test
#   web/tests/e2e/*.spec.ts    Playwright (playwright.config.ts)
#   web/tests/shots/*.spec.ts  Playwright with playwright.shots.config.ts, into
#                              web/pregate-shots/shots ($SHOTS_DIR); `bun run shots` is what rewrites docs/img
#   web/tests/guard/*.spec.ts  Playwright with playwright.guard.config.ts, twice, each run
#                              shooting into its own dir ($GUARD_SHOTS); the two must
#                              match byte for byte (refactor-guard on GitHub compares base
#                              with head the same way; here the build with itself)
# The `runner` of .gstack/pregate.json, so /ship runs the tests a change selects:
#   scripts/test-files.sh web/tests/unit/format.test.ts web/tests/e2e/find.spec.ts
# Each runner runs in the background while the script waits for it, so a HUP, INT or TERM sent to the script alone
# reaches it too: the script passes it on as TERM, waits for the runner to exit, then exits 128 + the signal number.
# A run waiting for another one's lock says so every $TEST_FILES_HEARTBEAT seconds (default 30).
set -eu
unset CDPATH

say() { printf '%s\n' "$*" >&2; }

[ "$#" -gt 0 ] || { say "usage: $0 test-file..."; exit 2; }
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
unit=
e2e=
shots=
guard=
for f in "$@"; do
  case $f in
    # The lists below are split on spaces, so a path must not need quoting.
    *[!A-Za-z0-9._/-]* | *..*) say "$0: unsafe test path $f"; exit 2 ;;
    web/tests/unit/*.test.ts) unit="$unit ./${f#web/}" ;;
    web/tests/e2e/*.spec.ts) e2e="$e2e ${f#web/}" ;;
    web/tests/shots/*.spec.ts) shots="$shots ${f#web/}" ;;
    web/tests/guard/*.spec.ts) guard="$guard ${f#web/}" ;;
    # A runner given a helper finds no test in it and passes.
    web/tests/unit/* | web/tests/e2e/* | web/tests/shots/* | web/tests/guard/*) say "$0: not a test file $f"; exit 2 ;;
    *) say "$0: no runner for $f"; exit 2 ;;
  esac
  # A runner given a missing file next to others runs the others and passes.
  [ -f "$root/$f" ] || { say "$0: no such test file $f"; exit 2; }
done
if [ -n "$e2e$shots$guard" ] && ! command -v flock >/dev/null 2>&1; then
  say "$0: flock not found (util-linux); it keeps two e2e runs in one checkout apart"
  exit 2
fi

cd "$root/web"
child=
# On a signal: pass it on to the running runner as TERM and reap it, so the lock outlives it; then the EXIT trap.
stop() {
  trap '' HUP INT TERM
  if [ -n "$child" ]; then
    kill -TERM "$child" 2>/dev/null || :
    wait "$child" 2>/dev/null || :
  fi
  exit "$1"
}
trap 'stop 129' HUP
trap 'stop 130' INT
trap 'stop 143' TERM
# Runs a command in the background and waits for it; its status is the function's.
run() {
  "$@" &
  child=$!
  status=0
  wait "$child" || status=$?
  child=
  return "$status"
}

# The runners get fd 9 closed (9<&-): a server orphaned by a killed run must not keep holding the lock.
# shellcheck disable=SC2086
if [ -n "$unit" ]; then run bun test $unit 9<&-; fi
if [ -n "$e2e$shots$guard" ]; then
  # Runs of this script in one checkout share web/dist, test-results (Playwright empties it at start) and
  # pregate-shots, so a second run waits for the first; the pre-gate's stall timeout bounds the wait.
  exec 9<.
  beat=${TEST_FILES_HEARTBEAT:-30}
  status=0
  flock -n 9 || status=$?
  [ "$status" -ne 1 ] || say "$0: waiting for another e2e run in $(pwd)"
  waited=0
  while [ "$status" -eq 1 ]; do
    status=0
    run flock -w "$beat" 9 || status=$?
    waited=$((waited + beat))
    [ "$status" -ne 1 ] || say "$0: still waiting for another e2e run in $(pwd) (${waited} s)"
  done
  [ "$status" -eq 0 ] || { say "$0: flock failed with status $status"; exit 2; }
  # Shots go to a fixed, ignored dir, cleared under the lock: a run killed outright (SIGKILL skips the EXIT trap)
  # leaves it for the next run to clear.
  out=$root/web/pregate-shots
  rm -rf "$out"
  trap 'rm -rf "$out"' EXIT
  # A free port for this run's static server (playwright.config.ts): a server left
  # behind by a killed run keeps its port but blocks neither the next run nor a
  # run in another worktree.
  E2E_PORT=$(bun -e 'const s = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response() }); console.log(s.port); s.stop(true);')
  export E2E_PORT
fi
# shellcheck disable=SC2086
if [ -n "$e2e" ]; then run bunx playwright test $e2e 9<&-; fi
if [ -n "$shots" ]; then
  SHOTS_DIR=$out/shots
  export SHOTS_DIR
  mkdir -p "$SHOTS_DIR"
  # shellcheck disable=SC2086
  run bunx playwright test -c playwright.shots.config.ts $shots 9<&-
fi
if [ -n "$guard" ]; then
  for take in a b; do
    GUARD_SHOTS=$out/guard-$take
    export GUARD_SHOTS
    mkdir -p "$GUARD_SHOTS"
    # shellcheck disable=SC2086
    run bunx playwright test -c playwright.guard.config.ts --update-snapshots=all $guard 9<&-
  done
  if ! diff -rq "$out/guard-a" "$out/guard-b" >&2; then
    say "$0: two runs of one build took different shots, so the guard cannot compare builds here"
    exit 1
  fi
fi
