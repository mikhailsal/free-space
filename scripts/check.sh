#!/usr/bin/env bash
set -uo pipefail

parts=(rust size shell tsc biome)

if ! command -v git >/dev/null 2>&1; then
  printf 'check: git is required; install: sudo apt-get install git\n' >&2
  exit 1
fi
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

need_web_binary() {
  local binary=$1
  need_tool bunx 'install Bun: curl -fsSL https://bun.com/install | bash' || return 1
  if ! command -v "$repo_root/web/node_modules/.bin/$binary" >/dev/null 2>&1; then
    printf 'check: local %s is required; install: (cd web && bun install --frozen-lockfile)\n' "$binary" >&2
    return 1
  fi
}

reject_biome_suppressions() {
  local matches status=0
  matches=$(git grep -n -F 'biome-ignore' -- web/src web/tests web/playwright.config.ts web/playwright.shots.config.ts web/playwright.guard.config.ts) || status=$?
  case "$status" in
    0)
      while IFS= read -r match; do
        printf 'check: Biome suppression forbidden: %s\n' "$match" >&2
      done <<< "$matches"
      return 1
      ;;
    1) return 0 ;;
    *) printf 'check: could not inspect Biome suppressions\n' >&2; return 1 ;;
  esac
}

run_part() {
  local part=$1
  case "$part" in
    rust)
      need_tool cargo "install Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" || return 1
      need_tool rustfmt 'run rustup component add rustfmt for the active toolchain' || return 1
      need_tool cargo-clippy 'run rustup component add clippy for the active toolchain' || return 1
      need_tool python3 'install Python: sudo apt-get install python3' || return 1
      cargo fmt --all --check &&
        "$repo_root/scripts/check-rust-suppressions.sh" &&
        cargo clippy --workspace --all-targets -- -D warnings
      ;;
    size)
      "$repo_root/scripts/check-size.sh"
      ;;
    shell)
      need_tool shellcheck 'install ShellCheck: sudo apt-get install shellcheck' || return 1
      shellcheck install.sh scripts/*.sh scripts/rust-diff/*.sh .githooks/pre-push
      ;;
    tsc)
      need_web_binary tsc || return 1
      (cd "$repo_root/web" && bunx --no-install tsc --noEmit)
      ;;
    biome)
      need_web_binary biome || return 1
      reject_biome_suppressions || return 1
      (cd "$repo_root/web" && bunx --no-install biome ci --error-on-warnings)
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
