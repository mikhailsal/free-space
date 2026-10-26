#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || { printf 'test-check: run inside a Git work tree\n' >&2; exit 1; }
size_script="$repo_root/scripts/check-size.sh"
check_script="$repo_root/scripts/check.sh"
tmp=$(mktemp -d -t freespace-test-check.XXXXXX)
cleanup() { rm -rf -- "$tmp"; }
trap cleanup EXIT
fail() { printf 'test-check: %s\n' "$*" >&2; exit 1; }
pass() { printf 'test-check: PASS %s\n' "$*"; }
make_lines() { awk -v n="$1" 'BEGIN { for (i=0;i<n;i++) print "line" }' > "$2"; }
expect_size_fail() {
  local path=$1 rule=$2 output=$tmp/size.out
  if "$size_script" >"$output" 2>&1; then fail "check-size accepted $path"; fi
  grep -F "$path" "$output" | grep -F "$rule" >/dev/null || { cat "$output" >&2; fail "missing $path / $rule diagnostic"; }
  pass "size rejects $path ($rule)"
}

mkdir -p "$tmp/size" "$tmp/outside"
git -C "$tmp/size" init -q
git -C "$tmp/size" config user.email test@example.invalid
git -C "$tmp/size" config user.name test
cd "$tmp/size"
if (cd "$tmp/outside" && "$size_script") >"$tmp/outside.out" 2>&1; then fail 'size passed outside a work tree'; fi
grep -F 'inside a Git work tree' "$tmp/outside.out" >/dev/null || fail 'outside-worktree diagnostic missing'
pass 'size rejects execution outside a work tree'
git init -q "$tmp/empty"
if (cd "$tmp/empty" && "$size_script") >"$tmp/empty.out" 2>&1; then fail 'size passed with zero tracked files'; fi
grep -F 'no tracked source files' "$tmp/empty.out" >/dev/null || fail 'empty-scope diagnostic missing'
pass 'size rejects an empty tracked scope'

# Extensions in scope and .githooks files use the 500-line cap.
for suffix in rs ts js css html sh; do
  path="source.$suffix"
  make_lines 500 "$path"; git add "$path"; "$size_script" || fail "$path at 500 should pass"
  make_lines 501 "$path"; expect_size_fail "$path" 'limit 500'
  make_lines 1 "$path"
done
mkdir -p .githooks
make_lines 501 .githooks/pre-push; git add .githooks/pre-push
expect_size_fail .githooks/pre-push 'limit 500'
make_lines 1 .githooks/pre-push
pass 'size checks all counted source extensions and .githooks/*'

# Explicit exclusions.
mkdir -p docs
make_lines 501 README.md; make_lines 501 skill.json; make_lines 501 asset.png; make_lines 501 docs/x.js
git add README.md skill.json asset.png docs/x.js
"$size_script" || fail 'excluded files affected the gate'
pass 'size excludes Markdown, JSON, PNG and docs/**'

# All three test classifications receive 1000 lines.
mkdir -p alpha/tests beta web/tests
for path in alpha/tests/test.rs beta/tests.rs web/tests/test.ts; do make_lines 1000 "$path"; git add "$path"; done
"$size_script" || fail '1000-line test files should pass'
pass 'size accepts each test path form at 1000 lines'
for path in alpha/tests/test.rs beta/tests.rs web/tests/test.ts; do
  make_lines 1001 "$path"; expect_size_fail "$path" 'limit 1000'; make_lines 1000 "$path"
done
printf 'line\nline' > 'path with space.rs'; git add 'path with space.rs'
for ((i=1;i<500;i++)); do printf 'line\n' >> 'path with space.rs'; done
printf 'last' >> 'path with space.rs'
expect_size_fail 'path with space.rs' 'limit 500'
make_lines 1 'path with space.rs'
pass 'size counts an unterminated line and handles spaces'
make_lines 500 deleted.rs; git add deleted.rs; python3 -c 'from pathlib import Path; Path("deleted.rs").unlink()'
"$size_script" || fail 'deleted tracked file affected the gate'
pass 'size skips a deleted tracked file'

