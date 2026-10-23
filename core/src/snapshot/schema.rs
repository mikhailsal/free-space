//! Snapshot schema (v1). `id` is the DFS preorder number, so "everything under
//! X" is `id BETWEEN X.id AND X.subtree_end`.

pub const SCHEMA_VERSION: i64 = 1;

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
