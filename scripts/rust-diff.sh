#!/usr/bin/env bash
# The rust-diff job of .github/workflows/refactor-guard.yml (D15, D20, D23):
# two builds of the CLI must give the same answers to the same questions.
#
#   scripts/rust-diff.sh <base-bin> <head-bin> <out-dir>
#
# Base scans a made-up tree twice (with changes in between, so `diff` has
# something to show) into one fixture snapshot. Then base, base again and head
# each answer, one after the other (the questions are in rust-diff/questions.sh,
# the trees in rust-diff/tree.sh):
#   - on the fixture snapshot: ONCE and `<command> --help`, as written (the
#     binary's own texts); QUERIES, text and --json; TZ_QUERIES in another time
#     zone; TTY_QUERIES on a terminal; the serve phase (rust-diff/serve.sh);
#     REFRESHES on a copy of the snapshot (the tree has moved on since), then
#     AFTER_REFRESH on the copy. refresh needs no root while
#     FREESPACE_SNAPSHOT_DIR is a folder of ours;
#   - fresh: the build makes a tree of its own, scans it with 8 walker threads,
#     changes it and scans again, then QUERIES; moves it on, REFRESHES and
#     AFTER_REFRESH, all on the snapshot it made itself;
#   - config: a config file with two roots and no previous snapshot; `scan`
#     without a root, CONFIG_QUERIES; then a config with an unknown key;
#   - after each part, the files the CLI keeps (the snapshot folder, $HOME, the
#     unit dir): name, type and mode, and state.json normalized.
# No command may change a tree it reads: each tree is listed (names, types,
# modes, sizes, links, mtimes) before and after the commands that read it, and
# any change stops the job at once (exit 1).
# Base must answer every question with the exit status EXPECTED lists (0 when
# it lists none); otherwise the harness is stale (exit 4) before head runs: a
# question both builds refuse alike would test nothing.
# A call that runs past RUST_DIFF_TIMEOUT seconds (300) is stopped and answers
# exit 124, so a hang shows up as the question that hung.
# Answers are normalized (D20, rust-diff/normalize.sh) and compared byte for
# byte: base against base first (a difference there means the comparison
# itself is unreliable: exit 3), then base against head (exit 1 on any
# difference). A version bump alone is no difference: each binary's own version
# (from its --version) is masked in its answers. The normalized answers stay in
# <out-dir>/{base,base2,head}/{fixture,fresh,config}.txt.
# Needs jq, curl and script (util-linux). JSON is compared as jq prints it, keys
# in the CLI's order. RUST_DIFF_FAN (4) sets the wide part of the trees.
# shellcheck source-path=SCRIPTDIR
set -euo pipefail

usage() { echo "usage: $0 <base-bin> <head-bin> <out-dir>" >&2; exit 2; }
[ "$#" -eq 3 ] || usage
base=$1 head=$2 out=$3
for b in "$base" "$head"; do
  if [ ! -f "$b" ] || [ ! -x "$b" ]; then echo "$0: not an executable file: $b" >&2; exit 2; fi
done
for tool in jq curl script; do
  command -v "$tool" >/dev/null || { echo "$0: $tool is required" >&2; exit 2; }
done
here=$(dirname "$(realpath "$0")")
# shellcheck source=rust-diff/tree.sh
. "$here/rust-diff/tree.sh"
# shellcheck source=rust-diff/questions.sh
. "$here/rust-diff/questions.sh"
# shellcheck source=rust-diff/normalize.sh
. "$here/rust-diff/normalize.sh"
# shellcheck source=rust-diff/serve.sh
. "$here/rust-diff/serve.sh"
base=$(realpath "$base") head=$(realpath "$head")
mkdir -p "$out"
out=$(realpath "$out")

work=$(mktemp -d)
work_re=$(printf '%s' "$work" | sed 's/[][\.*^$#]/\\&/g')
# the trees hold a folder nobody may read
trap 'chmod -R u+rwx "$work" 2>/dev/null; rm -rf "$work"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
tree=$work/tree

# Each build runs as .../freespace: clap writes argv[0] into usage lines
# ("Usage: base ls"), and the two builds sit in files with different names.
mkdir -p "$work/bin/base" "$work/bin/head"
ln -s "$base" "$work/bin/base/freespace"
ln -s "$head" "$work/bin/head/freespace"
base=$work/bin/base/freespace head=$work/bin/head/freespace

# The CLI's environment: this host's config, units, $HOME and XDG folders stay
# out; $tz and $config, when a phase sets them, replace UTC and the missing
# config file. doctor still reads the host (the systemd timer, an installed
# binary on PATH, the skill in the passwd home), alike for both builds on a
# clean CI runner; on a dev machine with freespace installed a local run can
# differ where CI's won't.
run_prefix() { # sets argv: the command line a call starts with
  argv=(env -u XDG_CONFIG_HOME -u XDG_STATE_HOME -u XDG_CACHE_HOME -u XDG_DATA_HOME -u XDG_RUNTIME_DIR
    TZ="${tz:-UTC}" LC_ALL=C HOME="$work/home" FREESPACE_CONFIG="${config:-$work/none.toml}"
    FREESPACE_UNIT_DIR="$work/units" timeout -k 10 "${RUST_DIFF_TIMEOUT:-300}")
}
run() { # <bin> <args...>
  local argv
  run_prefix
  "${argv[@]}" "$@"
}