# Dispatcher tests use a separate repository and first-on-PATH stubs.
mkdir -p "$tmp/dispatch/scripts" "$tmp/dispatch/web" "$tmp/bin"
git -C "$tmp/dispatch" init -q
git -C "$tmp/dispatch" config user.email test@example.invalid
git -C "$tmp/dispatch" config user.name test
cp "$size_script" "$tmp/dispatch/scripts/check-size.sh"
cp "$check_script" "$tmp/dispatch/scripts/check.sh"
chmod +x "$tmp/dispatch/scripts/"*.sh
printf 'ok\n' > "$tmp/dispatch/ok.rs"; git -C "$tmp/dispatch" add .
for tool in cargo shellcheck bunx; do
  cat > "$tmp/bin/$tool" <<'STUB'
#!/usr/bin/env bash
printf '%s\t%s\t' "${0##*/}" "$PWD" >> "$STUB_LOG"
printf '<%s>' "$@" >> "$STUB_LOG"
printf '\n' >> "$STUB_LOG"
case "${0##*/}" in
  cargo) exit "${STUB_CARGO_STATUS:-0}" ;;
  shellcheck) exit "${STUB_SHELLCHECK_STATUS:-0}" ;;
  bunx) exit "${STUB_BUNX_STATUS:-0}" ;;
esac
STUB
  chmod +x "$tmp/bin/$tool"
done
dispatch="$tmp/dispatch/scripts/check.sh"
export STUB_LOG="$tmp/dispatch.log" PATH="$tmp/bin:$PATH"
assert_call() {
  local expected
  expected=$(printf '%s\t%s\t%s\n' "$1" "$2" "$3")
  grep -F "$expected" "$STUB_LOG" >/dev/null || fail "missing dispatcher call: $expected"
}
(cd "$tmp/dispatch" && "$dispatch") || fail 'default dispatch failed with successful stubs'
assert_call cargo "$tmp/dispatch" '<fmt><--all><--check>'
assert_call cargo "$tmp/dispatch" '<clippy><--workspace><--all-targets><--><-D><warnings>'
assert_call shellcheck "$tmp/dispatch" '<install.sh>'
assert_call bunx "$tmp/dispatch/web" '<tsc><--noEmit>'
assert_call bunx "$tmp/dispatch/web" '<biome><ci><--error-on-warnings>'
pass 'check with no arguments runs all five parts'

if (cd "$tmp/dispatch" && "$dispatch" unknown) >"$tmp/unknown.out" 2>&1; then fail 'unknown part was accepted'; else status=$?; fi
[[ ${status:-0} -eq 2 ]] || fail "unknown part exit was ${status:-0}"
grep -F 'rust size shell tsc biome' "$tmp/unknown.out" >/dev/null || fail 'unknown part list missing'
pass 'check rejects unknown parts with exit 2 and a list'

if (cd "$tmp/dispatch" && STUB_CARGO_STATUS=1 "$dispatch" rust size shell tsc biome) >"$tmp/fail.out" 2>&1; then fail 'first-part failure was hidden'; else status=$?; fi
[[ $status -eq 1 ]] || fail "failure exit was $status"
assert_call bunx "$tmp/dispatch/web" '<biome><ci><--error-on-warnings>' || fail 'later parts did not run'
pass 'check fails overall but continues after a failed part'

mkdir "$tmp/minimal"
ln -s "$(command -v git)" "$tmp/minimal/git"
if (cd "$tmp/dispatch" && PATH="$tmp/minimal" /bin/bash scripts/check.sh shell) >"$tmp/missing.out" 2>&1; then fail 'missing shellcheck passed'; else status=$?; fi
[[ $status -eq 1 ]] || fail "missing tool exit was $status"
grep -F 'shellcheck is required; install ShellCheck' "$tmp/missing.out" >/dev/null || fail 'missing-tool hint absent'
pass 'check reports missing tools with install hints'

(cd "$tmp/dispatch/web" && "$dispatch" tsc) || fail 'check failed from web/'
assert_call bunx "$tmp/dispatch/web" '<tsc><--noEmit>' || fail 'subdirectory invocation did not resolve web root'
pass 'check works when started from web/'
printf 'test-check: all G1/G4 cases passed\n'
