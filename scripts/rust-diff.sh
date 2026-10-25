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
#     binary's own texts); QUERIES and GLOB_QUERIES, text and --json;
#     CWD_QUERIES from inside the tree and PATH_QUERIES as given; TZ_QUERIES in
#     another time zone; TTY_QUERIES on a terminal; the serve phase
#     (rust-diff/serve.sh) and the gui phase (rust-diff/gui.sh); REFRESHES on a
#     copy of the snapshot (the tree has moved on since), then AFTER_REFRESH on
#     the copy. refresh needs no root while FREESPACE_SNAPSHOT_DIR is a folder of
#     ours;
#   - start: EMPTY_QUERIES with no snapshot folder, FIRST_QUERIES with an empty
#     one while a scan holds its lock, BUSY_QUERIES on a copy of the fixture
#     snapshot while a scan holds its lock;
#   - fresh: the build makes a tree of its own, scans it with 8 walker threads,
#     changes it and scans again, then QUERIES; moves it on, REFRESHES and
#     AFTER_REFRESH, all on the snapshot it made itself;
#   - config: a config file with two roots and no previous snapshot; `scan`
#     without a root, CONFIG_QUERIES; GONE_QUERIES once one root is gone from
#     the disk; then a config with an unknown key;
#   - after each part, the files the CLI keeps (the snapshot folder, $HOME, the
#     unit dir): name, type and mode, state.json normalized, and each database
#     row by row (rust-diff/kept.sh).
# Each run starts from the same files: a copy of the fixture snapshot, an empty
# $HOME and unit dir.
# No command may change a tree it reads: each tree is listed (names, types,
# modes, sizes, links, mtimes) before and after the commands that read it, and
# any change stops the job at once (exit 1).
# Base must answer every question with the exit status EXPECTED lists (0 when
# it lists none), a refusal with the words it lists, and --json with JSON;
# otherwise the harness is stale (exit 4) before head runs: a question both
# builds refuse alike would test nothing.
# Not as root: install-timer would install the build on this host.
# A call that runs past RUST_DIFF_TIMEOUT seconds (300) is stopped and answers
# exit 124, so a hang shows up as the question that hung.
# Answers are normalized (D20, rust-diff/normalize.sh) and compared byte for
# byte: base against base first (a difference there means the comparison
# itself is unreliable: exit 3), then base against head (exit 1 on any
# difference). A command of the harness's own that fails is a harness error,
# not a difference: exit 5, naming the command. A version bump alone is no
# difference: each binary's own version (from its --version) is masked in its
# answers. The normalized answers stay in
# <out-dir>/{base,base2,head}/{fixture,start,fresh,config}.txt.
# Needs jq, curl, script and flock (util-linux), perl, sqlite3 and tzdata. JSON
# is compared as jq prints it, keys in the CLI's order. RUST_DIFF_FAN (4, from
# 1 to 9) sets the wide part of the trees.
# shellcheck source-path=SCRIPTDIR
set -Eeuo pipefail
# -E: in a function, a subshell or a $(...) too; a subshell's exit 5 then fails its caller, which names itself as well
trap 'echo "$0: harness error: ${BASH_SOURCE[0]##*/} line $LINENO: $BASH_COMMAND exited $?" >&2; exit 5' ERR

usage() { echo "usage: $0 <base-bin> <head-bin> <out-dir>" >&2; exit 2; }
[ "$#" -eq 3 ] || usage
base=$1 head=$2 out=$3
for b in "$base" "$head"; do
  if [ ! -f "$b" ] || [ ! -x "$b" ]; then echo "$0: not an executable file: $b" >&2; exit 2; fi
done
if [[ ! ${RUST_DIFF_FAN:-4} =~ ^[1-9]$ ]]; then echo "$0: RUST_DIFF_FAN must be 1 to 9, not ${RUST_DIFF_FAN}" >&2; exit 2; fi
for tool in jq curl script flock perl sqlite3; do
  command -v "$tool" >/dev/null || { echo "$0: $tool is required" >&2; exit 2; }
