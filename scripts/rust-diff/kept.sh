# shellcheck shell=bash
# shellcheck disable=SC2154 # $work, $work_hex and the helpers are scripts/rust-diff.sh's
# What the CLI keeps, for scripts/rust-diff.sh, sourced by it: after each part of
# a run, the snapshot folder, $HOME and the unit dir by name, type and mode,
# state.json normalized, and each database whole: its schema, its user_version
# and every row of every table, SQLite's statistics too.

# The columns a scan fills from the clock, the disk or the run (table.column):
# each shows as its type ("<integer>"), so a value that turns NULL still shows.
# A hard link's inode shows as the first entry that has it: the fresh phase
# makes its tree anew, inodes and all, but the links that share an inode must
# still share it.
DB_LIVE=(meta.scanned_at meta.snapshot_id meta.scan_seconds entries.scanned_at entries.min_scanned_at
  roots.fs_used roots.fs_avail roots.inodes_used mounts.fs_used refreshes.scanned_at)
DB_INODE="'<the inode of entry ' || (SELECT min(o.entry_id) FROM hardlinks AS o WHERE o.dev = hardlinks.dev AND o.ino = hardlinks.ino) || '>'"

# A database row by row, read-only (a rollback journal: opening it writes
# nothing): by table name, each table's row count, then its rows as `table:
# column=value ...` in rowid order (the key's, where the key is an INTEGER
# PRIMARY KEY); a value as SQLite quotes it, a BLOB (every path) in hex, the
# work dir in it masked; the version that wrote it masked (the fixture snapshot
# is base's, also when head reads it). A database sqlite3 cannot read says so.
db_of() { # <db>
  local db=$1 cols table col last='' row='' sql=''
  echo "-- ${db##*/}"
  if ! cols=$(sqlite3 -readonly "$db" "SELECT m.name || '|' || p.name FROM sqlite_master AS m, pragma_table_info(m.name) AS p
      WHERE m.type = 'table' ORDER BY m.name, p.cid" 2>&1); then
    echo "(sqlite3 cannot read it: $cols)"
    return 0
  fi
  sqlite3 -readonly "$db" '.schema' "SELECT 'user_version ' || user_version FROM pragma_user_version"
  while IFS='|' read -r table col; do
    [ -n "$table" ] || continue
    if [ "$table" != "$last" ]; then
      [ -z "$last" ] || sql+="$row FROM \"$last\" ORDER BY rowid;"$'\n'
      sql+="SELECT '$table rows ' || count(*) FROM \"$table\";"$'\n'
      row="SELECT '$table:'" last=$table
    fi
    if [ "$table.$col" = hardlinks.ino ]; then
      row+=" || ' $col=' || $DB_INODE"
    elif [[ " ${DB_LIVE[*]} " == *" $table.$col "* ]]; then
      row+=" || ' $col=<' || typeof(\"$col\") || '>'"
    else
      row+=" || ' $col=' || quote(\"$col\")"
    fi
  done <<<"$cols"
  [ -z "$last" ] || sql+="$row FROM \"$last\" ORDER BY rowid;"
  sqlite3 -readonly "$db" "$sql" |
    sed -E -e "s/$work_hex/<work>/g" -e "s/ freespace_version='[0-9]+(\.[0-9]+)+([-+][0-9A-Za-z.+-]+)?'/ freespace_version='<version>'/"
}

files() { # <dir>: a folder the CLI keeps, by name, type and mode; state.json in it, normalized; its databases
  local db
  {
    echo "== files in $1"
    if [ ! -d "$1" ]; then
      echo "(missing)"
    else
      (cd "$1" && find . -printf '%p %y %m\n') | LC_ALL=C sort
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
