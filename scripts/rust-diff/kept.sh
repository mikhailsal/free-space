# shellcheck shell=bash
# shellcheck disable=SC2154 # $work, $work_hex and the helpers are scripts/rust-diff.sh's
# What the CLI keeps, for scripts/rust-diff.sh, sourced by it: after each part of
# a run, the snapshot folder, $HOME and the unit dir by name, type and mode (a
# symlink with its target), state.json normalized, each other file as it is, and
# each database whole: its schema, its user_version and every row of every
# table, SQLite's statistics too.

# The columns a scan fills from the clock, the disk or the run (table.column):
# each shows as its type ("<integer>"), so a value that turns NULL still shows.
# Their order is lost too (a scan stamps each folder as it walks it, so two runs
# differ): core/tests/queries.rs checks which rows a refresh stamps.
# A hard link's inode shows as the first entry that has it: the fresh phase
# makes its tree anew, inodes and all, but the links that share an inode must
# still share it.
DB_LIVE=(meta.scanned_at meta.snapshot_id meta.scan_seconds entries.scanned_at entries.min_scanned_at
  roots.fs_used roots.fs_avail roots.inodes_used mounts.fs_used refreshes.scanned_at)
DB_INODE="'<the inode of entry ' || coalesce((SELECT min(o.entry_id) FROM hardlinks AS o WHERE o.dev = hardlinks.dev AND o.ino = hardlinks.ino), 'none') || '>'"

# A database row by row, read-only (a rollback journal: opening it writes
# nothing): by table name, each table's row count, then its rows as `table:
# column=value ...` in rowid order (the key's, where the key is an INTEGER
# PRIMARY KEY), generated columns too; a value as SQLite quotes it, a BLOB
# (every path) in hex, the work dir in it masked; the version that wrote it
# masked (the fixture snapshot is base's, also when head reads it). Names are
# quoted as SQL quotes them, whatever they hold. A database sqlite3 cannot read
# says so.
db_of() { # <db>
  local db=$1 cols table col last='' row='' sql='' id text dq='"' sq="'" from=
  echo "-- ${db##*/}"
  if ! cols=$(sqlite3 -readonly "$db" "SELECT m.name || char(31) || p.name FROM sqlite_master AS m, pragma_table_xinfo(m.name) AS p
      WHERE m.type = 'table' ORDER BY m.name, p.cid" 2>&1); then
    echo "(sqlite3 cannot read it: $cols)"
    return 0
  fi
  sqlite3 -readonly "$db" '.schema' "SELECT 'user_version ' || user_version FROM pragma_user_version"
  while IFS=$'\x1f' read -r table col; do
    [ -n "$table" ] || continue
    if [ "$table" != "$last" ]; then
      [ -z "$last" ] || sql+="$row FROM $from ORDER BY rowid;"$'\n'
      printf -v from '"%s"' "${table//$dq/$dq$dq}"
      printf -v text "'%s'" "${table//$sq/$sq$sq}"
      sql+="SELECT $text || ' rows ' || count(*) FROM $from;"$'\n'
      row="SELECT $text || ':'" last=$table
    fi
    printf -v id '"%s"' "${col//$dq/$dq$dq}"
    printf -v text "' %s='" "${col//$sq/$sq$sq}"
    if [ "$table.$col" = hardlinks.ino ]; then
      row+=" || $text || $DB_INODE"
    elif [[ " ${DB_LIVE[*]} " == *" $table.$col "* ]]; then
      row+=" || $text || '<' || typeof($id) || '>'"
    else
      row+=" || $text || quote($id)"
    fi
  done <<<"$cols"
  [ -z "$last" ] || sql+="$row FROM $from ORDER BY rowid;"
  sqlite3 -readonly "$db" "$sql" |
    sed -E -e "s/$work_hex/<work>/g" -e "s/ freespace_version='[0-9]+(\.[0-9]+)+([-+][0-9A-Za-z.+-]+)?'/ freespace_version='<version>'/"
}

files() { # <dir>: a folder the CLI keeps, by name, type and mode; state.json in it, normalized; other files; its databases
  local db f
  {
    echo "== files in $1"
    if [ ! -d "$1" ]; then
      echo "(missing)"
    else
      (cd "$1" && find . \( -type l -printf '%p %y %m -> %l\n' \) -o -printf '%p %y %m\n') | LC_ALL=C sort
      # the other files as they are (a lock a scan leaves, gui's redirect file), in the order of their names; one
      # with a NUL in it is no text: its size and checksum stand for it
      (cd "$1" && find . -type f ! -name state.json ! -name '*.db' -print0) | LC_ALL=C sort -z |
        while IFS= read -r -d '' f; do
          echo "-- ${f#./}"
          if [ "$(tr -d '\000' <"$1/$f" | wc -c)" = "$(wc -c <"$1/$f")" ]; then
            text_of "$1/$f"
          else
            echo "(binary: $(wc -c <"$1/$f") bytes, sha256 $(sha256sum <"$1/$f" | cut -d ' ' -f 1))"
          fi
        done
    fi
    if [ -f "$1/state.json" ]; then
      echo "-- state.json"
      normalize_json "$1/state.json" true
    fi
    for db in "$1"/*.db; do [ ! -f "$db" ] || db_of "$db"; done
  } | mask
}

kept() { # <dir>: the snapshot folder, then $HOME and the unit dir, which no command should fill
  files "$1"
  files "$work/home"
  files "$work/units"
}