done
if [ "$(id -u)" = 0 ]; then echo "$0: run it as a user, not root: install-timer would install a build on this host" >&2; exit 2; fi
here=$(dirname "$(realpath "$0")")
# shellcheck source=rust-diff/tree.sh
. "$here/rust-diff/tree.sh"
# shellcheck source=rust-diff/questions.sh
. "$here/rust-diff/questions.sh"
# shellcheck source=rust-diff/normalize.sh
. "$here/rust-diff/normalize.sh"
# shellcheck source=rust-diff/serve.sh
. "$here/rust-diff/serve.sh"
# shellcheck source=rust-diff/gui.sh
. "$here/rust-diff/gui.sh"
# shellcheck source=rust-diff/kept.sh
. "$here/rust-diff/kept.sh"
# without the zone (no tzdata) the C library falls back to UTC, and TZ_QUERIES test nothing
if [ "$(TZ=$TZ_OTHER date -d @1704067200 +%z)" != +0530 ]; then
  echo "$0: tzdata is required: TZ=$TZ_OTHER must be UTC+05:30" >&2
  exit 2
fi
base=$(realpath "$base") head=$(realpath "$head")
mkdir -p "$out"
out=$(realpath "$out")

# absolute, also from a relative TMPDIR: the cwd questions run from inside the tree
work=$(realpath "$(mktemp -d)")
work_re=$(printf '%s' "$work" | sed 's/[][\.*^$#]/\\&/g')
# as SQLite prints the paths a database holds (rust-diff/kept.sh)
work_hex=$(printf '%s' "$work" | od -An -v -tx1 | tr -d ' \n' | tr a-f A-F)
# the dates tree.sh gives by age, the same for every tree of the run
epoch=$(date +%s)
# a server still running (rust-diff/serve.sh) goes too; the trees hold a folder nobody may read
serving=
# (no ERR trap there: the job's own exit status stands)
trap 'trap - ERR; [ -z "$serving" ] || kill "$serving" 2>/dev/null || true; chmod -R u+rwx "$work" 2>/dev/null || true; rm -rf "$work"' EXIT
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
# out, and so do its display and its terminal (stdin is /dev/null: given a
# terminal, `gui` would start, not refuse); $tz and $config, when a phase sets
# them, replace UTC and the missing config file. doctor still reads the host
# (the systemd timer, an installed binary on PATH, the skill in the passwd
# home), alike for both builds on a clean CI runner; on a dev machine with
# freespace installed a local run can differ where CI's won't. timeout stays in
# the job's process group (--foreground), so a Ctrl-C reaches the call in
# progress too.
run_prefix() { # sets argv: the command line a call starts with
  argv=(env -u XDG_CONFIG_HOME -u XDG_STATE_HOME -u XDG_CACHE_HOME -u XDG_DATA_HOME -u XDG_RUNTIME_DIR -u DISPLAY -u WAYLAND_DISPLAY
    TZ="${tz:-UTC}" LC_ALL=C HOME="$work/home" FREESPACE_CONFIG="${config:-$work/none.toml}"
    FREESPACE_UNIT_DIR="$work/units" timeout --foreground -k 10 "${RUST_DIFF_TIMEOUT:-300}")
}
run() { # <bin> <args...>
  local argv
  run_prefix
  "${argv[@]}" "$@" </dev/null
}

# The floor: while $checking (base's first run), an exit status EXPECTED does not
# list for this phase and question goes to $work/stale, and so does a status it
# lists given without the words EXPECTED says stderr ($work/stderr) holds then.
checking=false phase=
floor() { # <phase> <question> <got> [<wanted, space-separated>]
  $checking || return 0
  local want=${4-} says='' e
  if [ -z "$want" ]; then
    want=0
    for e in "${EXPECTED[@]}"; do
      case $e in "$1|$2|"*) e=${e#"$1|$2|"} want=${e%%|*} says=${e#*|} ;; esac
    done
  fi
  case " $want " in
    *" $3 "*) [ -z "$says" ] || grep -qF -- "$says" "$work/stderr" ||
      echo "$1: $2: answered $3 without \"$says\" on stderr" >>"$work/stale" ;;
    *) echo "$1: $2: answered $3, the harness expects $want" >>"$work/stale" ;;
  esac
}

