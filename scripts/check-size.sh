#!/usr/bin/env bash
set -euo pipefail

for tool in git awk mktemp rm; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    case "$tool" in
      git) printf 'check-size: git is required; install: sudo apt-get install git\n' >&2 ;;
      awk) printf 'check-size: awk is required; install: sudo apt-get install gawk\n' >&2 ;;
      mktemp|rm) printf 'check-size: %s is required; install: sudo apt-get install coreutils\n' "$tool" >&2 ;;
    esac
    exit 1
  fi
done

if ! repo_root=$(git rev-parse --show-toplevel 2>/dev/null); then
  printf 'check-size: run this script inside a Git work tree\n' >&2
  exit 1
fi

cd "$repo_root"

file_list=$(mktemp -t freespace-size.XXXXXX)
trap 'rm -f -- "$file_list"' EXIT
if ! git ls-files -z > "$file_list"; then
  printf 'check-size: could not list tracked files\n' >&2
  exit 1
fi

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
done < "$file_list"

if (( ! files_found )); then
  printf 'check-size: no tracked source files found in scope\n' >&2
  exit 1
fi

exit "$violations"
