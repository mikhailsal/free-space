#!/usr/bin/env bash
# CI ONLY: real host installer, never the ordinary non-root rust-diff.
# sudo scripts/rust-diff-install.sh --github-disposable-runner BASE HEAD OUT SCRATCH USER
# SCRATCH must not exist and must be directly inside RUNNER_TEMP. OUT holds raw evidence.
# shellcheck source-path=SCRIPTDIR
set -Eeuo pipefail
export LC_ALL=C
here=$(dirname "$(realpath "$0")")
# shellcheck source=rust-diff/install-kept.sh
. "$here/rust-diff/install-kept.sh"
[[ $# = 6 && $1 = --github-disposable-runner ]] || fail 'expected --github-disposable-runner BASE HEAD OUT SCRATCH USER'
install_require_runner
base=$(realpath -e "$2") || fail 'BASE path must exist'
head=$(realpath -e "$3") || fail 'HEAD path must exist'
out=$4 scratch=$5 human=$6
install_check_inputs
out=$(realpath -m "$out") scratch=$(realpath -m "$scratch")
[ -d /run/systemd/system ] || fail 'real systemd required'
case $(realpath -e /etc/localtime) in /usr/share/zoneinfo/Etc/UTC | /usr/share/zoneinfo/UTC) ;; *) fail 'systemd host timezone must be UTC for the future schedule' ;; esac
home=$(install_user_home "$human")
completions=(/usr/local/share/bash-completion/completions/freespace /usr/local/share/zsh/site-functions/_freespace /usr/local/share/fish/vendor_completions.d/freespace.fish)
paths=(/usr/local/bin/freespace /etc/freespace /var/lib/freespace /etc/systemd/system/freespace.service /etc/systemd/system/freespace.timer /etc/systemd/system/timers.target.wants/freespace.timer /var/lib/systemd/timers/stamp-freespace.timer "$home/.claude" "${completions[@]}")
# Refuse before ANY writes; don't overwrite an installation, timer history, skill or group.
install_pristine "${paths[@]}"
for leaf in "${paths[@]}"; do
  case $leaf in "$home/.claude") continue ;; esac
  install_root_ancestors "$leaf"
