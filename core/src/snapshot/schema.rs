//! Snapshot schema (v1). `id` is the DFS preorder number, so "everything under
//! X" is `id BETWEEN X.id AND X.subtree_end`.

pub const SCHEMA_VERSION: i64 = 1;

/// Tables and columns needed for a snapshot to serve as a generation. Checking
/// these with PRAGMA table_info is bounded by the schema size, unlike scanning
/// snapshot rows or running SQLite's full integrity check.
pub const REQUIRED_COLUMNS: &[(&str, &[&str])] = &[
    (
        "meta",
        &[
            "schema_version",
            "generation",
            "prev_generation",
            "scanned_at",
            "host",
            "os",
            "snapshot_id",
            "entries",
            "one_file_system",
            "scan_seconds",
            "freespace_version",
        ],
    ),
    (
        "entries",
        &[
            "id",
            "parent_id",
            "name",
            "path_hash",
            "kind",
            "flags",
            "allocated",
            "apparent",
            "mtime",
            "nlink",
            "items",
            "errors",
            "scanned_at",
            "subtree_end",
            "min_scanned_at",
            "newest_mtime",
        ],
    ),
    (
        "hardlinks",
        &["entry_id", "dev", "ino", "allocated", "apparent"],
    ),
    (
        "roots",
        &[
            "entry_id",
            "path",
            "fs_type",
            "fs_used",
            "fs_total",
            "fs_avail",
            "inodes_used",
        ],
    ),
    (
        "mounts",
        &["entry_id", "path", "fs_type", "fs_used", "fs_total"],
    ),
    ("denied", &["entry_id"]),
    ("refreshes", &["path", "scanned_at", "generation"]),
];

pub const CREATE_TABLES: &str = "
CREATE TABLE meta(
  schema_version INTEGER NOT NULL,
  generation INTEGER NOT NULL,
  prev_generation INTEGER,
  scanned_at INTEGER NOT NULL,
  host TEXT NOT NULL,
  os TEXT NOT NULL,
  snapshot_id TEXT NOT NULL,
  entries INTEGER NOT NULL,
  one_file_system INTEGER NOT NULL,
  scan_seconds REAL,
  freespace_version TEXT NOT NULL
);
CREATE TABLE entries(
  id INTEGER PRIMARY KEY,
  parent_id INTEGER,
  name BLOB NOT NULL,
  path_hash INTEGER NOT NULL,
  kind INTEGER NOT NULL,
  flags INTEGER NOT NULL,
  allocated INTEGER NOT NULL,
  apparent INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  nlink INTEGER NOT NULL,
  items INTEGER NOT NULL,
  errors INTEGER NOT NULL,
  scanned_at INTEGER NOT NULL,
  subtree_end INTEGER NOT NULL,
  min_scanned_at INTEGER NOT NULL,
  newest_mtime INTEGER NOT NULL
);
CREATE TABLE hardlinks(
  entry_id INTEGER PRIMARY KEY,
  dev INTEGER NOT NULL,
  ino INTEGER NOT NULL,
  allocated INTEGER NOT NULL,
  apparent INTEGER NOT NULL
);
CREATE TABLE roots(
  entry_id INTEGER PRIMARY KEY,
  path BLOB NOT NULL,
  fs_type TEXT NOT NULL,
  fs_used INTEGER NOT NULL,
  fs_total INTEGER NOT NULL,
  fs_avail INTEGER NOT NULL,
  inodes_used INTEGER
);
CREATE TABLE mounts(
  entry_id INTEGER PRIMARY KEY,
  path BLOB NOT NULL,
  fs_type TEXT NOT NULL,
  fs_used INTEGER,
  fs_total INTEGER
);
CREATE TABLE denied(entry_id INTEGER PRIMARY KEY);
CREATE TABLE refreshes(
  path BLOB NOT NULL,
  scanned_at INTEGER NOT NULL,
  generation INTEGER NOT NULL
);
";

/// Built after the bulk insert (faster than maintaining them row by row).
pub const CREATE_INDEXES: &[&str] = &[
    "CREATE INDEX entries_parent_alloc ON entries(parent_id, allocated DESC)",
    "CREATE INDEX entries_alloc ON entries(allocated)",
    "CREATE INDEX entries_mtime ON entries(mtime)",
    "CREATE INDEX entries_path_hash ON entries(path_hash)",
    "CREATE INDEX hardlinks_inode ON hardlinks(dev, ino)",
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{CREATE_TABLES, REQUIRED_COLUMNS};

    #[test]
    fn required_columns_match_the_written_schema() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(CREATE_TABLES).unwrap();
        let mut table_stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap();
        let actual_tables: BTreeSet<String> = table_stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let expected_tables: BTreeSet<String> = REQUIRED_COLUMNS
            .iter()
            .map(|(table, _)| (*table).to_owned())
            .collect();
        assert_eq!(actual_tables, expected_tables);
        for (table, required) in REQUIRED_COLUMNS {
            let mut columns = conn
                .prepare(&format!("PRAGMA table_info(\"{table}\")"))
                .unwrap();
            let actual: BTreeSet<String> = columns
                .query_map([], |row| row.get(1))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            let expected: BTreeSet<String> =
                required.iter().map(|name| (*name).to_owned()).collect();
            assert_eq!(actual, expected, "{table}");
        }
    }
}
