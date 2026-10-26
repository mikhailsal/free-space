#!/usr/bin/env bash
set -euo pipefail

if ! repo_root=$(git rev-parse --show-toplevel 2>/dev/null); then
  printf 'check-size: run this script inside a Git work tree\n' >&2
  exit 1
fi

cd "$repo_root"

files_found=0
violations=0

while IFS= read -r -d '' path; do
  case "$path" in
    docs/*) continue ;;
    .githooks/*) ;;
    *.rs|*.ts|*.js|*.css|*.html|*.sh) ;;
    *) continue ;;
  esac

  # A tracked path can be absent after a worktree deletion or rename.
  [[ -f "$path" ]] || continue
  files_found=1

  limit=500
  case "$path" in
    */tests/*|*/tests.rs) limit=1000 ;;
  esac

  # awk counts records, including a final physical line without a newline.
  lines=$(awk 'END { print NR }' < "$path")
  if (( lines > limit )); then
    printf 'check-size: %s has %s lines (limit %s)\n' "$path" "$lines" "$limit" >&2
    violations=1
  fi
done < <(git ls-files -z)

if (( ! files_found )); then
  printf 'check-size: no tracked source files found in scope\n' >&2
  exit 1
fi

exit "$violations"
