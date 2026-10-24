#!/usr/bin/env bash
# The rust-diff job of .github/workflows/refactor-guard.yml (D15, D20, D23):
# two builds of the CLI must give the same answers to the same questions.
#
#   scripts/rust-diff.sh <base-bin> <head-bin> <out-dir>
#
# Base scans a made-up tree twice (with changes in between, so `diff` has
# something to show) into one fixture snapshot. Then base, base again and head
# each answer, one after the other:
#   - QUERIES on that snapshot, text and --json;
#   - ONCE and `<command> --help`, as written: the binary's own texts (version,
#     help, completions, schema, the skill, usage errors, the bare run);
#   - REFRESHES on a copy of that snapshot (the tree has moved on since: one
#     folder changed, one gone), then AFTER_REFRESH on the copy. refresh needs
#     no root while FREESPACE_SNAPSHOT_DIR is a folder of ours; this is the only
#     way a refactor of core/src/refresh.rs shows up in an answer;
#   - QUERIES again on a fresh scan of the tree the binary makes itself;
#   - after each of the three, the files in that snapshot folder: name, type and
#     mode, so a stray file or a wider mode is a difference too.
# A call that runs past RUST_DIFF_TIMEOUT seconds (300) is stopped and answers
# exit 124, so a hang shows up as the question that hung.
# Answers are normalized (D20, see normalize_json and mask) and compared byte
# for byte: base against base first (a difference there means the comparison
# itself is unreliable: exit 3), then base against head (exit 1 on any
# difference). A version bump alone is no difference: each binary's own version
# (from its --version) is masked in its answers. The normalized answers stay in
# <out-dir>/{base,base2,head}/{fixture,fresh}.txt.
# Needs jq. JSON is compared as jq prints it, keys in the CLI's order.
set -euo pipefail

usage() { echo "usage: $0 <base-bin> <head-bin> <out-dir>" >&2; exit 2; }
[ "$#" -eq 3 ] || usage
base=$1 head=$2 out=$3
for b in "$base" "$head"; do
  if [ ! -f "$b" ] || [ ! -x "$b" ]; then echo "$0: not an executable file: $b" >&2; exit 2; fi
done
command -v jq >/dev/null || { echo "$0: jq is required" >&2; exit 2; }
base=$(realpath "$base") head=$(realpath "$head")
mkdir -p "$out"
out=$(realpath "$out")

work=$(mktemp -d)
# chmod first: the tree holds a folder nobody may read
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

# Every question, run once as written and once with --json; @ is the tree.
# Ages are absolute dates, so the answers do not move with the clock.
QUERIES=(
  "status"
  "doctor"
  "ls @"
  "ls @ --sort apparent"
  "ls @ --sort mtime --limit 3"
  "ls @/projects/app"
  "ls @/odd"
  "ls @/nope"
  "ls @ --snapshot not-this-one"
  "top @"
  "top @ --min-size 100K --sort mtime"
  "top @ --older-than 2024-06-01"
  "top @ --kind dir"
  "top @ --kind dir --older-than 2024-06-01 --limit 2"
  "find @ --name *.iso"
  "find @ --iname NODE_MODULES --kind dir"
  "find @ --newer-than 2024-06-01 --min-size 10K --kind file"
  "find @ --name *.log --limit 1"
  "diff @"
  "diff @ --depth 3 --kind any"
  "diff @ --kind file --limit 2"
)

# Questions asked once, as written, on the fixture snapshot: a --json twin would
# mean nothing for texts, and the snapshot makes no difference to them (the
# bare run reads it, so it is here). The empty one is the bare run; --json is
# its summary. A usage error has a --json form of its own, listed too.
ONCE=(
  ""
  "--json"
  "--version"
  "--help"
  "ls --no-such-flag"
  "ls --no-such-flag --json"
  "completions bash"
  "completions zsh"
  "completions fish"
  "schema"
  "skill print"
)
# `<command> --help` is asked of each of these (the list in `freespace --help`).
COMMANDS=(ls top find diff gui serve status doctor scan refresh install-timer skill schema completions)

# refresh, in this order, on a copy of the fixture snapshot, after drift_tree:
# a changed folder (text), a folder gone from the disk (--json), a path the
# snapshot does not have and a file (both refused). AFTER_REFRESH, text and
# --json, then ask what the copy holds. None of it needs root.
REFRESHES=(
  "refresh @/logs --workers 2"
  "refresh @/cache --workers 2 --json"
  "refresh @/nope --workers 2"
  "refresh @/top.bin --workers 2 --json"
)
AFTER_REFRESH=(
  "status"
  "ls @"
  "ls @/logs"
  "top @"
  "top @ --kind dir"
  "find @ --name *.log"
  "diff @"
)

