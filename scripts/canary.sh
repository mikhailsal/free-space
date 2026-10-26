#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || {
  printf 'canary: run this script inside a Git work tree\n' >&2
  exit 1
}
for tool in git cargo python3 bun; do
  command -v "$tool" >/dev/null 2>&1 || { printf 'canary: %s is required\n' "$tool" >&2; exit 1; }
done

tmp=$(mktemp -d -t freespace-canary.XXXXXX)
worktree="$tmp/worktree"
log="$tmp/output.log"
target_dir=${CARGO_TARGET_DIR:-$repo_root/target}
export CARGO_TARGET_DIR="$target_dir"
cleanup() {
  git -C "$repo_root" worktree remove --force "$worktree" >/dev/null 2>&1 || true
  rm -rf -- "$tmp"
}
trap cleanup EXIT

git -C "$repo_root" worktree add --detach "$worktree" HEAD >/dev/null

fail() {
  printf 'canary: %s\n' "$*" >&2
  exit 1
}

reset_tree() {
  git -C "$worktree" checkout -- .
  git -C "$worktree" clean -fdq
}

assert_diagnostic() {
  local rule=$1 path=$2
  python3 - "$log" "$rule" "$path" <<'PY'
from pathlib import Path
import sys
import re

text = Path(sys.argv[1]).read_text(errors="replace")
text = re.sub(r"\x1b\][^\x1b]*(?:\x1b\\|\x07)", "", text)
text = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", text)
lines = text.splitlines()
rule, path = sys.argv[2:]
records = []
current = []
for line in lines:
    # Clippy starts each diagnostic with error:, Biome with a file:line:column
    # header, and the shell gates emit one check: line per violation.
    begins = (line.startswith(("error:", "warning:", "check-size:", "check:", "check-rust-suppressions:"))
              or re.match(r"^(?:src|tests)/[^:]+:\d+:\d+\b", line))
    if begins and current:
        records.append("\n".join(current))
        current = []
    current.append(line)
if current:
    records.append("\n".join(current))
if any(rule in record and path in record for record in records):
    raise SystemExit(0)
raise SystemExit(1)
PY
}

