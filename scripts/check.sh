#!/usr/bin/env bash
set -uo pipefail

parts=(rust size shell tsc biome)

if ! repo_root=$(git rev-parse --show-toplevel 2>/dev/null); then
  printf 'check: run this script inside a Git work tree\n' >&2
  exit 1
fi

cd "$repo_root" || exit 1

if (( $# == 0 )); then
  set -- "${parts[@]}"
else
  for requested in "$@"; do
    valid=0
    for part in "${parts[@]}"; do
      if [[ "$requested" == "$part" ]]; then
        valid=1
        break
      fi
    done
    if (( ! valid )); then
      printf 'check: unknown part %q; choose from: %s\n' "$requested" "${parts[*]}" >&2
      exit 2
    fi
  done
fi

failed=0

need_tool() {
  local tool=$1 hint=$2
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'check: %s is required; %s\n' "$tool" "$hint" >&2
    return 1
  fi
}

run_part() {
  local part=$1
  case "$part" in
    rust)
      need_tool cargo 'install Rust with https://rustup.rs/' || return 1
      cargo fmt --all --check &&
        cargo clippy --workspace --all-targets -- -D warnings
      ;;
    size)
      "$repo_root/scripts/check-size.sh"
      ;;
    shell)
      need_tool shellcheck 'install ShellCheck: https://www.shellcheck.net/' || return 1
      shellcheck install.sh scripts/*.sh scripts/rust-diff/*.sh .githooks/pre-push
      ;;
    tsc)
      need_tool bunx 'install Bun from https://bun.sh/docs/installation, then run cd web && bun install --frozen-lockfile' || return 1
      (cd "$repo_root/web" && bunx tsc --noEmit)
      ;;
    biome)
      need_tool bunx 'install Bun from https://bun.sh/docs/installation, then run cd web && bun install --frozen-lockfile' || return 1
      (cd "$repo_root/web" && bunx biome ci --error-on-warnings)
      ;;
  esac
}

for part in "$@"; do
  printf 'check: %s\n' "$part"
  if ! run_part "$part"; then
    failed=1
  fi
done

exit "$failed"