# The CLI's environment: this host's config, units and $HOME stay out. doctor
# still reads the host (the systemd timer, an installed binary on PATH, the
# skill in the passwd home), alike for both builds on a clean CI runner; on a
# dev machine with freespace installed a local run can differ where CI's won't.
run() { # <bin> <args...>
  env TZ=UTC LC_ALL=C HOME="$work/home" FREESPACE_CONFIG="$work/none.toml" \
    FREESPACE_UNIT_DIR="$work/units" timeout -k 10 "${RUST_DIFF_TIMEOUT:-300}" "$@"
}

blob() { # <path> <KiB> <mtime>: a file of that size, modified then
  mkdir -p "$(dirname "$1")"
  head -c "$(($2 * 1024))" /dev/zero >"$1"
  touch -d "$3" "$1"
}

settle_dirs() { # every folder's own mtime, after its files changed
  chmod 700 "$tree/locked" # find must enter it
  find "$tree" -depth -type d -exec touch -d "2024-03-01 00:00:00 UTC" {} +
  chmod 000 "$tree/locked"
}

make_tree() {
  blob "$tree/projects/app/src/main.rs" 12 "2024-09-10 08:00 UTC"
  blob "$tree/projects/app/node_modules/pkg/index.js" 64 "2024-02-01 08:00 UTC"
  blob "$tree/projects/old-site/node_modules/lib/big.js" 180 "2023-05-05 08:00 UTC"
  blob "$tree/projects/old-site/index.html" 8 "2023-05-05 08:00 UTC"
  blob "$tree/media/iso/debian.iso" 400 "2024-01-20 08:00 UTC"
  ln "$tree/media/iso/debian.iso" "$tree/media/iso/debian-copy.iso"
  truncate -s 50M "$tree/media/sparse.img"
  touch -d "2024-07-01 08:00 UTC" "$tree/media/sparse.img"
  blob "$tree/logs/app.log" 40 "2024-10-01 08:00 UTC"
  blob "$tree/logs/old.log" 16 "2022-12-31 08:00 UTC"
  blob "$tree/top.bin" 100 "2021-03-04 08:00 UTC"
  blob "$tree/odd/$(printf 'bad\377name.bin')" 4 "2024-04-04 08:00 UTC"
  ln -s ../media "$tree/projects/media-link"
  mkdir -p "$tree/empty" "$tree/locked/inside"
  blob "$tree/locked/inside/secret.bin" 8 "2024-05-05 08:00 UTC"
  settle_dirs
}

change_tree() { # what the second scan sees: growth, a new file, a removal
  blob "$tree/media/iso/ubuntu.iso" 300 "2024-10-20 08:00 UTC"
  blob "$tree/logs/app.log" 90 "2024-10-21 08:00 UTC"
  blob "$tree/cache/blob.bin" 32 "2024-10-22 08:00 UTC"
  rm -r "$tree/projects/old-site"
  settle_dirs
}

drift_tree() { # what refresh finds, after both scans: a folder changed, one gone
  blob "$tree/logs/app.log" 120 "2024-10-23 08:00 UTC"
  blob "$tree/logs/new.log" 24 "2024-10-23 09:00 UTC"
  rm "$tree/logs/old.log"
  rm -r "$tree/cache"
  settle_dirs
}

# D20: drop snapshot_id, every time field and live filesystem numbers. A fresh
# scan or a refresh stores its root's fs_used and fs_avail at that time, so
# there they are live too ($fresh is true). ($fresh is jq's, not the shell's.)
# shellcheck disable=SC2016
JQ_NORM='walk(if type == "object" then
  with_entries(select(.key | test("^(snapshot_id|scanned_at|prev_scanned_at)$|_ts$|_seconds$") | not))
  | (if (.fs | type) == "array" then .fs |= map(del(.total, .used, .avail)) else . end)
  | (if $fresh and (.roots | type) == "array"
     then .roots |= map(if type == "object" then del(.fs_used, .fs_avail) else . end) else . end)
else . end)'

normalize_json() { # <file> <fresh>: the JSON normalized, or the text as is
  jq --argjson fresh "$2" "$JQ_NORM" "$1" 2>/dev/null || cat "$1"
}

