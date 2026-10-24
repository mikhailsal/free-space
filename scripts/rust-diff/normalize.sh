# shellcheck shell=bash
# shellcheck disable=SC2154 # $work, $tree and the like are scripts/rust-diff.sh's
# How scripts/rust-diff.sh makes two answers comparable (D20), sourced by it:
# mask what changes from run to run (times, ids, live disk numbers, the
# server's port and token) and nothing else.

# D20: snapshot_id, every time field and live filesystem numbers keep their key,
# and their value becomes its type ("<number>"), so a key that comes or goes
# still shows. A fresh scan or a refresh stores its root's fs_used and fs_avail
# at that time, so there they are live too ($fresh is true). A bound given as an
# age (`1y`) is echoed back as now less that age ($rel is true): it becomes the
# age again, in whole days. ($fresh and $rel are jq's.)
# shellcheck disable=SC2016
JQ_NORM='def live: "<" + type + ">";
def live_keys($re): with_entries(if (.key | test($re)) then .value |= live else . end);
walk(if type == "object" then
  live_keys("^(snapshot_id|scanned_at|prev_scanned_at)$|_ts$|_seconds$")
  | (if (.fs | type) == "array" then .fs |= map(if type == "object" then live_keys("^(total|used|avail)$") else . end) else . end)
  | (if $fresh and (.roots | type) == "array"
     then .roots |= map(if type == "object" then live_keys("^fs_(used|avail)$") else . end) else . end)
  | (if $rel then with_entries(if (.key | test("^(older|newer)_than$")) and (.value | type) == "number"
     then .value |= ((now - .) / 86400 | round) else . end) else . end)
else . end)'

normalize_json() { # <file> <fresh> [<rel>]: the JSON normalized, or the text as is
  jq --argjson fresh "$2" --argjson rel "${3:-false}" "$JQ_NORM" "$1" 2>/dev/null || text_of "$1"
}

# A file as it is, and a line saying so when its last line has no newline: the
# next heading then starts a line of its own, and a lost newline still shows.
text_of() { # <file>
  cat "$1"
  [ -z "$(tail -c 1 "$1")" ] || printf '\n\\ no newline at end\n'
}

# The same values in text: the snapshot id, ages, clock times, live free and
# used space (not the space the next scan needs: doctor works it out from the
# snapshot), the scan's durations; the work dir, so the answers read the same
# anywhere; the server's port and token; a terminal's carriage returns; and the
# binary's own version, which a release bump changes (the exact version, as a
# word, not any number that looks like one).
snap_id=
ver_re=
mask() {
  sed -E \
    -e 's/\r$//' \
    -e "s#$work_re#<work>#g" \
    -e "${snap_id:+s/$snap_id/<snapshot_id>/g}" \
    -e "${ver_re:+s/\\<$ver_re\\>/<version>/g}" \
    -e 's/127\.0\.0\.1:[0-9]+/127.0.0.1:<port>/g' \
    -e 's/-L [0-9]+:127/-L <port>:127/g' \
    -e 's/t=[0-9a-f]{64}/t=<token>/g' \
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