done
for p in / /usr /usr/local /usr/local/bin; do
  if [[ $(stat -c %u "$p") != 0 ]] || (( (8#$(stat -c %a "$p") & 022) != 0 )); then
    fail "$p must be root-owned and not group/other writable (CI prepares /usr/local)"
  fi
done
# Record existing completion parent directories; cleanup only newly created, empty parents.
new_parents=()
for p in /usr/local/share /usr/local/share/bash-completion /usr/local/share/bash-completion/completions /usr/local/share/zsh /usr/local/share/zsh/site-functions /usr/local/share/fish /usr/local/share/fish/vendor_completions.d; do
  [ -d "$p" ] || new_parents+=("$p")
done
mkdir -p "$out"
mkdir "$scratch"
active=false
cleanup() {
  local status=$? cleanup_status=0
  trap - EXIT ERR
  trap '' HUP INT TERM
  if $active; then
    if [[ -e /etc/systemd/system/freespace.timer || -L /etc/systemd/system/freespace.timer || -e /etc/systemd/system/timers.target.wants/freespace.timer || -L /etc/systemd/system/timers.target.wants/freespace.timer ]]; then
      systemctl disable --now freespace.timer >/dev/null 2>&1 || cleanup_status=1
    fi
    systemctl stop freespace.service >/dev/null 2>&1 || true
    rm -f -- /etc/systemd/system/freespace.service /etc/systemd/system/freespace.timer /etc/systemd/system/timers.target.wants/freespace.timer /var/lib/systemd/timers/stamp-freespace.timer /usr/local/bin/freespace "${completions[@]}" || cleanup_status=1
    rm -rf -- /etc/freespace /var/lib/freespace "$home/.claude" "$scratch" || cleanup_status=1
    systemctl daemon-reload || cleanup_status=1
    systemctl reset-failed freespace.service >/dev/null 2>&1 || true
    if getent group freespace >/dev/null; then groupdel freespace || cleanup_status=1; fi
  fi
  install_remove_empty_parents "${new_parents[@]}" || cleanup_status=1
  [ ! -d "$scratch" ] || rm -rf -- "$scratch" || cleanup_status=1
  if [ "$cleanup_status" != 0 ]; then echo "$0: cleanup failed" >&2; [ "$status" != 0 ] || status=5; fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'echo "$0: command failed at line $LINENO: $BASH_COMMAND" >&2' ERR
# All takes share the same future daily schedule, safely away from midnight/runs.
at=$(date -u -d '+12 hours' +%H:%M)

never_scanned() {
  [ "$(systemctl show freespace.service -p ExecMainStartTimestampMonotonic --value)" = 0 ] || fail 'service started despite --no-start/future schedule'
}

capture() {
  local p
  for p in "${paths[@]}" "${new_parents[@]}" "$scratch/snap"; do install_kept "$p" "$bin" "$version"; done
  getent group freespace
  getent gshadow freespace
  id "$human"
  unit_state freespace.timer LoadState ActiveState UnitFileState
  unit_state freespace.service LoadState ActiveState ExecMainStartTimestampMonotonic
}
take() {
  local label=$1 bin=$2 format=$3 version sh i flags=()
  [ "$format" != json ] || flags=(--json)
  version=$(run "$bin" --version); version=${version#freespace }
  mkdir "$out/$label"
  active=true
  run "$bin" install-timer --no-start --roots "$scratch" --snapshot-dir "$scratch/snap" --user "$human" --interval daily --at "$at" "${flags[@]}" >"$out/$label/install.raw.$format" 2>"$out/$label/install.stderr"
  if [ "$format" = json ]; then
    install_no_scan_report "$out/$label/install.raw.json" false "$bin"
    install_report "$out/$label/install.raw.json" "$bin" >"$out/$label/install.json"
  else
    install_text_check "$out/$label/install.raw.txt" "$version" /etc/freespace/config.toml "$scratch" daily "$at" true "$scratch/snap" "$human"
    install_text_report "$out/$label/install.raw.txt" "$version" >"$out/$label/install.txt"
  fi
  never_scanned
  for i in 0 1 2; do
    sh=(bash zsh fish)
    run "$bin" completions "${sh[i]}" >"$scratch/completion"
    install -D -o root -g root -m 0644 "$scratch/completion" "${completions[i]}"
  done
  run "$bin" skill install >"$out/$label/skill.stdout" 2>"$out/$label/skill.stderr"
  capture >"$out/$label/installed.txt"
  never_scanned
  run /usr/local/bin/freespace install-timer --no-start --roots "$scratch" --snapshot-dir "$scratch/snap" --user "$human" --interval daily --at "$at" --json >"$out/$label/rerun.raw.json" 2>"$out/$label/rerun.stderr"
  install_no_scan_report "$out/$label/rerun.raw.json" true
  install_report "$out/$label/rerun.raw.json" /usr/local/bin/freespace >"$out/$label/rerun.json"
  never_scanned
  capture >"$out/$label/installed-rerun.txt"
  cmp -- "$out/$label/installed.txt" "$out/$label/installed-rerun.txt" || fail 'installed manifest changed after installed-binary rerun'
  run "$bin" install-timer --uninstall --user "$human" "${flags[@]}" >"$out/$label/uninstall.$format" 2>"$out/$label/uninstall.stderr"
  capture >"$out/$label/residual.txt"
  # Preserve residues as evidence before resetting the exact pristine baseline.
  for p in "${paths[@]}" "$scratch/snap"; do
    case $p in "$home/.claude" | /var/lib/systemd/timers/stamp-freespace.timer) ;; *) [[ ! -e $p && ! -L $p ]] || fail "uninstall left $p" ;; esac
  done
  [ ! -e "$home/.claude/skills/freespace" ] || fail 'uninstall left skill files'
  rmdir "$home/.claude/skills" "$home/.claude"
  groupdel freespace
  rm -f /var/lib/systemd/timers/stamp-freespace.timer "$scratch/completion"
  install_remove_empty_parents "${new_parents[@]}"
  active=false
}

take base "$base" json
take base2 "$base" json
compare base base2 3
take base-text "$base" txt
take base2-text "$base" txt
compare base-text base2-text 3
# Both base self-checks must pass before either head take.
take head "$head" json
compare base head 1
take head-text "$head" txt
compare base-text head-text 1
echo "$0: installed manifest and uninstall residues match"