# The same values in text: the snapshot id, ages, clock times, live free and
# used space (not the space the next scan needs: doctor works it out from the
# snapshot), the scan's durations; the work dir, so the answers read the same
# anywhere; and the binary's own version, which a release bump changes (the
# exact version, as a word, not any number that looks like one).
snap_id=
ver_re=
work_re=$(printf '%s' "$work" | sed 's/[][\.*^$#]/\\&/g')
mask() {
  sed -E \
    -e "s#$work_re#<work>#g" \
    -e "${snap_id:+s/$snap_id/<snapshot_id>/g}" \
    -e "${ver_re:+s/\\<$ver_re\\>/<version>/g}" \
    -e 's/[0-9]+ (s|min|h|d) old/<age> old/g' \
    -e 's/[0-9]{4}-[0-9]{2}-[0-9]{2}[ T][0-9]{2}:[0-9]{2}(:[0-9]{2})?/<time>/g' \
    -e '/needs [0-9]+(\.[0-9])? [BKMGTPE] free/!s/[0-9]+(\.[0-9])? [BKMGTPE] free/<size> free/g' \
    -e 's/[0-9]+(\.[0-9])? [BKMGTPE] available/<size> available/g' \
    -e 's/used of [0-9]+(\.[0-9])? [BKMGTPE]/used of <size>/g' \
    -e 's/filesystem [0-9]+(\.[0-9])? [BKMGTPE] used/filesystem <size> used/g' \
    -e 's/[0-9]+\.[0-9]+ s\b/<seconds>/g'
}

version_re() { # <bin>: its version from its own --version, as a regex ("freespace 0.1.1" gives 0\.1\.1)
  local v
  v=$(run "$1" --version 2>/dev/null | sed -nE '1s/^.* ([0-9]+(\.[0-9]+)+([-+][0-9A-Za-z.+-]+)?)$/\1/p') || true
  printf '%s' "$v" | sed 's/[.+]/\\&/g'
}

ask() { # <bin> <fresh> <query words...>: one answer, normalized; no words: the bare run
  local bin=$1 fresh=$2 w args=() json=false code=0
  shift 2
  for w in "$@"; do
    args+=("${w/#@/$tree}")
    [ "$w" = --json ] && json=true
  done
  run "$bin" "${args[@]}" >"$work/stdout" 2>"$work/stderr" || code=$?
  # a scan and a refresh publish a new snapshot with a new id
  case "${1:-}" in scan | refresh) snap_id=$(current_id) ;; esac
  echo "== ${*:-(no arguments)}"
  echo "exit $code"
  for s in stdout stderr; do
    echo "-- $s"
    if $json; then normalize_json "$work/$s" "$fresh"; else cat "$work/$s"; fi
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

ask_once() { # <bin>: the questions asked as written, no --json twin
  local q words c
  for q in "${ONCE[@]}"; do
    read -ra words <<<"$q"
    ask "$1" false "${words[@]}"
  done
  for c in "${COMMANDS[@]}"; do ask "$1" false "$c" --help; done
}

ask_refreshed() { # <bin>: refresh on a copy of the fixture snapshot, then ask the copy
  local q words
  # one path for every binary, so the answers read the same; a new copy for each
  rm -rf "$work/refreshed"
  cp -a "$work/snap" "$work/refreshed"
  FREESPACE_SNAPSHOT_DIR=$work/refreshed
  # true: a refresh stores the root's live fs_used and fs_avail, as a scan does
  for q in "${REFRESHES[@]}"; do
    read -ra words <<<"$q"
    ask "$1" true "${words[@]}"
  done
  ask_all "$1" true "${AFTER_REFRESH[@]}"
  files "$work/refreshed"
}

files() { # <dir>: that snapshot folder and what the CLI keeps in it, by name, type and mode
  {
    echo "== files in $1"
    (cd "$1" && find . -printf '%p %y %m\n') | LC_ALL=C sort
  } | mask
}

current_id() { # the snapshot id in $FREESPACE_SNAPSHOT_DIR, as base reads it
  run "$base" status --json 2>/dev/null | jq -r '.snapshot_id // empty' | grep -E '^[0-9A-Za-z_-]+$' || true
}

answers() { # <bin> <label>: the fixture snapshot's answers, then a fresh scan's
  local dir=$out/$2
  mkdir -p "$dir"
  export FREESPACE_SNAPSHOT_DIR=$work/snap
  snap_id=$(current_id)
  ver_re=$(version_re "$1")
  {
    ask_once "$1"
    ask_all "$1" false "${QUERIES[@]}"
    files "$work/snap"
    ask_refreshed "$1"
  } >"$dir/fixture.txt"
  rm -rf "$work/fresh"
  FREESPACE_SNAPSHOT_DIR=$work/fresh
  {
    ask "$1" true scan @ --workers 2 --json
    ask "$1" true scan @ --workers 2
    ask_all "$1" true "${QUERIES[@]}"
    files "$work/fresh"
  } >"$dir/fresh.txt"
}

same() { # <label-a> <label-b>: identical, or the differences on stdout
  local f ok=0
  for f in fixture fresh; do
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

answers "$base" base
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
echo "$0: head answers ${#QUERIES[@]} queries x 2 forms x 2 snapshots, $((${#ONCE[@]} + ${#COMMANDS[@]})) questions once" \
  "and ${#REFRESHES[@]} refreshes exactly like base"
