#!/usr/bin/env bash
# Sourced by the disposable-runner install comparator; independently testable without root.
install_report() { # <raw-json> <source-binary>: only the source path and scheduled clock vary
  local next canonical
  next=$(jq -r 'if (.next_run | type) == "string" then .next_run else empty end' "$1")
  if [ -n "$next" ]; then
    canonical=$(LC_ALL=C TZ=UTC date -u -d "$next" '+%a %Y-%m-%d %H:%M:%S UTC') || return 1
    [ "$canonical" = "$next" ] || { echo 'next_run is not a canonical UTC calendar time' >&2; return 1; }
  fi
  jq --arg source "$2" '
    if .copied_from == $source then .copied_from = "<source-binary>" else . end |
    if .next_run == null then .
    elif (.next_run | type) != "string" then error("next_run must be string or null")
    elif (.next_run | test("^(Mon|Tue|Wed|Thu|Fri|Sat|Sun) [0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2} UTC$")) then
      .next_run |= sub("[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}"; "<scheduled-date-time>")
    else error("unexpected systemd next_run format; update the comparator explicitly") end' "$1"
}

install_no_scan_report() { # <raw-json> <rerun> [source-binary]
  jq -e --argjson rerun "$2" --arg source "${3:-}" '
    .scan_started == false and has("scan_reason") and .scan_reason == null and .access == "acl" and
    (.next_run | type) == "string" and
    has("copied_from") and (if $rerun then .copied_from == null else .copied_from == $source end)
  ' "$1" >/dev/null || fail 'install report must show ACL access, a scheduled next_run, expected copy behavior and no scan'
}

# Text stays byte-for-byte observable except the exact own header and canonical UTC clock.
install_text_report() { # <raw-text> <own-version>
  local next canonical
  next=$(perl -ne 'print $1 if /^  timer      freespace[.]timer, next run ((?:Mon|Tue|Wed|Thu|Fri|Sat|Sun) [0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2} UTC)\n?\z/' "$1")
  if [ -n "$next" ]; then
    canonical=$(LC_ALL=C TZ=UTC date -u -d "$next" '+%a %Y-%m-%d %H:%M:%S UTC') || return 1
    [ "$canonical" = "$next" ] || { echo 'next_run is not a canonical UTC calendar time' >&2; return 1; }
  fi
  OWN_VERSION=$2 perl -0777 -pe '
    s/\Afreespace \Q$ENV{OWN_VERSION}\E timer installed(?=\n|\z)/freespace <own-version> timer installed/;
    s/^(  timer      freespace[.]timer, next run (?:Mon|Tue|Wed|Thu|Fri|Sat|Sun)) [0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2} UTC(?=\n|\z)/$1 <scheduled-date-time> UTC/mg;
  ' "$1"
}

install_text_check() { # <raw> <own-version> <config> <roots> <interval> <at> <keep-previous> <snapshot> <user>
  local raw=$1 version=$2 config=$3 roots=$4 interval=$5 at=$6 keep=$7 snap=$8 user=$9 first_line
  [ "$(wc -l <"$raw")" = 4 ] || fail 'text install report must have exactly four no-scan lines'
  IFS= read -r first_line <"$raw"
  [ "$first_line" = "freespace $version timer installed" ] || fail 'text install report must show the exact own-version header'
  grep -Fx "  config     $config (roots $roots, $interval at $at, keep_previous $keep)" "$raw" >/dev/null || fail 'text install report must show the expected configuration'
  grep -Fx "  snapshots  $snap (read access for $user (ACL))" "$raw" >/dev/null || fail 'text install report must show ACL access'
  grep -Ex '  timer      freespace[.]timer, next run (Mon|Tue|Wed|Thu|Fri|Sat|Sun) [0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2} UTC' "$raw" >/dev/null || fail 'text install report must show a scheduled next_run'
}

install_skill_content() { # exact own stamp, preserving every other byte and final newline state
  OWN_VERSION=$2 perl -0777 -pe 's/^<!-- freespace-version: \Q$ENV{OWN_VERSION}\E -->(?=\n|\z)/<!-- freespace-version: <own-version> -->/mg' "$1"
}

install_kept() { # <path> <binary-path> <own-version>: actual metadata, ACL and contents; never follow links
  local p=$1 binary=$2 version=$3 f
  local LC_ALL=C
  export LC_ALL
  if [ ! -e "$p" ] && [ ! -L "$p" ]; then printf 'absent %s\n' "$p"; return; fi
  find -P "$p" -print0 | sort -z | while IFS= read -r -d '' f; do
    printf 'path %s\n' "$f"
    stat -c 'type=%F uid=%u gid=%g mode=%a links=%h target=%N' -- "$f"
    if [ ! -L "$f" ]; then getfacl --absolute-names --numeric -- "$f"; fi
    if [ -f "$f" ] && [ ! -L "$f" ]; then
      if [ "$f" = /usr/local/bin/freespace ]; then
        install_verify_binary "$f" "$binary"
      elif [[ $f = */freespace/SKILL.md ]]; then
        # Only the exact own-version stamp; all other versions/text remain observable.
        install_skill_content "$f" "$version"
      else cat -- "$f"; fi
      printf '\n-- end content\n'
    fi
  done
}