ask() { # <bin> <fresh> <query words...>: one answer, normalized; no words: the bare run
  local bin=$1 fresh=$2 w args=() json=false rel=false code=0 q s
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
  # JSON: one value at least (blank output is none)
  if $checking && $json && [ "$code" = 0 ] && ! jq -en '[inputs] | length > 0' "$work/stdout" >/dev/null 2>&1; then
    echo "$phase: $q: answered 0 without JSON" >>"$work/stale"
  fi
  echo "== ${*:-(no arguments)}"
  echo "exit $code"
  # an age bound's date in text is today's less the age, so it moves at midnight
  for s in stdout stderr; do
    echo "-- $s"
    if $json; then normalize_json "$work/$s" "$fresh" "$rel"; else text_of "$work/$s"; fi
  done | if $rel; then sed -E 's/ since [0-9]{4}-[0-9]{2}-[0-9]{2}\b/ since <date>/g'; else cat; fi | mask
}

ask_each() { # <bin> <fresh> <queries...>: each as written
  local bin=$1 fresh=$2 q words
  shift 2
  for q in "$@"; do
    read -ra words <<<"$q"
    ask "$bin" "$fresh" "${words[@]}"
  done
}

ask_all() { # <bin> <fresh> <queries...>: each, text then --json
  local bin=$1 fresh=$2 q
  shift 2
  for q in "$@"; do ask_each "$bin" "$fresh" "$q" "$q --json"; done
}

ask_tty() { # <bin> <query>: the text a terminal shows, under script(1)
  local bin=$1 argv code=0 cmd w
  local -a words
  read -ra words <<<"$2"
  run_prefix
  printf -v cmd '%q ' "${argv[@]}" "$bin"
  for w in "${words[@]}"; do printf -v cmd '%s%q ' "$cmd" "${w/#@/$tree}"; done
  # the terminal passes line ends as written (no CR added): a CR the CLI writes shows
  script -qec "stty -onlcr; $cmd" /dev/null </dev/null >"$work/stdout" 2>&1 || code=$?
  # the floor looks for its words in $work/stderr: on a terminal both streams are one
  cp "$work/stdout" "$work/stderr"
  floor tty "$2" "$code"
  {
    echo "== on a terminal: ${2:-(no arguments)}"
    echo "exit $code"
    text_of "$work/stdout"
  } | mask
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
  phase=refresh
  # true: a refresh stores the root's live fs_used and fs_avail, as a scan does
  ask_each "$1" true "${REFRESHES[@]}"
  phase=after
  ask_all "$1" true "${AFTER_REFRESH[@]}"
}

paths_answers() { # <bin>: CWD_QUERIES from inside the tree, PATH_QUERIES, and the tree's odd/ as --path-raw hex
  local hex
  phase=cwd
  echo "== in @/projects"
  (cd "$tree/projects" && ask_all "$1" false "${CWD_QUERIES[@]}")
  phase=paths
  hex=$(printf '%s' "$tree/odd" | od -An -v -tx1 | tr -d ' \n')
  ask_all "$1" false "${PATH_QUERIES[@]}" "ls --path-raw $hex"
}

fixture_answers() { # <bin>: what it says of the fixture snapshot and a copy it refreshes
  local q tz
  FREESPACE_SNAPSHOT_DIR=$work/snap
  snap_id=$(current_id)
  tree_mark
  phase=once
  ask_each "$1" false "${ONCE[@]}" "${COMMANDS[@]/%/ --help}"
  phase=fixture
  ask_all "$1" false "${QUERIES[@]}"
  phase=glob
  ask_all "$1" false "${GLOB_QUERIES[@]}"
  paths_answers "$1"
  tz=$TZ_OTHER phase=tz
  echo "== TZ=$tz"
  ask_all "$1" false "${TZ_QUERIES[@]}"
  tz=
  for q in "${TTY_QUERIES[@]}"; do ask_tty "$1" "$q"; done
  serve_answers "$1"
  gui_answers "$1"
  kept "$work/snap"
  # one path for every binary, so the answers read the same; a new copy for each
  rm -rf "$work/refreshed"
  cp -a "$work/pristine" "$work/refreshed"
  FREESPACE_SNAPSHOT_DIR=$work/refreshed
  refreshes "$1"
  kept "$work/refreshed"
  tree_check "$1" "the fixture phase"
}

hold_lock() { # <dir>: a scan of $tree runs, as its lock says (at a fixed time, 2024-01-01 00:00 UTC); the fd in $locked
  mkdir -p "$1"
  printf '{"op":"scan","path":"%s","started_at":1704067200,"pid":4242}' "$tree" >"$1/.lock"
  exec {locked}>>"$1/.lock"
  flock -x "$locked"
}