run_case() {
  local name=$1 part=$2 rule=$3 path=$4 injector=$5
  shift 5
  local diagnostic_path=$path
  [[ "$part" != biome ]] || diagnostic_path=${path#web/}
  reset_tree
  "$injector" "$path" "$@"
  if [[ "$part" == biome ]]; then
    (cd "$worktree/web" && bun install --frozen-lockfile) >>"$log" 2>&1 || {
      cat "$log" >&2
      fail 'could not install web tools in the temporary worktree'
    }
  fi
  if (cd "$worktree" && scripts/check.sh "$part") >"$log" 2>&1; then
    cat "$log" >&2
    fail "$name unexpectedly passed"
  fi
  if ! assert_diagnostic "$rule" "$diagnostic_path"; then
    cat "$log" >&2
    fail "$name failed without a nearby $rule diagnostic naming $path"
  fi
  printf 'canary: PASS %s (%s: %s)\n' "$name" "$rule" "$path"
  reset_tree
}

inject_long_rust() {
  python3 - "$worktree/$1" "${2:-}" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
body = "\n".join("    n += 1;" for _ in range(79))
with path.open("a") as f:
    if sys.argv[2] == "suppress":
        f.write("\n#[allow(clippy::too_many_lines)]")
    elif sys.argv[2] == "comment":
        f.write("\n#[allow(clippy:: /* canary */ too_many_lines)]")
    elif sys.argv[2] == "cfg_doc":
        f.write('\n#[cfg_attr(all(), doc = "]", allow(clippy::too_many_lines))]')
    elif sys.argv[2] == "raw":
        f.write("\n#[allow(clippy::r#too_many_lines)]")
    f.write("\n#[allow(dead_code)]\nfn canary_long_function() {\n    let mut n = 0;\n")
    f.write(body + "\n    std::hint::black_box(n);\n}\n")
PY
}

inject_nested_rust() {
  python3 - "$worktree/$1" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = ["\n#[allow(dead_code)]", "fn canary_nested_block() {", "    let condition = std::hint::black_box(true);"]
for depth in range(5):
    lines.append("    " * (depth + 1) + "if condition {")
lines.append("    " * 6 + "std::hint::black_box(());")
for depth in reversed(range(5)):
    lines.append("    " * (depth + 1) + "}")
lines.append("}")
with path.open("a") as f:
    f.write("\n".join(lines) + "\n")
PY
}

inject_ts_length() {
  python3 - "$worktree/$1" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
body = "\n".join("  value += 1;" for _ in range(79))
path.write_text("function canaryLongFunction() {\n  let value = 0;\n" + body + "\n  return value;\n}\nvoid canaryLongFunction;\n")
PY
}

inject_ts_complexity() {
  python3 - "$worktree/$1" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
body = "\n".join("  if (Math.random() > 2) { value += 1; }" for _ in range(26))
path.write_text("function canaryComplexity() {\n  let value = 0;\n" + body + "\n  return value;\n}\nvoid canaryComplexity;\n")
PY
}

inject_format() {
  printf 'const canaryFormat={value:1};\nvoid canaryFormat;\n' > "$worktree/$1"
}

inject_suppression() {
  printf '// biome-ignore lint/suspicious/noExplicitAny: canary unused suppression\nconst canarySuppression = 1;\nvoid canarySuppression;\n' > "$worktree/$1"
}

inject_used_suppression() {
  python3 - "$worktree/$1" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
body = "\n".join("  value += 1;" for _ in range(79))
with path.open("a") as f:
    f.write("\n// biome-ignore lint/complexity/noExcessiveLinesPerFunction: canary used suppression\n")
    f.write("function canarySuppressedLength() {\n  let value = 0;\n")
    f.write(body + "\n  return value;\n}\nvoid canarySuppressedLength;\n")
PY
}

inject_lines() {
  local count=$2
  awk -v n="$count" 'BEGIN { for (i=0;i<n;i++) print "line" }' > "$worktree/$1"
}

metadata="$tmp/metadata.json"
(cd "$worktree" && cargo metadata --no-deps --format-version 1) > "$metadata"
python3 - "$metadata" "$worktree" > "$tmp/rust-members.tsv" <<'PY'
import json
import os
import sys

metadata = json.load(open(sys.argv[1], encoding="utf-8"))
root = sys.argv[2]
members = set(metadata["workspace_members"])
for package in metadata["packages"]:
    if package["id"] not in members:
        continue
    targets = package["targets"]
    preferred = [target for target in targets if "lib" in target["kind"]]
    if not preferred:
        preferred = [target for target in targets if "bin" in target["kind"]]
    if not preferred:
        raise SystemExit(f"no lib/bin target for workspace member {package['name']}")
    source = os.path.relpath(preferred[0]["src_path"], root)
    print(f"{package['name']}\t{source}")
PY

members=0
while IFS=$'\t' read -r package source; do
  [[ -n "$source" ]] || continue
  run_case "Rust function length in $package" rust too_many_lines "$source" inject_long_rust
  ((members += 1))
done < "$tmp/rust-members.tsv"
(( members > 0 )) || fail 'cargo metadata returned no workspace members'

first_member=$(head -n 1 "$tmp/rust-members.tsv" | cut -f2)
run_case Rust-nesting rust excessive_nesting "$first_member" inject_nested_rust
run_case Rust-used-suppression rust too_many_lines "$first_member" inject_long_rust suppress
run_case Rust-comment-suppression rust too_many_lines "$first_member" inject_long_rust comment
run_case Rust-cfg-doc-suppression rust too_many_lines "$first_member" inject_long_rust cfg_doc
run_case Rust-raw-suppression rust too_many_lines "$first_member" inject_long_rust raw
run_case TypeScript-source-function-length biome noExcessiveLinesPerFunction web/src/canary-length.ts inject_ts_length
run_case TypeScript-test-function-length biome noExcessiveLinesPerFunction web/tests/canary-length.ts inject_ts_length
run_case TypeScript-complexity biome noExcessiveCognitiveComplexity web/src/canary-complexity.ts inject_ts_complexity
run_case Biome-formatting biome format web/src/canary-format.ts inject_format
run_case Biome-unused-suppression biome 'unused suppression' web/src/canary-suppression.ts inject_suppression
run_case Biome-used-suppression biome 'Biome suppression forbidden' web/src/main.ts inject_used_suppression
css_file=$(git -C "$worktree" ls-files 'web/src/*.css' | head -n 1)
test_file=$(git -C "$worktree" ls-files 'core/tests/*.rs' | head -n 1)
[[ -n "$css_file" && -n "$test_file" ]] || fail 'could not find tracked CSS and Rust test files'
run_case Rust-file-size size 'limit 500' core/src/lib.rs inject_lines 501
run_case CSS-file-size size 'limit 500' "$css_file" inject_lines 501
run_case Test-file-size size 'limit 1000' "$test_file" inject_lines 1001

printf 'canary: all configured gates rejected their injected violation\n'