install_remove_empty_parents() { # directories in parent-before-child order; never remove another file
  local i parents=("$@")
  for ((i=${#parents[@]}-1; i>=0; i--)); do
    [ ! -d "${parents[i]}" ] || rmdir -- "${parents[i]}" || return 1
  done
}

# Operational boundaries shared by take(), capture() and the base/head comparison.
run() {
  local home=${home:?} human=${human:?}
  env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin HOME="$home" SUDO_USER="$human" TZ=UTC LC_ALL=C \
    timeout -k 10 120 "$@" </dev/null
}

unit_state() {
  local unit=$1 prop code=0 args=() scratch=${scratch:?}
  shift
  for prop in "$@"; do args+=(-p "$prop"); done
  systemctl show "$unit" "${args[@]}" >"$scratch/unit-state" || code=$?
  cat "$scratch/unit-state"
  echo "systemctl-show-exit=$code"
  if [ "$code" != 0 ]; then
    [ "$code" = 1 ] || return "$code"
    grep -qx LoadState=not-found "$scratch/unit-state" || return "$code"
  fi
}

compare() {
  local left=$1 right=$2 mismatch=$3 f code label source_bin own_version format=json
  [[ $left != *-text ]] || format=txt
  local base=${base:?} head=${head:?} out=${out:?} scratch=${scratch:?}
  for f in installed.txt installed-rerun.txt residual.txt "install.$format" "uninstall.$format" rerun.json install.stderr rerun.stderr uninstall.stderr skill.stdout skill.stderr; do
    # Exact source path and exact version in diagnostic text only; no time/id/global regex masks.
    for label in "$left" "$right"; do
      case $label in head | head-text) source_bin=$head ;; *) source_bin=$base ;; esac
      own_version=$(run "$source_bin" --version); own_version=${own_version#freespace }
      if [[ $f = *.stderr ]]; then
        SOURCE_BIN=$source_bin OWN_VERSION=$own_version perl -pe 's/^installed \/usr\/local\/bin\/freespace \(copied from \Q$ENV{SOURCE_BIN}\E\)$/installed \/usr\/local\/bin\/freespace (copied from <source-binary>)/; s/^freespace skill \Q$ENV{OWN_VERSION}\E installed in /freespace skill <own-version> installed in /' "$out/$label/$f" >"$scratch/$label.txt"
      else cp "$out/$label/$f" "$scratch/$label.txt"; fi
    done
    code=0
    diff -u "$scratch/$left.txt" "$scratch/$right.txt" || code=$?
    case $code in
      0) ;;
      1) echo "$0: $right differs from $left ($f)" >&2; exit "$mismatch" ;;
      *) echo "$0: diff failed ($code)" >&2; exit 5 ;;
    esac
  done
}

# Preflight operations are called before main creates output/scratch or touches the manifest.
fail() { echo "$0: $*" >&2; exit 2; }
install_require_runner() {
  [[ $(id -u) = 0 && ${GITHUB_ACTIONS:-} = true && ${RUNNER_ENVIRONMENT:-} = github-hosted ]] || fail 'requires sudo on a disposable GitHub-hosted runner'
}
install_check_inputs() {
  local base=${base:?} head=${head:?} out=${out:?} scratch=${scratch:?} tool
  for tool in jq getfacl setfacl systemctl systemd-analyze groupadd groupdel getent timeout find sort stat cmp perl grep wc date; do
    command -v "$tool" >/dev/null || fail "$tool required (install acl and standard Ubuntu system tools)"
  done
  [[ -x $base && -x $head ]] || fail 'both builds must be executable'
  [ "$(realpath -e "$(dirname "$scratch")")" = "$(realpath -e "${RUNNER_TEMP:?RUNNER_TEMP required}")" ] || fail 'scratch must be directly inside RUNNER_TEMP'
  [[ $scratch != *[[:space:]]* ]] || fail 'scratch path must contain no whitespace'
  [[ ! -e $scratch && ! -L $scratch ]] || fail 'scratch must not exist'
  [ "$(realpath -e "$(dirname "$out")")" = "$(realpath -e "$RUNNER_TEMP")" ] || fail 'OUT must be directly inside RUNNER_TEMP'
  [[ ! -e $out && ! -L $out ]] || fail 'OUT must not exist'
  [[ $(realpath -m "$out") != "$(realpath -m "$scratch")" ]] || fail 'OUT must be outside scratch'
}
install_user_home() {
  local human=$1 entry home
  entry=$(getent passwd "$human") || fail 'USER must exist'
  home=$(cut -d: -f6 <<<"$entry")
  [[ $human != root && $home != / && -d $home ]] || fail 'USER must have an existing non-root home'
  printf '%s\n' "$home"
}
install_pristine() {
  local p unit state
  for p in "$@"; do [[ ! -e $p && ! -L $p ]] || fail "runner not pristine: $p exists"; done
  if getent group freespace >/dev/null; then fail 'runner not pristine: freespace group exists'; fi
  for unit in freespace.timer freespace.service; do
    state=$(systemctl show "$unit" -p LoadState --value 2>/dev/null) || true
    [ "$state" = not-found ] || fail "runner not pristine: $unit is known to systemd ($state)"
  done
}
install_root_directory() {
  local p=$1
  [ ! -L "$p" ] || fail "unsafe symlink ancestor: $p"
  if [ -e "$p" ]; then
    if [[ ! -d $p || $(stat -c %u "$p") != 0 ]] || (( (8#$(stat -c %a "$p") & 022) != 0 )); then
      fail "unsafe manifest ancestor: $p"
    fi
  fi
}
install_root_ancestors() {
  local p
  p=$(dirname "$1")
  while [ "$p" != / ]; do
    install_root_directory "$p"
    p=$(dirname "$p")
  done
}
install_verify_binary() { # <installed-copy> <own-input>: builds inherently differ; their copy must not
  cmp -- "$1" "$2"
  echo 'content=identical-to-input-binary'
}