# The floor: while $checking (base's first run), an exit status EXPECTED does not
# list for this phase and question goes to $work/stale.
checking=false phase=
floor() { # <phase> <question> <got> [<wanted, space-separated>]
  $checking || return 0
  local want=${4-} e
  if [ -z "$want" ]; then
    want=0
    for e in "${EXPECTED[@]}"; do
      [ "${e%|*}" != "$1|$2" ] || want=${e##*|}
    done
  fi
  case " $want " in *" $3 "*) ;; *) echo "$1: $2: answered $3, the harness expects $want" >>"$work/stale" ;; esac
}

ask() { # <bin> <fresh> <query words...>: one answer, normalized; no words: the bare run
  local bin=$1 fresh=$2 w args=() json=false rel=false code=0 q
  shift 2
  for w in "$@"; do
    args+=("${w/#@/$tree}")
    [ "$w" = --json ] && json=true
    [[ ! $w =~ ^[0-9]+(d|w|mo|y)$ ]] || rel=true
  done
  run "$bin" "${args[@]}" >"$work/stdout" 2>"$work/stderr" || code=$?
  # a scan and a refresh publish a new snapshot with a new id
  case "${1:-}" in scan | refresh) snap_id=$(current_id) ;; esac
  q="$*"
  floor "$phase" "${q% --json}" "$code"
  echo "== ${*:-(no arguments)}"
  echo "exit $code"
  for s in stdout stderr; do
    echo "-- $s"
    if $json; then normalize_json "$work/$s" "$fresh" "$rel"; else text_of "$work/$s"; fi
  done | mask
}

ask_all() { # <bin> <fresh> <queries...>: each, text then --json
  local bin=$1 fresh=$2 q words
  shift 2
  for q in "$@"; do
    read -ra words <<<"$q"
    ask "$bin" "$fresh" "${words[@]}"
    ask "$bin" "$fresh" "${words[@]}" --json
  done
}

ask_tty() { # <bin> <query>: the text a terminal shows, under script(1)
  local bin=$1 argv code=0 cmd w
  local -a words
  read -ra words <<<"$2"
  run_prefix
  printf -v cmd '%q ' "${argv[@]}" "$bin"
  for w in "${words[@]}"; do printf -v cmd '%s%q ' "$cmd" "${w/#@/$tree}"; done
  script -qec "$cmd" /dev/null </dev/null >"$work/stdout" 2>&1 || code=$?
  floor tty "$2" "$code"
  {
    echo "== on a terminal: ${2:-(no arguments)}"
    echo "exit $code"
    text_of "$work/stdout"
  } | mask
}

files() { # <dir>: a folder the CLI keeps, by name, type and mode; state.json in it, normalized
  {
    echo "== files in $1"
    (cd "$1" && find . -printf '%p %y %m\n') | LC_ALL=C sort
    if [ -f "$1/state.json" ]; then
      echo "-- state.json"
      normalize_json "$1/state.json" true
    fi
  } | mask
}

kept() { # <dir>: the snapshot folder, then $HOME and the unit dir, which no command should fill
  files "$1"
  files "$work/home"
  files "$work/units"
}

current_id() { # the snapshot id in $FREESPACE_SNAPSHOT_DIR, as base reads it
  run "$base" status --json 2>/dev/null | jq -r '.snapshot_id // empty' | grep -E '^[0-9A-Za-z_-]+$' || true
}

tree_mark() { tree_listing >"$work/listing"; }
tree_check() { # <bin> <what it did>: stop at once if the tree changed under it
  if ! tree_listing | diff -u --label before --label after "$work/listing" - >"$work/listing.diff"; then
    head -n 50 "$work/listing.diff" >&2
    echo "$0: $1 changed the tree it read ($tree) during $2" >&2
    exit 1
  fi
}

refreshes() { # <bin>: REFRESHES on the snapshot in FREESPACE_SNAPSHOT_DIR, then AFTER_REFRESH
  local q words
  phase=refresh
  # true: a refresh stores the root's live fs_used and fs_avail, as a scan does
  for q in "${REFRESHES[@]}"; do
    read -ra words <<<"$q"
    ask "$1" true "${words[@]}"
  done
  phase=after
  ask_all "$1" true "${AFTER_REFRESH[@]}"
}

