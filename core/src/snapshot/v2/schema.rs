pub(super) const APP_ID: i64 = 0x46535632;
pub(super) const COLUMNS: &str = "id,parent,path,sort_key,kind,flags,own_allocated,own_apparent,own_errors,mtime,scanned_at,allocated,apparent,items,errors,min_scanned_at,newest_mtime";
pub(super) const CREATE: &str = "
CREATE TABLE control(singleton INTEGER PRIMARY KEY CHECK(singleton=1), current INTEGER NOT NULL, prev INTEGER, root BLOB NOT NULL, full_scan INTEGER NOT NULL, root_dev INTEGER NOT NULL);
CREATE TABLE identities(id INTEGER PRIMARY KEY, path BLOB NOT NULL UNIQUE);
CREATE TABLE live(
 id INTEGER PRIMARY KEY, parent INTEGER, path BLOB NOT NULL UNIQUE, sort_key BLOB NOT NULL UNIQUE,
 kind INTEGER NOT NULL, flags INTEGER NOT NULL,
 own_allocated INTEGER NOT NULL, own_apparent INTEGER NOT NULL, own_errors INTEGER NOT NULL,
 mtime INTEGER NOT NULL, scanned_at INTEGER NOT NULL,
 allocated INTEGER NOT NULL, apparent INTEGER NOT NULL, items INTEGER NOT NULL, errors INTEGER NOT NULL,
 min_scanned_at INTEGER NOT NULL, newest_mtime INTEGER
);
CREATE INDEX live_child_min ON live(parent,min_scanned_at);
CREATE INDEX live_child_newest ON live(parent,newest_mtime DESC);
CREATE TABLE versions(
 id INTEGER NOT NULL, parent INTEGER, path BLOB NOT NULL, sort_key BLOB NOT NULL,
 kind INTEGER NOT NULL, flags INTEGER NOT NULL,
 own_allocated INTEGER NOT NULL, own_apparent INTEGER NOT NULL, own_errors INTEGER NOT NULL,
 mtime INTEGER NOT NULL, scanned_at INTEGER NOT NULL,
 allocated INTEGER NOT NULL, apparent INTEGER NOT NULL, items INTEGER NOT NULL, errors INTEGER NOT NULL,
 min_scanned_at INTEGER NOT NULL, newest_mtime INTEGER,
 born INTEGER NOT NULL, dead INTEGER,
 PRIMARY KEY(id,born)
);
CREATE UNIQUE INDEX versions_open ON versions(id) WHERE dead IS NULL;
CREATE INDEX versions_path ON versions(path,born);
CREATE INDEX versions_subtree ON versions(sort_key,born);
CREATE INDEX versions_children ON versions(parent,sort_key,born);
CREATE TABLE refreshes(generation INTEGER PRIMARY KEY,path BLOB NOT NULL,scanned_at INTEGER NOT NULL);
";