start_answers() { # <bin>: before the first snapshot, during the first scan, and during a scan over the fixture snapshot
  local locked
  rm -rf "$work/empty" "$work/first" "$work/busy"
  tree_mark
  FREESPACE_SNAPSHOT_DIR=$work/empty phase=empty snap_id=
  ask_all "$1" false "${EMPTY_QUERIES[@]}"
  kept "$work/empty"
  FREESPACE_SNAPSHOT_DIR=$work/first phase=first
  hold_lock "$work/first"
  ask_all "$1" false "${FIRST_QUERIES[@]}"
  exec {locked}>&-
  kept "$work/first"
  cp -a "$work/pristine" "$work/busy"
  FREESPACE_SNAPSHOT_DIR=$work/busy phase=busy
  snap_id=$(current_id)
  hold_lock "$work/busy"
  ask_all "$1" false "${BUSY_QUERIES[@]}"
  exec {locked}>&-
  kept "$work/busy"
  tree_check "$1" "the start phase"
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

config_answers() { # <bin>: roots, workers and keep_previous from a config file; a root gone; then a bad config; in the fresh phase's tree
  local tree=$work/ftree config=$work/config.toml
  [ -d "$tree" ] || { echo "$0: the config phase needs the tree of the fresh phase, and there is none" >&2; exit 1; }
  rm -rf "$work/conf"
  FREESPACE_SNAPSHOT_DIR=$work/conf phase=config
  printf 'roots = ["%s/projects", "%s/media"]\nkeep_previous = false\nworkers = 3\n' "$tree" "$tree" >"$config"
  tree_mark
  ask_each "$1" true "${CONFIG_QUERIES[@]}"
  kept "$work/conf"
  tree_check "$1" "the config phase"
  # the last questions of the run in this tree: the next run's fresh phase makes it anew
  rm -rf "${tree:?}/media"
  tree_mark
  phase=gone
  ask_each "$1" true "${GONE_QUERIES[@]}"
  kept "$work/conf"
  printf 'roots = ["%s"]\nkeep_old = true\n' "$tree" >"$config"
  phase=badconfig
  ask_each "$1" true "${BAD_CONFIG_QUERIES[@]}"
  kept "$work/conf"
  tree_check "$1" "the config phase"
}

PHASES=(fixture start fresh config)
answers() { # <bin> <label>: every phase, each into its own file, from the same files as every other run
  local dir=$out/$2
  mkdir -p "$dir"
  rm -rf "${work:?}/snap" "${work:?}/home" "${work:?}/units"
  cp -a "$work/pristine" "$work/snap"
  mkdir -p "$work/home" "$work/units"
  export FREESPACE_SNAPSHOT_DIR=$work/snap
  ver_re=$(version_re "$1")
  fixture_answers "$1" >"$dir/fixture.txt"
  start_answers "$1" >"$dir/start.txt"
  fresh_answers "$1" >"$dir/fresh.txt"
  config_answers "$1" >"$dir/config.txt"
}

same() { # <label-a> <label-b>: identical, or the differences on stdout
  local f ok=0
  for f in "${PHASES[@]}"; do
    diff -u --label "$1/$f.txt" --label "$2/$f.txt" "$out/$1/$f.txt" "$out/$2/$f.txt" | head -n 200 || ok=1
  done
  return "$ok"
}

mkdir -p "$work/home" "$work/units"
make_tree
for step in 1 2; do
  if ! FREESPACE_SNAPSHOT_DIR=$work/pristine run "$base" scan "$tree" --workers 2 >"$work/scan.log" 2>&1; then
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
echo "$0: head answers ${#QUERIES[@]} queries x 2 forms x 2 snapshots, ${#GLOB_QUERIES[@]} globs and" \
  "$((${#CWD_QUERIES[@]} + ${#PATH_QUERIES[@]} + 1)) paths as typed x 2 forms, ${#TZ_QUERIES[@]} in another time zone," \
  "${#TTY_QUERIES[@]} on a terminal, $((${#ONCE[@]} + ${#COMMANDS[@]})) questions once, $served requests to serve and gui," \
  "$((${#EMPTY_QUERIES[@]} + ${#FIRST_QUERIES[@]} + ${#BUSY_QUERIES[@]})) x 2 forms before and during a scan," \
  "${#REFRESHES[@]} refreshes x 2 snapshots and a config file whose root goes, exactly like base"