fixture_answers() { # <bin>: what it says of the fixture snapshot and a copy it refreshes
  local q words tz
  FREESPACE_SNAPSHOT_DIR=$work/snap
  snap_id=$(current_id)
  tree_mark
  phase=once
  for q in "${ONCE[@]}"; do
    read -ra words <<<"$q"
    ask "$1" false "${words[@]}"
  done
  for q in "${COMMANDS[@]}"; do ask "$1" false "$q" --help; done
  phase=fixture
  ask_all "$1" false "${QUERIES[@]}"
  tz=$TZ_OTHER phase=tz
  echo "== TZ=$tz"
  ask_all "$1" false "${TZ_QUERIES[@]}"
  tz=
  for q in "${TTY_QUERIES[@]}"; do ask_tty "$1" "$q"; done
  serve_answers "$1"
  kept "$work/snap"
  # one path for every binary, so the answers read the same; a new copy for each
  rm -rf "$work/refreshed"
  cp -a "$work/snap" "$work/refreshed"
  FREESPACE_SNAPSHOT_DIR=$work/refreshed
  refreshes "$1"
  kept "$work/refreshed"
  tree_check "$1" "the fixture phase"
}

fresh_answers() { # <bin>: its own tree, scanned twice, asked, moved on and refreshed
  local tree=$work/ftree
  remove_tree
  make_tree
  rm -rf "$work/fresh"
  FREESPACE_SNAPSHOT_DIR=$work/fresh phase=fresh
  tree_mark
  ask "$1" true scan @ --workers 8 --json
  tree_check "$1" "its first scan"
  change_tree
  tree_mark
  ask "$1" true scan @ --workers 8
  ask_all "$1" true "${QUERIES[@]}"
  kept "$work/fresh"
  tree_check "$1" "its second scan and the questions"
  drift_tree
  tree_mark
  refreshes "$1"
  kept "$work/fresh"
  tree_check "$1" "its refreshes"
}

config_answers() { # <bin>: roots, workers and keep_previous from a config file; then a bad one
  local tree=$work/ftree config=$work/config.toml q words
  rm -rf "$work/conf"
  FREESPACE_SNAPSHOT_DIR=$work/conf phase=config
  printf 'roots = ["%s/projects", "%s/media"]\nkeep_previous = false\nworkers = 3\n' "$tree" "$tree" >"$config"
  tree_mark
  for q in "${CONFIG_QUERIES[@]}"; do
    read -ra words <<<"$q"
    ask "$1" true "${words[@]}"
  done
  kept "$work/conf"
  printf 'roots = ["%s"]\nkeep_old = true\n' "$tree" >"$config"
  phase=badconfig
  for q in "${BAD_CONFIG_QUERIES[@]}"; do
    read -ra words <<<"$q"
    ask "$1" true "${words[@]}"
  done
  kept "$work/conf"
  tree_check "$1" "the config phase"
}

answers() { # <bin> <label>: every phase, each into its own file
  local dir=$out/$2
  mkdir -p "$dir"
  export FREESPACE_SNAPSHOT_DIR=$work/snap
  ver_re=$(version_re "$1")
  fixture_answers "$1" >"$dir/fixture.txt"
  fresh_answers "$1" >"$dir/fresh.txt"
  config_answers "$1" >"$dir/config.txt"
}

same() { # <label-a> <label-b>: identical, or the differences on stdout
  local f ok=0
  for f in fixture fresh config; do
    diff -u --label "$1/$f.txt" --label "$2/$f.txt" "$out/$1/$f.txt" "$out/$2/$f.txt" | head -n 200 || ok=1
  done
  return "$ok"
}

mkdir -p "$work/home" "$work/units"
make_tree
for step in 1 2; do
  if ! FREESPACE_SNAPSHOT_DIR=$work/snap run "$base" scan "$tree" --workers 2 >"$work/scan.log" 2>&1; then
    cat "$work/scan.log" >&2
    echo "$0: base could not make the fixture snapshot (scan $step)" >&2
    exit 1
  fi
  [ "$step" = 2 ] || change_tree
done
drift_tree

checking=true
answers "$base" base
checking=false
if [ -s "$work/stale" ]; then
  cat "$work/stale" >&2
  echo "$0: base no longer answers these questions as the harness expects: the harness is stale" >&2
  exit 4
fi
answers "$base" base2
if ! same base base2; then
  echo "$0: base gives different answers on two runs: the comparison is unreliable" >&2
  exit 3
fi
answers "$head" head
if ! same base head; then
  echo "$0: head answers differently from base (normalized answers in $out)" >&2
  exit 1
fi
echo "$0: head answers ${#QUERIES[@]} queries x 2 forms x 2 snapshots, ${#TZ_QUERIES[@]} in another time zone," \
  "${#TTY_QUERIES[@]} on a terminal, $((${#ONCE[@]} + ${#COMMANDS[@]})) questions once, ${#SERVE_QUESTIONS[@]} + 6 requests" \
  "to serve, ${#REFRESHES[@]} refreshes x 2 snapshots and a config file exactly like base"
